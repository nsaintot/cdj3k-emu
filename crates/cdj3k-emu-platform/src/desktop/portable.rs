//! The window layer through egui and winit: the window shape is asked for as
//! viewport commands, the file picker is the host's native dialog via rfd,
//! and the process has no activation or naming hook to reach.

use super::{first_frame, panel_initial_size, MIN_WINDOW_W, PICKER_SIZE};
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
/// to slot `instance_id`'s last saved panel frame for that model (or the
/// default size fitted to the monitor, centred where the picker was), then
/// aspect-locked to `ref_canvas`. The position comes back
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
    let min = egui::vec2(
        MIN_WINDOW_W,
        MIN_WINDOW_W * (ref_canvas.1 / ref_canvas.0) + bar,
    );
    ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(min));
    let key = super::frame_key(instance_id, model);
    let saved = super::frame_store::load(&key);
    let now = ctx.input(|i| i.viewport().clone());
    let size = match saved {
        Some(f) => f.size.max(min),
        None => first_frame::fit_to_monitor(egui::vec2(w, h), bar, min, now.monitor_size),
    };
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
    let pos = match saved {
        Some(f) => f.pos,
        None => now
            .outer_rect
            .zip(now.inner_rect)
            .map(|(outer, inner)| first_frame::centred_on(outer, inner, size)),
    };
    if let Some(pos) = pos {
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

/// Run the picker on a thread of its own: the dialog blocks the thread it
/// runs on, and a UI thread that stops answering the compositor is offered
/// for killing while its own picker is open.
pub fn run_picker(
    title: &str,
    allowed_types: &[&str],
    reply: std::sync::mpsc::Sender<Option<std::path::PathBuf>>,
) {
    let title = title.to_string();
    let types: Vec<String> = allowed_types.iter().map(|s| s.to_string()).collect();
    let spawned = std::thread::Builder::new()
        .name("cdj3k-emu-file-dialog".into())
        .spawn({
            let reply = reply.clone();
            move || {
                let refs: Vec<&str> = types.iter().map(String::as_str).collect();
                let _ = reply.send(open_file_picker(&title, &refs));
            }
        });
    if spawned.is_err() {
        // No thread: a cancelled dialog rather than a wait that never ends.
        let _ = reply.send(None);
    }
}

/// Open the desktop's own file-open dialog and return the chosen path, or
/// `None` if cancelled.
///
/// Through rfd: the desktop portal (D-Bus) on Linux, the Win32 common dialog
/// on Windows. No GUI toolkit is linked in.
pub fn open_file_picker(title: &str, allowed_types: &[&str]) -> Option<std::path::PathBuf> {
    let mut dialog = rfd::FileDialog::new().set_title(title);
    if !allowed_types.is_empty() {
        dialog = dialog.add_filter("Firmware", allowed_types);
    }
    dialog.pick_file()
}
