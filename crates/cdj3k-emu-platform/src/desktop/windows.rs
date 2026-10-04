//! Windows: the egui window layer, the chassis aspect held in the window's
//! `WM_SIZING`, the panel's strip as its title bar, and Explorer for reveal.
//!
//! winit has no aspect hook on Windows, so the window is subclassed and the
//! rectangle `WM_SIZING` proposes is reshaped ([`super::aspect_fit`]). The
//! parent module's settle snap covers a window that comes up at the wrong
//! shape.
//!
//! While a [`super::CaptionArea`] is set the window is undecorated (winit
//! keeps the caption and sizing styles, so Snap and the system menu stay), and
//! `WM_NCHITTEST` answers its edges as resize borders and the strip's empty
//! stretches as the caption. The strip draws the caption buttons.

use std::os::windows::process::CommandExt;
use std::sync::Mutex;

pub use super::portable::{
    activate_process, enter_panel_window, enter_picker_window, on_creation_context,
    open_file_picker, run_picker, set_app_name,
};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    EnableMenuItem, GetClientRect, GetSystemMenu, GetWindowRect, IsZoomed, PostMessageW,
    SetMenuDefaultItem, TrackPopupMenu, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT,
    HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, MF_BYCOMMAND, MF_ENABLED, MF_GRAYED, SC_CLOSE,
    SC_MAXIMIZE, SC_MINIMIZE, SC_MOVE, SC_RESTORE, SC_SIZE, SM_CXPADDEDBORDER, SM_CYFRAME,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_NCDESTROY, WM_NCHITTEST, WM_NCRBUTTONUP, WM_SIZING,
    WM_SYSCOMMAND,
};

use super::aspect_fit::{fit, Edge, Ratio, Rect};

/// The subclass id: one per window, ours alone.
const SUBCLASS_ID: usize = 0x6364_6a33;

/// The ratio each subclassed window is held to, by `HWND`. `None` is a
/// subclass that lets the window resize freely.
static HELD: Mutex<Vec<(isize, Option<Ratio>)>> = Mutex::new(Vec::new());

fn held_ratio(hwnd: isize) -> Option<Option<Ratio>> {
    let held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    held.iter().find(|&&(h, _)| h == hwnd).map(|&(_, r)| r)
}

fn hold(hwnd: isize, ratio: Option<Ratio>) {
    let current = held_ratio(hwnd);
    if current == Some(ratio) {
        return;
    }
    if current.is_none() {
        // SAFETY: called from the thread that owns the window (the UI
        // thread), with a procedure that lives for the whole process.
        let installed =
            unsafe { SetWindowSubclass(HWND(hwnd as *mut _), Some(subclass_proc), SUBCLASS_ID, 0) };
        if !installed.as_bool() {
            return;
        }
        round_corners(HWND(hwnd as *mut _));
    }
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    held.retain(|&(h, _)| h != hwnd);
    held.push((hwnd, ratio));
}

fn forget(hwnd: isize) {
    HELD.lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|&(h, _)| h != hwnd);
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    _data: usize,
) -> LRESULT {
    if msg == WM_NCRBUTTONUP && wparam.0 == HTCAPTION as usize && caption_in_client() {
        // SAFETY: `hwnd` is the window clicked; `lparam` holds the cursor.
        unsafe { system_menu(hwnd, lparam) };
        return LRESULT(0);
    }
    // SAFETY: the arguments are the ones the window system passed in.
    let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    match msg {
        WM_NCHITTEST if result.0 == HTCLIENT as isize && caption_in_client() => {
            // SAFETY: `hwnd` is the window being hit-tested.
            return LRESULT(unsafe { caption_hit(hwnd, lparam) } as isize);
        }
        WM_SIZING => {
            if let (Some(Some(ratio)), Some(edge)) =
                (held_ratio(hwnd.0 as isize), Edge::from_wmsz(wparam.0))
            {
                // SAFETY: for `WM_SIZING`, `lparam` points at the `RECT` the
                // window is about to take, valid for this call.
                unsafe { reshape(hwnd, &mut *(lparam.0 as *mut RECT), edge, ratio) };
            }
        }
        WM_NCDESTROY => {
            forget(hwnd.0 as isize);
            // SAFETY: removes the subclass this module installed.
            let _ = unsafe { RemoveWindowSubclass(hwnd, Some(subclass_proc), id) };
        }
        _ => {}
    }
    result
}

/// The strip in client pixels: its rectangle and its widgets.
#[derive(Clone, PartialEq)]
struct Caption {
    strip: RECT,
    widgets: Vec<RECT>,
}

/// Set while the panel's strip is the title bar. One emulation window per
/// process.
static CAPTION: Mutex<Option<Caption>> = Mutex::new(None);

fn caption_in_client() -> bool {
    CAPTION.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// The panel draws its own caption buttons in the strip.
pub const CAPTION_IN_STRIP: bool = true;

/// The title bar and the taskbar show the icon the app gives its window.
pub const OWN_WINDOW_ICON: bool = true;

/// Make the strip in `area` the title bar, or give the window its own back
/// with `None`. Call every frame the strip is drawn.
pub fn set_caption_area(ctx: &egui::Context, area: Option<super::CaptionArea>) {
    let to_px = |r: egui::Rect, k: f32| RECT {
        left: (r.left() * k).floor() as i32,
        top: (r.top() * k).floor() as i32,
        right: (r.right() * k).ceil() as i32,
        bottom: (r.bottom() * k).ceil() as i32,
    };
    let next = area.map(|a| Caption {
        strip: to_px(a.strip, a.pixels_per_point),
        widgets: a
            .widgets
            .iter()
            .map(|&w| to_px(w, a.pixels_per_point))
            .collect(),
    });
    let mut current = CAPTION.lock().unwrap_or_else(|e| e.into_inner());
    if current.is_some() != next.is_some() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(next.is_none()));
    }
    *current = next;
}

/// Windows 11's rounded corners, which an undecorated window does not get by
/// default. DWM keeps a maximised or snapped window square; Windows 10 has
/// no such attribute and refuses it.
fn round_corners(hwnd: HWND) {
    let pref = DWMWCP_ROUND;
    // SAFETY: `pref` is a live DWM_WINDOW_CORNER_PREFERENCE of the size given.
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const pref).cast(),
            std::mem::size_of_val(&pref) as u32,
        )
    };
}

/// The window menu a right click on the caption opens, which an undecorated
/// window does not get from the window system.
unsafe fn system_menu(hwnd: HWND, lparam: LPARAM) {
    let (x, y) = point_of(lparam);
    // SAFETY: `hwnd` is a live window; the menu is its own system menu.
    unsafe {
        let menu = GetSystemMenu(hwnd, false);
        if menu.is_invalid() {
            return;
        }
        let zoomed = IsZoomed(hwnd).as_bool();
        for (cmd, on) in [
            (SC_RESTORE, zoomed),
            (SC_MOVE, !zoomed),
            (SC_SIZE, !zoomed),
            (SC_MINIMIZE, true),
            (SC_MAXIMIZE, !zoomed),
            (SC_CLOSE, true),
        ] {
            let state = if on { MF_ENABLED } else { MF_GRAYED };
            let _ = EnableMenuItem(menu, cmd, MF_BYCOMMAND | state);
        }
        let _ = SetMenuDefaultItem(menu, SC_CLOSE, 0);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            x,
            y,
            Some(0),
            hwnd,
            None,
        );
        if cmd.0 != 0 {
            let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(cmd.0 as usize), LPARAM(0));
        }
    }
}

/// The screen point a mouse message carries in `lparam`.
fn point_of(lparam: LPARAM) -> (i32, i32) {
    (
        (lparam.0 & 0xFFFF) as u16 as i16 as i32,
        ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32,
    )
}

/// The resize frame's thickness at the window's DPI.
fn frame_thickness(hwnd: HWND) -> i32 {
    // SAFETY: plain queries on a live window.
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
    }
}

/// What a point in the undecorated window is, with the strip as the caption:
/// a resize border at its edges, the caption on the strip's empty stretches,
/// or the client.
unsafe fn caption_hit(hwnd: HWND, lparam: LPARAM) -> u32 {
    let (x, y) = point_of(lparam);
    let mut pt = POINT { x, y };
    let mut client = RECT::default();
    // SAFETY: `pt` and `client` are locals; `hwnd` is a live window.
    let ok = unsafe {
        ScreenToClient(hwnd, &mut pt).as_bool() && GetClientRect(hwnd, &mut client).is_ok()
    };
    if !ok {
        return HTCLIENT;
    }
    // SAFETY: as above.
    let zoomed = unsafe { IsZoomed(hwnd) }.as_bool();
    if !zoomed {
        let edge = frame_thickness(hwnd);
        let corner = edge * 2;
        let (w, h) = (client.right, client.bottom);
        let left = pt.x < edge;
        let right = pt.x >= w - edge;
        let top = pt.y < edge;
        let bottom = pt.y >= h - edge;
        let near_left = pt.x < corner;
        let near_right = pt.x >= w - corner;
        let near_top = pt.y < corner;
        let near_bottom = pt.y >= h - corner;
        let hit = if (top && near_left) || (left && near_top) {
            Some(HTTOPLEFT)
        } else if (top && near_right) || (right && near_top) {
            Some(HTTOPRIGHT)
        } else if (bottom && near_left) || (left && near_bottom) {
            Some(HTBOTTOMLEFT)
        } else if (bottom && near_right) || (right && near_bottom) {
            Some(HTBOTTOMRIGHT)
        } else if top {
            Some(HTTOP)
        } else if bottom {
            Some(HTBOTTOM)
        } else if left {
            Some(HTLEFT)
        } else if right {
            Some(HTRIGHT)
        } else {
            None
        };
        if let Some(hit) = hit {
            return hit;
        }
    }
    let inside = |r: &RECT| pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom;
    let caption = CAPTION.lock().unwrap_or_else(|e| e.into_inner());
    match caption.as_ref() {
        Some(c) if inside(&c.strip) && !c.widgets.iter().any(inside) => HTCAPTION,
        _ => HTCLIENT,
    }
}

/// Edit the proposed outer rectangle in place.
unsafe fn reshape(hwnd: HWND, proposed: &mut RECT, edge: Edge, ratio: Ratio) {
    let (mut outer, mut inner) = (RECT::default(), RECT::default());
    // SAFETY: both rectangles are locals.
    let read = unsafe { GetWindowRect(hwnd, &mut outer).and(GetClientRect(hwnd, &mut inner)) };
    if read.is_err() {
        return;
    }
    let chrome = (
        (outer.right - outer.left) - (inner.right - inner.left),
        (outer.bottom - outer.top) - (inner.bottom - inner.top),
    );
    let fitted = fit(
        edge,
        Rect {
            left: proposed.left,
            top: proposed.top,
            right: proposed.right,
            bottom: proposed.bottom,
        },
        chrome,
        ratio,
    );
    *proposed = RECT {
        left: fitted.left,
        top: fitted.top,
        right: fitted.right,
        bottom: fitted.bottom,
    };
}

/// Hold the window to `ref_canvas`'s aspect, or release it with `None`.
pub fn apply_resize_constraints(
    ctx: &egui::Context,
    frame: &eframe::Frame,
    state: &mut super::ResizeState,
    ref_canvas: Option<(f32, f32)>,
) {
    if let Ok(RawWindowHandle::Win32(h)) = frame.window_handle().map(|h| h.as_raw()) {
        let ratio = ref_canvas
            .filter(|&(w, h)| w > 0.0 && h > 0.0)
            .map(|(w, h)| Ratio {
                width: (w * 16.0).round() as u32,
                height: (h * 16.0).round() as u32,
                base_height: (crate::menu::in_window_bar_height() * ctx.pixels_per_point()).round()
                    as u32,
            });
        hold(h.hwnd.get(), ratio);
    }
    if let Some(canvas) = ref_canvas {
        super::snap_when_settled(ctx, state, canvas);
        super::frame_store::observe(ctx);
    }
}

/// Show a file in Explorer, selected.
pub fn reveal_in_file_manager(path: &std::path::Path) {
    // `/select,` takes its path quoted within the same argument.
    let _ = std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", path.display()))
        .spawn();
}

/// Open a web page in the default browser.
pub fn open_url(url: &str) {
    let url = windows::core::HSTRING::from(url);
    // SAFETY: both strings outlive the call.
    unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(
            None,
            w!("open"),
            &url,
            None,
            None,
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        );
    }
}

/// Whether setup opens as a second window centred over the first. Windows
/// keeps it in the one window.
pub const PLACES_WINDOWS: bool = false;

/// No swap interval: with one, NVIDIA's OpenGL presentation over a DXGI swap
/// chain drops frames. DWM composes without tearing; frames come as the app
/// repaints.
pub const VSYNC: bool = false;

/// Create the named mutex the installer and uninstaller check (Inno Setup's
/// `AppMutex`), so neither runs while the app holds files open. Held until the
/// process exits.
pub fn announce_running() {
    // SAFETY: no security attributes, a static NUL-terminated name. The
    // handle is never closed.
    let _ = unsafe { CreateMutexW(None, false, w!("Global\\cdj3k-emu")) };
}
