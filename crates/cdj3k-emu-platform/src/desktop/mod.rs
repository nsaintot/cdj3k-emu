//! Platform desktop integration: native window options, the window shell,
//! file pickers, and revealing a file in the host's file manager.
//!
//! **Rendering:** egui draws the UI with **OpenGL** (glow) into the window's
//! content area. On macOS that is not Cocoa/AppKit `NSView` drawing APIs for
//! the panel - only the **window shell** (title bar, resize) is standard AppKit
//! `NSWindow`.
//!
//! **Two window shapes.** The app opens as the compact, fixed-size model
//! picker ([`PICKER_SIZE`]). Choosing a model turns the same window into the
//! resizable, aspect-locked panel ([`enter_panel_window`]); "Switch
//! Emulation" shrinks it back ([`enter_picker_window`]). Only the panel frame
//! is persisted, per instance slot and model ([`frame_key`]), by each host's
//! adapter: AppKit's frame autosave on macOS, a file of our own on Linux and
//! Windows.

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod imp;
#[cfg(windows)]
#[path = "windows.rs"]
mod imp;
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
#[path = "unsupported.rs"]
mod imp;

/// The window layer drawn through egui and winit, for an adapter whose host
/// has no native one to call.
#[cfg(any(target_os = "linux", windows))]
mod portable;
#[cfg(any(target_os = "linux", windows))]
mod frame_store;

pub mod aspect_fit;
pub mod first_frame;

pub use imp::{
    activate_process, announce_running, apply_resize_constraints, enter_panel_window, enter_picker_window,
    on_creation_context, open_file_picker, reveal_in_file_manager, set_caption_area, CAPTION_IN_STRIP,
    OWN_WINDOW_ICON, PLACES_WINDOWS,
};

/// The in-window strip standing in for the title bar, where
/// [`CAPTION_IN_STRIP`]: its rectangle and the widgets on it, in points. What
/// is in `strip` and in none of `widgets` drags the window.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptionArea {
    pub strip: egui::Rect,
    pub widgets: Vec<egui::Rect>,
    pub pixels_per_point: f32,
}

/// What holding a window to its aspect carries between frames: the settle
/// snap's view of the size.
#[derive(Debug)]
pub struct ResizeState {
    last_size: egui::Vec2,
    changed_at: std::time::Instant,
}

impl Default for ResizeState {
    fn default() -> Self {
        Self {
            last_size: egui::Vec2::ZERO,
            changed_at: std::time::Instant::now(),
        }
    }
}

/// How long the size has to stay put before the settle snap fires, so a drag
/// in progress is not fought. Time rather than frames: frames arrive only as
/// fast as something repaints.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(40);
/// Movement (px²) under which the size counts as unchanged.
const STABLE_SIZE_TOL_SQ: f32 = 0.25;
/// Width error (px) the settle snap lets stand. Above a whole pixel, so it
/// never chases the rounding of a native aspect lock.
const SNAP_TOL_PX: f32 = 1.5;

/// Square the window up to `ref_w`:`ref_h` once a resize has settled.
///
/// Behind every native lock, because those constrain resizes and nothing
/// else: a window that comes up at the wrong shape, from a saved frame or a
/// model change, would otherwise stay that way. Where the lock works, a
/// settled window is already square and this does nothing.
fn snap_when_settled(ctx: &egui::Context, state: &mut ResizeState, (ref_w, ref_h): (f32, f32)) {
    let size = ctx.screen_rect().size();
    // A maximized or full-screen window takes the screen's shape; resizing it
    // would take it out of that state.
    let whole_screen = ctx.input(|i| {
        let v = i.viewport();
        v.maximized == Some(true) || v.fullscreen == Some(true)
    });
    if whole_screen {
        state.last_size = size;
        return;
    }
    if (size - state.last_size).length_sq() >= STABLE_SIZE_TOL_SQ {
        state.changed_at = std::time::Instant::now();
    }
    state.last_size = size;
    let settled_for = state.changed_at.elapsed();
    if settled_for < SETTLE {
        // Nothing else may repaint in time, so ask for the frame that will
        // find the size settled.
        ctx.request_repaint_after(SETTLE - settled_for);
        return;
    }
    // The in-window menu is inside the window but outside the chassis, so the
    // aspect applies to what is left below it.
    let bar = crate::menu::in_window_bar_height();
    let want_w = (size.y - bar).max(1.0) * ref_w / ref_h;
    if (size.x - want_w).abs() > SNAP_TOL_PX {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::Vec2::new(want_w, size.y)));
        state.changed_at = std::time::Instant::now();
    }
}

/// The name a panel frame is saved under, per slot and model.
///
/// The CDJ-3000 uses the unsuffixed name, which existing saved frames use;
/// other models append their slug (different canvas aspect).
pub fn frame_key(instance_id: u32, model: cdj3k_emu_panel::Model) -> String {
    match model {
        cdj3k_emu_panel::Model::Cdj3k => format!("cdj3k-emu-instance-{}", instance_id),
        other => format!("cdj3k-emu-instance-{}-{}", instance_id, other.slug()),
    }
}

/// Set the process's user-visible name (Dock tile, menu bar, Activity Monitor).
/// Must be called before [`eframe::run_native`].
pub fn set_app_name(name: &str) {
    let _ = imp::set_app_name(name);
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

pub fn native_options(instance_id: u32) -> eframe::NativeOptions {
    let title = format!("{} - {}", crate::app_meta::APP_DISPLAY_NAME, instance_id);
    // The app id has to match the .desktop file's name, or a Wayland
    // compositor cannot pair the window with its entry: GNOME then has no
    // name and no icon for it, and calls it "Unknown" in its dialogs.
    let viewport = egui::ViewportBuilder::default()
        .with_title(title)
        .with_app_id(crate::app_meta::APP_ID)
        .with_inner_size(PICKER_SIZE)
        .with_min_inner_size(PICKER_SIZE)
        .with_resizable(false);
    eframe::NativeOptions {
        viewport,
        centered: true,
        vsync: imp::VSYNC,
        ..Default::default()
    }
}

/// A file dialog the frame loop polls for its answer ([`PendingPick::take`]).
///
/// How the dialog runs is the host's: on a thread of its own where blocking
/// the UI thread would stop the app answering the desktop (Linux, Windows),
/// on the UI thread where the toolkit requires it (AppKit).
pub struct PendingPick {
    rx: std::sync::mpsc::Receiver<Option<std::path::PathBuf>>,
}

impl PendingPick {
    /// Open a picker for `title`, filtered to `allowed_types`.
    pub fn open(title: &str, allowed_types: &[&str]) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        imp::run_picker(title, allowed_types, tx);
        Self { rx }
    }

    /// The answer, once there is one. `Some(None)` is a cancelled dialog.
    pub fn take(&self) -> Option<Option<std::path::PathBuf>> {
        match self.rx.try_recv() {
            Ok(v) => Some(v),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(None),
        }
    }
}
