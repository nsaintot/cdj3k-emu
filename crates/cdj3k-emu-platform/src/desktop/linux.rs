//! Linux: the egui window layer, the chassis aspect held by the window
//! system, and the file manager over D-Bus.
//!
//! The ratio is enforced where the window system settles a window's size, so
//! no frame is drawn at the wrong shape: in winit's Wayland configure handler
//! (`set_surface_aspect_ratio`, from winit/patches), or by the X11 window
//! manager through `WM_NORMAL_HINTS`. The settle snap in the parent module
//! stays behind it, for a window that comes up at the wrong shape.

use std::sync::Mutex;

pub use super::portable::{
    activate_process, enter_panel_window, enter_picker_window, on_creation_context,
    open_file_picker, run_picker, set_app_name,
};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// The chassis ratio, with the in-window menu strip as a base height outside
/// it. Kept in integers at 16× so a fractional reference canvas survives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ratio {
    width: u32,
    height: u32,
    base_height: u32,
}

impl Ratio {
    fn of(ref_canvas: Option<(f32, f32)>) -> Option<Self> {
        let (w, h) = ref_canvas.filter(|&(w, h)| w > 0.0 && h > 0.0)?;
        Some(Self {
            width: (w * 16.0).round() as u32,
            height: (h * 16.0).round() as u32,
            base_height: crate::menu::in_window_bar_height().round() as u32,
        })
    }
}

/// Which ratio each window was last given, so the window system is only told
/// when it changes. Keyed by the raw window, not held by the caller: the
/// picker and the panel both set it, and they must agree on what is in force.
static APPLIED: Mutex<Vec<(u64, Option<Ratio>)>> = Mutex::new(Vec::new());

fn already_applied(key: u64, ratio: Option<Ratio>) -> bool {
    let applied = APPLIED.lock().unwrap_or_else(|e| e.into_inner());
    applied.iter().any(|&(k, r)| k == key && r == ratio)
}

fn mark_applied(key: u64, ratio: Option<Ratio>) {
    let mut applied = APPLIED.lock().unwrap_or_else(|e| e.into_inner());
    applied.retain(|&(k, _)| k != key);
    applied.push((key, ratio));
}

/// Hold the window to `ref_canvas`'s aspect, or release it with `None`.
pub fn apply_resize_constraints(
    ctx: &egui::Context,
    frame: &eframe::Frame,
    state: &mut super::ResizeState,
    ref_canvas: Option<(f32, f32)>,
) {
    let ratio = Ratio::of(ref_canvas);
    match frame.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Wayland(h)) => wayland::hold(h.surface, ratio),
        Ok(RawWindowHandle::Xlib(h)) => x11::hold(h.window as u32, ratio),
        Ok(RawWindowHandle::Xcb(h)) => x11::hold(h.window.get(), ratio),
        _ => {}
    }
    if let Some(canvas) = ref_canvas {
        super::snap_when_settled(ctx, state, canvas);
        super::frame_store::observe(ctx);
    }
}

/// Show a file in the host's file manager, selected rather than opened.
///
/// Uses `org.freedesktop.FileManager1.ShowItems` over D-Bus (Nautilus,
/// Dolphin, Thunar, Nemo), since `xdg-open` cannot select an item. If the
/// call fails, the containing directory is opened instead.
pub fn reveal_in_file_manager(path: &std::path::Path) {
    let uri = format!("file://{}", path.display());
    let shown = std::process::Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.FileManager1",
            "--object-path",
            "/org/freedesktop/FileManager1",
            "--method",
            "org.freedesktop.FileManager1.ShowItems",
            &format!("['{uri}']"),
            "",
        ])
        .status()
        .is_ok_and(|s| s.success());

    if !shown {
        if let Some(dir) = path.parent() {
            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
        }
    }
}

mod wayland {
    use std::ffi::c_void;
    use std::ptr::NonNull;

    use super::{already_applied, mark_applied, Ratio};

    pub(super) fn hold(surface: NonNull<c_void>, ratio: Option<Ratio>) {
        let key = surface.as_ptr() as u64;
        if already_applied(key, ratio) {
            return;
        }
        winit::platform::wayland::set_surface_aspect_ratio(
            surface,
            ratio.map(|r| winit::platform::wayland::AspectRatio {
                width: r.width,
                height: r.height,
                base_width: 0,
                base_height: r.base_height,
            }),
        );
        mark_applied(key, ratio);
    }
}

mod x11 {
    use std::sync::OnceLock;

    use x11rb::connection::Connection;
    use x11rb::properties::{AspectRatio, WmSizeHints};
    use x11rb::rust_connection::RustConnection;

    use super::{already_applied, mark_applied, Ratio};

    /// A connection of our own: winit's is not reachable through eframe. The
    /// hints are a property on the window, so whose connection sets them does
    /// not matter.
    fn conn() -> Option<&'static RustConnection> {
        static CONN: OnceLock<Option<RustConnection>> = OnceLock::new();
        CONN.get_or_init(|| RustConnection::connect(None).ok().map(|(c, _)| c))
            .as_ref()
    }

    /// Minimum and maximum aspect set equal, with the strip as the base size:
    /// ICCCM applies the aspect to the size minus the base. The existing hints
    /// are read first and kept, since winit writes the min and max sizes into
    /// the same property.
    pub(super) fn hold(window: u32, ratio: Option<Ratio>) {
        let key = u64::from(window);
        if already_applied(key, ratio) {
            return;
        }
        let Some(conn) = conn() else {
            return;
        };
        let set = (|| -> Option<()> {
            let mut hints = WmSizeHints::get_normal_hints(conn, window)
                .ok()?
                .reply()
                .ok()?
                .unwrap_or_else(WmSizeHints::new);
            match ratio {
                Some(r) => {
                    let aspect = AspectRatio::new(r.width as i32, r.height as i32);
                    hints.aspect = Some((aspect, aspect));
                    hints.base_size = Some((0, r.base_height as i32));
                }
                None => {
                    hints.aspect = None;
                    hints.base_size = None;
                }
            }
            hints.set_normal_hints(conn, window).ok()?;
            conn.flush().ok()
        })()
        .is_some();
        if set {
            mark_applied(key, ratio);
        }
    }
}

/// Whether a second window can be centred over the first: a Wayland client
/// cannot place one.
pub const PLACES_WINDOWS: bool = false;

/// Present on the display's refresh.
pub const VSYNC: bool = true;

/// Nothing to do: no installer here looks for a running copy.
pub fn announce_running() {}

/// The window manager draws the title bar; the strip sits under it.
pub const CAPTION_IN_STRIP: bool = false;

/// X11 shows the icon the app gives its window; Wayland takes the desktop
/// entry's.
pub const OWN_WINDOW_ICON: bool = true;

pub fn set_caption_area(_ctx: &egui::Context, _area: Option<super::CaptionArea>) {}

#[cfg(test)]
mod tests {
    use winit::dpi::LogicalSize;
    use winit::platform::wayland::AspectRatio;

    const MIN: LogicalSize<u32> = LogicalSize::new(100, 100);

    /// A 4:3 chassis under a 44 px strip, the shape the panel has.
    const PANEL: AspectRatio = AspectRatio {
        width: 4,
        height: 3,
        base_width: 0,
        base_height: 44,
    };

    fn fit(w: u32, h: u32) -> LogicalSize<u32> {
        PANEL.fit(LogicalSize::new(w, h), MIN)
    }

    /// Too wide: the width comes down to what the height allows.
    #[test]
    fn too_wide_gives_up_width() {
        assert_eq!(fit(1000, 344), LogicalSize::new(400, 344));
    }

    /// Too tall: the height comes down, and the strip stays on top of it.
    #[test]
    fn too_tall_gives_up_height() {
        assert_eq!(fit(400, 900), LogicalSize::new(400, 344));
    }

    /// Whatever the compositor offers, the answer fits inside it.
    #[test]
    fn it_never_enlarges_either_axis() {
        for w in (120..2000).step_by(37) {
            for h in (120..1500).step_by(41) {
                let got = fit(w, h);
                assert!(got.width <= w && got.height <= h, "{w}x{h} -> {got:?}");
            }
        }
    }

    /// A fitted size fits as itself: refitting it on every configure must not
    /// shave a pixel off each time.
    #[test]
    fn fitting_twice_changes_nothing() {
        for w in (120..2000).step_by(29) {
            for h in (120..1500).step_by(31) {
                let once = fit(w, h);
                assert_eq!(fit(once.width, once.height), once, "{w}x{h}");
            }
        }
    }

    /// Nothing to fit into, or nothing to fit: the offer stands.
    #[test]
    fn degenerate_input_is_left_alone() {
        assert_eq!(fit(500, 44), LogicalSize::new(500, 44));
        let none = AspectRatio { width: 0, ..PANEL };
        assert_eq!(
            none.fit(LogicalSize::new(640, 480), MIN),
            LogicalSize::new(640, 480)
        );
    }
}
