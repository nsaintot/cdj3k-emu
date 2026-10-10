//! AppKit: the window shell, the process's Dock presence, and the file panel.
//!
//! Aspect-ratio resize is enforced with `NSWindow` methods, not in egui.

use super::{panel_initial_size, PICKER_SIZE};
use cdj3k_emu_panel::Model;

fn ns_window_for_handle(
    handle: &impl raw_window_handle::HasWindowHandle,
) -> Result<objc2::rc::Retained<objc2_app_kit::NSWindow>, String> {
    use objc2::rc::Retained;
    use objc2_app_kit::NSView;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let raw = HasWindowHandle::window_handle(handle)
        .map_err(|e| format!("window handle: {e}"))?
        .as_raw();

    let RawWindowHandle::AppKit(appkit) = raw else {
        return Err("expected AppKit window handle".into());
    };

    let view_ptr = appkit.ns_view.as_ptr().cast::<NSView>();
    let mut current = unsafe { Retained::retain(view_ptr) }.ok_or("nil NSView")?;

    // Safety cap so a malformed/cyclic NSView graph cannot wedge us forever.
    const MAX_SUPERVIEW_DEPTH: usize = 32;
    for _ in 0..MAX_SUPERVIEW_DEPTH {
        if let Some(w) = current.window() {
            return Ok(w);
        }
        let next = unsafe { current.superview() };
        match next {
            Some(s) => current = s,
            None => return Err("NSView not in a window hierarchy".into()),
        }
    }
    Err("NSView hierarchy too deep".into())
}

/// Enable AppKit's built-in window-frame persistence for this window.
///
/// `setFrameAutosaveName:` makes AppKit transparently save the window's
/// position+size to `NSUserDefaults` whenever the user moves/resizes it, and
/// `setFrameUsingName:` applies any previously-saved frame for that name.
/// Off-screen-recovery (e.g. a monitor was unplugged) is handled by AppKit.
/// Use a per-instance name so multiple emulator instances keep separate frames.
/// Returns whether a previously saved frame was applied.
fn set_window_autosave_name(
    handle: &impl raw_window_handle::HasWindowHandle,
    name: &str,
) -> Result<bool, String> {
    use objc2_foundation::{MainThreadMarker, NSString};

    MainThreadMarker::new().ok_or("AppKit: not on main thread")?;

    let window = ns_window_for_handle(handle)?;
    let ns_name = NSString::from_str(name);
    let restored: bool = unsafe {
        // Restore first (if a saved frame exists), then enable autosave going forward.
        let restored: bool = objc2::msg_send![&*window, setFrameUsingName: &*ns_name];
        let _: bool = objc2::msg_send![&*window, setFrameAutosaveName: &*ns_name];
        restored
    };
    Ok(restored)
}

/// Stop AppKit frame persistence for this window (an empty autosave name).
/// The frame saved so far stays in `NSUserDefaults` for the next
/// [`set_window_autosave_name`].
fn clear_window_autosave_name(
    handle: &impl raw_window_handle::HasWindowHandle,
) -> Result<(), String> {
    use objc2_foundation::{MainThreadMarker, NSString};

    MainThreadMarker::new().ok_or("AppKit: not on main thread")?;

    let window = ns_window_for_handle(handle)?;
    let empty = NSString::from_str("");
    unsafe {
        let _: bool = objc2::msg_send![&*window, setFrameAutosaveName: &*empty];
    }
    Ok(())
}

/// Toggle `NSWindowStyleMaskResizable` (user resizing via the window edges).
fn set_window_resizable(
    handle: &impl raw_window_handle::HasWindowHandle,
    resizable: bool,
) -> Result<(), String> {
    use objc2_foundation::MainThreadMarker;

    MainThreadMarker::new().ok_or("AppKit: not on main thread")?;

    const NS_WINDOW_STYLE_MASK_RESIZABLE: usize = 1 << 3;
    let window = ns_window_for_handle(handle)?;
    unsafe {
        let mask: usize = objc2::msg_send![&*window, styleMask];
        let mask = if resizable {
            mask | NS_WINDOW_STYLE_MASK_RESIZABLE
        } else {
            mask & !NS_WINDOW_STYLE_MASK_RESIZABLE
        };
        let _: () = objc2::msg_send![&*window, setStyleMask: mask];
    }
    Ok(())
}

/// Resize the content area to `w` x `h` points, keeping the window centre
/// where it is (the frame grows/shrinks around it).
fn set_window_content_size_centered(
    handle: &impl raw_window_handle::HasWindowHandle,
    w: f64,
    h: f64,
) -> Result<(), String> {
    use objc2_foundation::{MainThreadMarker, NSRect, NSSize};

    MainThreadMarker::new().ok_or("AppKit: not on main thread")?;

    let window = ns_window_for_handle(handle)?;
    unsafe {
        let frame: NSRect = objc2::msg_send![&*window, frame];
        let cx = frame.origin.x + frame.size.width * 0.5;
        let cy = frame.origin.y + frame.size.height * 0.5;
        let content: NSRect = objc2::msg_send![&*window, contentRectForFrameRect: frame];
        let wanted = NSRect::new(content.origin, NSSize::new(w, h));
        let mut new_frame: NSRect = objc2::msg_send![&*window, frameRectForContentRect: wanted];
        new_frame.origin.x = cx - new_frame.size.width * 0.5;
        new_frame.origin.y = cy - new_frame.size.height * 0.5;
        let _: () = objc2::msg_send![&*window, setFrame: new_frame, display: true];
    }
    Ok(())
}

/// Disable macOS native window tabbing for this window.
/// Without this, AppKit adds "Show Tab Bar" / "Show All Tabs" to the View menu automatically.
fn disable_window_tabbing(handle: &impl raw_window_handle::HasWindowHandle) -> Result<(), String> {
    use objc2_foundation::MainThreadMarker;

    MainThreadMarker::new().ok_or("AppKit: not on main thread")?;

    let window = ns_window_for_handle(handle)?;
    // NSWindowTabbingModeDisallowed = 2
    unsafe {
        let _: () = objc2::msg_send![&*window, setTabbingMode: 2usize];
    }
    Ok(())
}

/// App-wide kill switch for the system "Show Tab Bar" / "Show All Tabs" View
/// menu entries. Per-window `setTabbingMode:` only suppresses tabbing on the
/// main eframe window; any extra `NSWindow` AppKit creates (e.g. the deferred
/// debug viewport) still opts into tabbing and re-introduces those menu items.
/// Setting the class property to `NO` covers every current and future window.
fn disable_automatic_window_tabbing_global() -> Result<(), String> {
    use objc2::runtime::AnyClass;
    use objc2_foundation::MainThreadMarker;

    MainThreadMarker::new().ok_or("AppKit: not on main thread")?;

    let cls = AnyClass::get("NSWindow").ok_or("NSWindow class not found")?;
    unsafe {
        let _: () = objc2::msg_send![cls, setAllowsAutomaticWindowTabbing: false];
    }
    Ok(())
}

/// Drop any runtime-installed dock icon override so the Dock falls back to
/// the bundle's `CFBundleIconFile` (`cdj3k-emu.icns` in `Contents/Resources/`).
///
/// `eframe` installs a default egui-logo placeholder via
/// `NSApplication.applicationIconImage` during window creation; without this
/// reset, that placeholder shadows the bundle icon for the lifetime of the
/// process - visible as the bundle icon briefly flashing as the override is
/// torn down on quit.  Passing nil to `setApplicationIconImage:` is the
/// AppKit-blessed way to revert to the Info.plist-declared icon.
fn reset_dock_icon_to_bundle() -> Result<(), String> {
    use objc2_app_kit::NSApplication;
    use objc2_foundation::MainThreadMarker;

    let mtm = MainThreadMarker::new().ok_or("AppKit: not on main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    unsafe {
        let _: () = objc2::msg_send![&app, setApplicationIconImage: std::ptr::null::<objc2::runtime::AnyObject>()];
    }
    Ok(())
}

fn set_window_aspect_constraints(
    handle: &impl raw_window_handle::HasWindowHandle,
    aspect_w: f64,
    aspect_h: f64,
) -> Result<(), String> {
    use objc2_foundation::{MainThreadMarker, NSSize};

    MainThreadMarker::new().ok_or("AppKit: not on main thread")?;

    let window = ns_window_for_handle(handle)?;
    // NOTE: do NOT also call `setAspectRatio` here - that locks the whole
    // window frame (incl. title bar) to the ratio, while we want the *content
    // area* locked. Setting both leaves AppKit arbitrating between two
    // mutually-incompatible constraints (differ by the title bar height),
    // which manifests as a snap/fight after each resize.
    let s = NSSize::new(aspect_w, aspect_h);
    unsafe {
        window.setContentAspectRatio(s);
    }
    Ok(())
}

/// Bring the process `pid` - another instance of the app - to the front.
pub fn activate_process(pid: u32) -> Result<(), String> {
    use objc2::runtime::{AnyClass, AnyObject, Bool};

    let cls = AnyClass::get("NSRunningApplication").ok_or("NSRunningApplication not found")?;
    unsafe {
        let app: *mut AnyObject =
            objc2::msg_send![cls, runningApplicationWithProcessIdentifier: pid as libc::pid_t];
        if app.is_null() {
            return Err(format!("no running application with pid {pid}"));
        }
        // NSApplicationActivateAllWindows | NSApplicationActivateIgnoringOtherApps
        let ok: Bool = objc2::msg_send![app, activateWithOptions: 3usize];
        if !ok.as_bool() {
            return Err(format!("pid {pid} refused activation"));
        }
    }
    Ok(())
}

/// Override the Dock tile / menu-bar / Activity Monitor name for this process.
///
/// The Dock and the application menu read `CFBundleName` from
/// `[NSBundle mainBundle].infoDictionary` at launch. The returned dictionary
/// is documented as immutable, but the backing store is a mutable
/// `CFDictionary`, so an `NSMutableDictionary`-typed `setObject:forKey:`
/// message lands in the real storage and is picked up by both surfaces.
/// We also update `NSProcessInfo.processName` so `ps`, `top`, and Activity
/// Monitor agree.
///
/// Must be called before `eframe::run_native` — once `NSApplication` finishes
/// launching, the menu-bar title is cached and won't refresh.
pub fn set_app_name(name: &str) -> Result<(), String> {
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::{MainThreadMarker, NSString};

    MainThreadMarker::new().ok_or("AppKit: not on main thread")?;

    let ns_name = NSString::from_str(name);
    let key = NSString::from_str("CFBundleName");

    let bundle_cls = AnyClass::get("NSBundle").ok_or("NSBundle class not found")?;
    let proc_cls = AnyClass::get("NSProcessInfo").ok_or("NSProcessInfo class not found")?;

    unsafe {
        let bundle: *mut AnyObject = objc2::msg_send![bundle_cls, mainBundle];
        if bundle.is_null() {
            return Err("nil mainBundle".into());
        }
        let info: *mut AnyObject = objc2::msg_send![bundle, infoDictionary];
        if info.is_null() {
            return Err("nil infoDictionary".into());
        }
        let _: () = objc2::msg_send![info, setObject: &*ns_name, forKey: &*key];

        let proc_info: *mut AnyObject = objc2::msg_send![proc_cls, processInfo];
        if !proc_info.is_null() {
            let _: () = objc2::msg_send![proc_info, setProcessName: &*ns_name];
        }
    }
    Ok(())
}

/// Best-effort from [`eframe::CreationContext`] (view may not be in a window yet).
pub fn on_creation_context(cc: &eframe::CreationContext<'_>) {
    let _ = disable_window_tabbing(cc);
    // Class-level kill switch: prevents the deferred debug viewport (and
    // any other NSWindow AppKit spawns) from re-adding "Show Tab Bar" /
    // "Show All Tabs" to the View menu.
    let _ = disable_automatic_window_tabbing_global();
    // Drop eframe's default placeholder icon so the Dock reads our
    // bundle's CFBundleIconFile instead.
    let _ = reset_dock_icon_to_bundle();
}

/// Turn the main window into the panel of `model`: user-resizable, restored
/// to slot `instance_id`'s last saved panel frame for that model (or sized to
/// the default and kept centred where the picker was), then aspect-locked to
/// `ref_canvas`.
pub fn enter_panel_window(
    ctx: &egui::Context,
    frame: &eframe::Frame,
    instance_id: u32,
    model: Model,
    ref_canvas: (f32, f32),
) {
    let _ = set_window_resizable(frame, true);
    let restored =
        set_window_autosave_name(frame, &super::frame_key(instance_id, model)).unwrap_or(false);
    if !restored {
        let [w, h] = panel_initial_size(ref_canvas);
        let _ = set_window_content_size_centered(frame, w as f64, h as f64);
    }
    let _ = set_window_aspect_constraints(frame, ref_canvas.0 as f64, ref_canvas.1 as f64);
    let _ = ctx;
}

/// Turn the main window back into the compact picker: stop frame autosave (so
/// the picker size is never recorded as the panel frame), fixed size, centred
/// where the panel was.
pub fn enter_picker_window(ctx: &egui::Context, frame: &eframe::Frame) {
    let _ = clear_window_autosave_name(frame);
    let _ = set_window_content_size_centered(frame, PICKER_SIZE[0] as f64, PICKER_SIZE[1] as f64);
    let _ = set_window_resizable(frame, false);
    let _ = ctx;
}

/// Re-apply AppKit aspect constraints every frame so nothing in the stack
/// resets them during resize. `None` while the picker is up (fixed size).
pub fn apply_resize_constraints(
    ctx: &egui::Context,
    frame: &eframe::Frame,
    state: &mut super::ResizeState,
    ref_canvas: Option<(f32, f32)>,
) {
    if let Some((w, h)) = ref_canvas {
        let _ = set_window_aspect_constraints(frame, w as f64, h as f64);
        super::snap_when_settled(ctx, state, (w, h));
    }
}

/// Run the picker on the calling thread, which is the UI thread:
/// `NSOpenPanel` exists only there, and its modal is AppKit's own.
pub(super) fn run_picker(
    title: &str,
    allowed_types: &[&str],
    reply: std::sync::mpsc::Sender<Option<std::path::PathBuf>>,
) {
    let _ = reply.send(open_file_picker(title, allowed_types));
}

/// [`run_picker`] for a mod: one panel that accepts an archive or a folder.
/// It has no type filter, because a filter would also grey out folders; the
/// caller checks the path it gets.
pub(super) fn run_mod_picker(
    title: &str,
    _allowed_types: &[&str],
    reply: std::sync::mpsc::Sender<Option<std::path::PathBuf>>,
) {
    let _ = reply.send(open_panel(title, &[], true));
}

/// Open a native file-open dialog and return the chosen path, or `None` if cancelled.
/// `title` is the panel's message text; `allowed_types` filters by UTType identifier
/// (e.g. `&["public.data"]` for any file).  Pass an empty slice for no filter.
pub fn open_file_picker(title: &str, allowed_types: &[&str]) -> Option<std::path::PathBuf> {
    open_panel(title, allowed_types, false)
}

/// `NSOpenPanel` for one file; with `folders`, a folder can be chosen too.
fn open_panel(title: &str, allowed_types: &[&str], folders: bool) -> Option<std::path::PathBuf> {
    use objc2::msg_send_id;
    use objc2::rc::Retained;
    use objc2_app_kit::NSOpenPanel;
    use objc2_foundation::{MainThreadMarker, NSString};

    let mtm = MainThreadMarker::new()?;
    let panel = unsafe { NSOpenPanel::openPanel(mtm) };
    unsafe {
        panel.setMessage(Some(&NSString::from_str(title)));
        panel.setCanChooseFiles(true);
        panel.setCanChooseDirectories(folders);
        panel.setAllowsOtherFileTypes(false);
        panel.setAllowsMultipleSelection(false);
        if !allowed_types.is_empty() {
            // NSOpenPanel.allowedFileTypes is deprecated in 12+ but still functional.
            let types: Vec<Retained<NSString>> = allowed_types
                .iter()
                .map(|t| NSString::from_str(t))
                .collect();
            let arr = objc2_foundation::NSArray::from_id_slice(&types);
            let _: () = objc2::msg_send![&panel, setAllowedFileTypes: &*arr];
        }
    }
    let response: isize = unsafe { objc2::msg_send![&panel, runModal] };
    if response != 1 {
        return None; // NSModalResponseOK = 1
    }
    let url: Option<Retained<objc2_foundation::NSURL>> = unsafe { msg_send_id![&panel, URL] };
    let path_ns: Option<Retained<NSString>> =
        unsafe { url.as_deref().and_then(|u| msg_send_id![u, path]) };
    path_ns.map(|s| std::path::PathBuf::from(s.to_string()))
}

/// Show a file in the host's file manager, selected rather than opened.
pub fn reveal_in_file_manager(path: &std::path::Path) {
    let _ = std::process::Command::new("/usr/bin/open")
        .arg("-R")
        .arg(path)
        .spawn();
}

/// Open a web page in the default browser.
pub fn open_url(url: &str) {
    let _ = std::process::Command::new("/usr/bin/open").arg(url).spawn();
}

/// Whether a second window can be centred over the first.
pub const PLACES_WINDOWS: bool = true;

/// Present on the display's refresh.
pub const VSYNC: bool = true;

/// Nothing to do: no installer here looks for a running copy.
pub fn announce_running() {}

/// The window manager draws the title bar; the strip sits under it.
pub const CAPTION_IN_STRIP: bool = false;

/// The Dock takes the icon from the bundle's `CFBundleIconFile`.
pub const OWN_WINDOW_ICON: bool = false;

pub fn set_caption_area(_ctx: &egui::Context, _area: Option<super::CaptionArea>) {}
