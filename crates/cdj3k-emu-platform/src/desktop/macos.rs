//! macOS: AppKit hooks - window resize constraints, file picker.

// ── Aspect-ratio constraint (shared on/off-macOS stub) ───────────────────────

#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
pub fn set_window_autosave_name(
    handle: &impl raw_window_handle::HasWindowHandle,
    name: &str,
) -> Result<(), String> {
    use objc2_foundation::{MainThreadMarker, NSString};

    MainThreadMarker::new().ok_or("AppKit: not on main thread")?;

    let window = ns_window_for_handle(handle)?;
    let ns_name = NSString::from_str(name);
    unsafe {
        // Restore first (if a saved frame exists), then enable autosave going forward.
        let _: bool = objc2::msg_send![&*window, setFrameUsingName: &*ns_name];
        let _: bool = objc2::msg_send![&*window, setFrameAutosaveName: &*ns_name];
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn set_window_autosave_name(
    _handle: &impl raw_window_handle::HasWindowHandle,
    _name: &str,
) -> Result<(), String> {
    Ok(())
}

/// Disable macOS native window tabbing for this window.
/// Without this, AppKit adds "Show Tab Bar" / "Show All Tabs" to the View menu automatically.
#[cfg(target_os = "macos")]
pub fn disable_window_tabbing(
    handle: &impl raw_window_handle::HasWindowHandle,
) -> Result<(), String> {
    use objc2_foundation::MainThreadMarker;

    MainThreadMarker::new().ok_or("AppKit: not on main thread")?;

    let window = ns_window_for_handle(handle)?;
    // NSWindowTabbingModeDisallowed = 2
    unsafe {
        let _: () = objc2::msg_send![&*window, setTabbingMode: 2usize];
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn disable_window_tabbing(
    _handle: &impl raw_window_handle::HasWindowHandle,
) -> Result<(), String> {
    Ok(())
}

/// App-wide kill switch for the system "Show Tab Bar" / "Show All Tabs" View
/// menu entries. Per-window `setTabbingMode:` only suppresses tabbing on the
/// main eframe window; any extra `NSWindow` AppKit creates (e.g. the deferred
/// debug viewport) still opts into tabbing and re-introduces those menu items.
/// Setting the class property to `NO` covers every current and future window.
#[cfg(target_os = "macos")]
pub fn disable_automatic_window_tabbing_global() -> Result<(), String> {
    use objc2::runtime::AnyClass;
    use objc2_foundation::MainThreadMarker;

    MainThreadMarker::new().ok_or("AppKit: not on main thread")?;

    let cls = AnyClass::get("NSWindow").ok_or("NSWindow class not found")?;
    unsafe {
        let _: () = objc2::msg_send![cls, setAllowsAutomaticWindowTabbing: false];
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn disable_automatic_window_tabbing_global() -> Result<(), String> {
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
#[cfg(target_os = "macos")]
pub fn reset_dock_icon_to_bundle() -> Result<(), String> {
    use objc2_app_kit::NSApplication;
    use objc2_foundation::MainThreadMarker;

    let mtm = MainThreadMarker::new().ok_or("AppKit: not on main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    unsafe {
        let _: () = objc2::msg_send![&app, setApplicationIconImage: std::ptr::null::<objc2::runtime::AnyObject>()];
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn reset_dock_icon_to_bundle() -> Result<(), String> {
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
#[cfg(target_os = "macos")]
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

#[cfg(not(target_os = "macos"))]
pub fn set_app_name(_name: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn set_window_aspect_constraints(
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

#[cfg(not(target_os = "macos"))]
pub fn set_window_aspect_constraints(
    _handle: &impl raw_window_handle::HasWindowHandle,
    _aspect_w: f64,
    _aspect_h: f64,
) -> Result<(), String> {
    Ok(())
}

// ── File picker ───────────────────────────────────────────────────────────────

/// Open a native file-open dialog and return the chosen path, or `None` if cancelled.
/// `title` is the panel's message text; `allowed_types` filters by UTType identifier
/// (e.g. `&["public.data"]` for any file).  Pass an empty slice for no filter.
#[cfg(target_os = "macos")]
pub fn open_file_picker(title: &str, allowed_types: &[&str]) -> Option<std::path::PathBuf> {
    use objc2::msg_send_id;
    use objc2::rc::Retained;
    use objc2_app_kit::NSOpenPanel;
    use objc2_foundation::{MainThreadMarker, NSString};

    let mtm = MainThreadMarker::new()?;
    let panel = unsafe { NSOpenPanel::openPanel(mtm) };
    unsafe {
        panel.setMessage(Some(&NSString::from_str(title)));
        panel.setCanChooseFiles(true);
        panel.setCanChooseDirectories(false);
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

#[cfg(not(target_os = "macos"))]
pub fn open_file_picker(_title: &str, _allowed_types: &[&str]) -> Option<std::path::PathBuf> {
    None
}
