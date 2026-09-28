//! The window layer through egui and winit: the window shape is asked for as
//! viewport commands, the file picker is the desktop portal via rfd, and the
//! process has no activation or naming hook to reach.

use super::{MIN_WINDOW_W, PICKER_SIZE, panel_initial_size};
use cdj3k_emu_panel::Model;

/// Stands in for "no maximum" when clearing one.
///
/// `ViewportCommand::MaxInnerSize` always sets a limit, so the way to lift one
/// is to put it past any display. Infinity is not used: it reaches the
/// compositor as a size.
const NO_WINDOW_LIMIT: f32 = 16384.0;

/// No host-side activation hook.
pub fn activate_process(_pid: u32) -> Result<(), String> {
    Ok(())
}

/// No host-side process naming hook.
pub fn set_app_name(_name: &str) -> Result<(), String> {
    Ok(())
}

pub fn on_creation_context(cc: &eframe::CreationContext<'_>) {
    let _ = cc;
}

/// Turn the main window into the panel of `model`: user-resizable, restored
/// to slot `instance_id`'s last saved panel frame for that model (or sized to
/// the default), then aspect-locked to `ref_canvas`. The position comes back
/// only where the window system lets a client place itself (not Wayland).
///
/// The aspect itself is held by [`apply_resize_constraints`], which also
/// saves the frame as it changes.
pub fn enter_panel_window(
    ctx: &egui::Context,
    frame: &eframe::Frame,
    instance_id: u32,
    model: Model,
    ref_canvas: (f32, f32),
) {
    let [w, h] = panel_initial_size(ref_canvas);
    let bar = crate::menu::in_window_bar_height();
    ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(true));
    // The picker was created non-resizable, and winit implements that on
    // Wayland by pinning the maximum to the minimum. Asking for
    // `Resizable(true)` does not undo it, so the panel would inherit the
    // picker's height as a ceiling — the window would refuse to grow.
    ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(egui::vec2(
        NO_WINDOW_LIMIT,
        NO_WINDOW_LIMIT,
    )));
    let min = egui::vec2(MIN_WINDOW_W, MIN_WINDOW_W * (ref_canvas.1 / ref_canvas.0) + bar);
    ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(min));
    let key = super::frame_key(instance_id, model);
    let saved = super::frame_store::load(&key);
    let size = saved.map_or(egui::vec2(w, h + bar), |f| f.size.max(min));
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
    if let Some(pos) = saved.and_then(|f| f.pos) {
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
    }
    super::frame_store::track(ctx, key, saved);
    let _ = frame;
}

/// Turn the main window back into the compact picker: fixed size, centred
/// where the panel was.
pub fn enter_picker_window(ctx: &egui::Context, frame: &eframe::Frame) {
    super::frame_store::untrack();
    // Min first: a viewport that is still at the panel's minimum would
    // clamp the picker size on the way down.
    let size = egui::vec2(PICKER_SIZE[0], PICKER_SIZE[1]);
    ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(size));
    ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(size));
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
    ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(false));
    let _ = frame;
}

/// Open the desktop's own file-open dialog and return the chosen path, or
/// `None` if cancelled.
///
/// The desktop portal, via rfd: a D-Bus request the desktop's own file
/// manager answers, so no GUI toolkit is linked in and the dialog looks
/// like every other one the user sees.
pub fn open_file_picker(title: &str, allowed_types: &[&str]) -> Option<std::path::PathBuf> {
    let mut dialog = rfd::FileDialog::new().set_title(title);
    if !allowed_types.is_empty() {
        dialog = dialog.add_filter("Firmware", allowed_types);
    }
    dialog.pick_file()
}
