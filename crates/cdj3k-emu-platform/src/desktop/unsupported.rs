//! A host with no window adapter: the window keeps the shape it opened with.

use cdj3k_emu_panel::Model;

pub fn activate_process(_pid: u32) -> Result<(), String> {
    Ok(())
}

pub fn set_app_name(_name: &str) -> Result<(), String> {
    Ok(())
}

pub fn on_creation_context(_cc: &eframe::CreationContext<'_>) {}

pub fn enter_panel_window(
    _ctx: &egui::Context,
    _frame: &eframe::Frame,
    _instance_id: u32,
    _model: Model,
    _ref_canvas: (f32, f32),
) {
}

pub fn enter_picker_window(_ctx: &egui::Context, _frame: &eframe::Frame) {}

/// The settle snap alone, which needs nothing from the host.
pub fn apply_resize_constraints(
    ctx: &egui::Context,
    _frame: &eframe::Frame,
    state: &mut super::ResizeState,
    ref_canvas: Option<(f32, f32)>,
) {
    if let Some(canvas) = ref_canvas {
        super::snap_when_settled(ctx, state, canvas);
    }
}

pub fn open_file_picker(_title: &str, _allowed_types: &[&str]) -> Option<std::path::PathBuf> {
    None
}

pub fn reveal_in_file_manager(_path: &std::path::Path) {}

/// Whether a second window can be centred over the first.
pub const PLACES_WINDOWS: bool = false;
