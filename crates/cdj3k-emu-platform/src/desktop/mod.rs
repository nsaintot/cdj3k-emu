//! Platform desktop integration: native window options, AppKit hooks for
//! macOS (window aspect/autosave/tabbing), file pickers, and the trackpad
//! scroll-phase probe used by jog haptic feedback.
//!
//! **Rendering:** egui draws the UI with **OpenGL** (glow) into the window's
//! content area. That is not Cocoa/AppKit `NSView` drawing APIs for the
//! panel - only the **window shell** (title bar, resize) is standard AppKit
//! `NSWindow`. Aspect-ratio resize is enforced with `NSWindow` methods, not
//! in egui.
//!
//! **Two window shapes.** The app opens as the compact, fixed-size model
//! picker ([`PICKER_SIZE`]). Choosing a model turns the same window into the
//! resizable, aspect-locked panel ([`enter_panel_window`]); "Switch
//! Emulation" shrinks it back ([`enter_picker_window`]). Only the panel frame
//! is persisted (AppKit frame autosave, one name per instance slot).

mod macos;

use cdj3k_emu_panel::Model;

pub use macos::open_file_picker;

/// Set the process's user-visible name (Dock tile, menu bar, Activity Monitor).
/// Must be called before [`eframe::run_native`].
pub fn set_app_name(name: &str) {
    let _ = macos::set_app_name(name);
}

/// Compact, fixed-size window the app wears whenever it is not a panel
/// (points): the startup picker, and the setup window over it. One shape for
/// both, so stepping between them never resizes anything.
pub const PICKER_SIZE: [f32; 2] = [760.0, 600.0];

const MIN_WINDOW_W: f32 = 200.0;

/// Scale applied to the minimum window width for a fresh panel window.
const INITIAL_WINDOW_SCALE: f32 = 5.0;

/// Default panel window size (points) for a `ref_w : ref_h` reference canvas.
pub fn panel_initial_size((ref_w, ref_h): (f32, f32)) -> [f32; 2] {
    let w = MIN_WINDOW_W * INITIAL_WINDOW_SCALE;
    [w, w * (ref_h / ref_w)]
}

/// Frame-autosave name for slot `instance_id`'s panel window of `model`.
/// The CDJ-3000 keeps the pre-model name so existing installs keep their
/// frame; other models get their slug appended (different canvas aspect).
fn autosave_name(instance_id: u32, model: Model) -> String {
    match model {
        Model::Cdj3k => format!("cdj3k-emu-instance-{}", instance_id),
        other => format!("cdj3k-emu-instance-{}-{}", instance_id, other.slug()),
    }
}

pub fn native_options(instance_id: u32) -> eframe::NativeOptions {
    let title = format!("{} - {}", crate::app_meta::APP_DISPLAY_NAME, instance_id);
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size(PICKER_SIZE)
            .with_min_inner_size(PICKER_SIZE)
            .with_resizable(false),
        centered: true,
        ..Default::default()
    }
}

/// Best-effort from [`eframe::CreationContext`] (view may not be in a window yet).
pub fn on_creation_context(cc: &eframe::CreationContext<'_>) {
    #[cfg(target_os = "macos")]
    {
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
    let _ = cc;
}

/// Turn the main window into the panel of `model`: user-resizable, restored
/// to slot `instance_id`'s last saved panel frame for that model (or sized to
/// the default and kept centred where the picker was), then aspect-locked to
/// `ref_canvas`.
pub fn enter_panel_window(
    frame: &eframe::Frame,
    instance_id: u32,
    model: Model,
    ref_canvas: (f32, f32),
) {
    #[cfg(target_os = "macos")]
    {
        let _ = macos::set_window_resizable(frame, true);
        let restored = macos::set_window_autosave_name(frame, &autosave_name(instance_id, model))
            .unwrap_or(false);
        if !restored {
            let [w, h] = panel_initial_size(ref_canvas);
            let _ = macos::set_window_content_size_centered(frame, w as f64, h as f64);
        }
        let _ =
            macos::set_window_aspect_constraints(frame, ref_canvas.0 as f64, ref_canvas.1 as f64);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (frame, instance_id, model, ref_canvas);
}

/// Turn the main window back into the compact picker: stop frame autosave (so
/// the picker size is never recorded as the panel frame), fixed size, centred
/// where the panel was.
pub fn enter_picker_window(frame: &eframe::Frame) {
    #[cfg(target_os = "macos")]
    {
        let _ = macos::clear_window_autosave_name(frame);
        let _ = macos::set_window_content_size_centered(
            frame,
            PICKER_SIZE[0] as f64,
            PICKER_SIZE[1] as f64,
        );
        let _ = macos::set_window_resizable(frame, false);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = frame;
}

/// Re-apply AppKit aspect constraints every frame so nothing in the stack
/// resets them during resize. `None` while the picker is up (fixed size).
#[cfg(target_os = "macos")]
pub fn apply_macos_resize_constraints_from_frame(
    frame: &eframe::Frame,
    ref_canvas: Option<(f32, f32)>,
) {
    if let Some((w, h)) = ref_canvas {
        let _ = macos::set_window_aspect_constraints(frame, w as f64, h as f64);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn apply_macos_resize_constraints_from_frame(
    _frame: &eframe::Frame,
    _ref_canvas: Option<(f32, f32)>,
) {
}
