//! Platform desktop integration: native window options, AppKit hooks for
//! macOS (window aspect/autosave/tabbing), file pickers, and the trackpad
//! scroll-phase probe used by jog haptic feedback.
//!
//! **Rendering:** egui draws the UI with **OpenGL** (glow) into the window's
//! content area. That is not Cocoa/AppKit `NSView` drawing APIs for the
//! panel - only the **window shell** (title bar, resize) is standard AppKit
//! `NSWindow`. Aspect-ratio resize is enforced with `NSWindow` methods, not
//! in egui.

mod macos;

pub use macos::open_file_picker;

/// Set the process's user-visible name (Dock tile, menu bar, Activity Monitor).
/// Must be called before [`eframe::run_native`].
pub fn set_app_name(name: &str) {
    let _ = macos::set_app_name(name);
}

pub const LAYOUT_REF_W: f32 = 3185.0;
pub const LAYOUT_REF_H: f32 = 4360.0;

const MIN_WINDOW_W: f32 = 200.0;
const MIN_WINDOW_H: f32 = MIN_WINDOW_W * (LAYOUT_REF_H / LAYOUT_REF_W);

/// Scale applied to the minimum window size for the initial desktop window.
const INITIAL_WINDOW_SCALE: f32 = 5.0;

pub fn native_options(instance_id: u32) -> eframe::NativeOptions {
    let w = MIN_WINDOW_W * INITIAL_WINDOW_SCALE;
    let h = MIN_WINDOW_H * INITIAL_WINDOW_SCALE;
    let title = format!("{} - {}", crate::app_meta::APP_DISPLAY_NAME, instance_id);
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([w, h])
            .with_min_inner_size([MIN_WINDOW_W, MIN_WINDOW_H])
            .with_resizable(true),
        ..Default::default()
    }
}

/// Best-effort from [`eframe::CreationContext`] (view may not be in a window yet).
/// `instance_id` is used as the suffix of the AppKit frame-autosave name so each
/// instance restores its own last position/size on launch.
pub fn on_creation_context(cc: &eframe::CreationContext<'_>, instance_id: u32) {
    #[cfg(target_os = "macos")]
    {
        let _ = macos::set_window_aspect_constraints(cc, LAYOUT_REF_W as f64, LAYOUT_REF_H as f64);
        let _ = macos::set_window_autosave_name(cc, &format!("cdj3k-emu-instance-{}", instance_id));
        let _ = macos::disable_window_tabbing(cc);
        // Class-level kill switch: prevents the deferred debug viewport (and
        // any other NSWindow AppKit spawns) from re-adding "Show Tab Bar" /
        // "Show All Tabs" to the View menu.
        let _ = macos::disable_automatic_window_tabbing_global();
        // Drop eframe's default placeholder icon so the Dock reads our
        // bundle's CFBundleIconFile instead.
        let _ = macos::reset_dock_icon_to_bundle();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (cc, instance_id);
}

/// Re-apply AppKit aspect constraints every frame so nothing in the stack resets them during resize.
#[cfg(target_os = "macos")]
pub fn apply_macos_resize_constraints_from_frame(frame: &eframe::Frame) {
    let _ = macos::set_window_aspect_constraints(frame, LAYOUT_REF_W as f64, LAYOUT_REF_H as f64);
}

#[cfg(not(target_os = "macos"))]
pub fn apply_macos_resize_constraints_from_frame(_frame: &eframe::Frame) {}
