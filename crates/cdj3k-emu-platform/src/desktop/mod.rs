//! Platform desktop integration: native window options, the window shell,
//! file pickers, and revealing a file in the host's file manager.
//!
//! egui draws the UI with OpenGL (glow) into the window's content area. On
//! macOS, AppKit provides only the window shell (title bar and resizing)
//! through `NSWindow`; the panel does not use `NSView` drawing.
//!
//! The window has two shapes. The app opens as the compact, fixed-size model
//! picker ([`PICKER_SIZE`]). Choosing a model turns the same window into the
//! resizable, aspect-locked panel ([`enter_panel_window`]), and emptying the
//! slot shrinks it back ([`enter_picker_window`]). Only the panel frame is
//! saved, per slot and model ([`frame_key`]), by each host's adapter: AppKit's
//! frame autosave on macOS, and a file in the app data directory on Linux and
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

#[cfg(any(target_os = "linux", windows))]
mod frame_store;
/// The window layer drawn through egui and winit, for an adapter whose host
/// has no native one to call.
#[cfg(any(target_os = "linux", windows))]
mod portable;

pub mod aspect_fit;
pub mod first_frame;

use std::sync::atomic::{AtomicBool, Ordering};

static HEADLESS: AtomicBool = AtomicBool::new(false);

/// Run without showing the window, for `--headless`. The window is created
/// hidden and stays hidden, so eframe runs only `logic`. On macOS the process
/// also stays out of the Dock and the menu bar. Call it before
/// [`native_options`].
pub fn set_headless() {
    HEADLESS.store(true, Ordering::Relaxed);
}

/// Whether [`set_headless`] was called.
pub fn is_headless() -> bool {
    HEADLESS.load(Ordering::Relaxed)
}

pub use imp::{
    activate_process, announce_running, answer_quit_request, apply_resize_constraints,
    enter_panel_window, enter_picker_window, open_file_picker, open_url, quit_pending,
    reveal_in_file_manager, set_caption_area, CAPTION_IN_STRIP, OWN_WINDOW_ICON, PLACES_WINDOWS,
};

/// Set when the app is asked to quit other than through the window's close
/// button, and taken by [`take_quit_request`].
static QUIT_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The context to wake when the app is asked to quit.
static QUIT_WAKE: std::sync::OnceLock<egui::Context> = std::sync::OnceLock::new();

/// Apply this host's window settings from [`eframe::CreationContext`], and
/// let [`request_quit`] reach the window.
pub fn on_creation_context(cc: &eframe::CreationContext<'_>) {
    let _ = QUIT_WAKE.set(cc.egui_ctx.clone());
    imp::on_creation_context(cc);
}

/// Ask the window to quit through the same close sequence as its close
/// button. Returns `false` when no window is up to take the request. The host
/// calls it for Cmd+Q on macOS, and for SIGINT and SIGTERM.
pub fn request_quit() -> bool {
    let Some(ctx) = QUIT_WAKE.get() else {
        return false;
    };
    QUIT_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
    ctx.request_repaint();
    true
}

/// Whether the app was asked to quit since the last call.
pub fn take_quit_request() -> bool {
    QUIT_REQUESTED.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// The in-window strip that stands in for the title bar on hosts where
/// [`CAPTION_IN_STRIP`] is true: its rectangle and the widgets on it, in
/// points. A press inside `strip` and outside every rectangle in `widgets`
/// drags the window.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptionArea {
    pub strip: egui::Rect,
    pub widgets: Vec<egui::Rect>,
    pub pixels_per_point: f32,
}

/// The state that the settle snap keeps between frames: the last size and
/// when it last changed.
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
/// in progress is not fought. It is a duration because frames arrive only
/// when something repaints.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(40);
/// Movement (px²) under which the size counts as unchanged.
const STABLE_SIZE_TOL_SQ: f32 = 0.25;
/// The width error in px that the settle snap leaves alone. It is above one
/// pixel, so the snap never chases the rounding of a native aspect lock.
const SNAP_TOL_PX: f32 = 1.5;

/// Square the window up to `ref_w`:`ref_h` once a resize has settled.
///
/// This runs behind every native lock, because a native lock only constrains
/// resizes: a window that comes up at the wrong shape, from a saved frame or a
/// model change, would otherwise stay that way. Where the lock works, a
/// settled window is already square and this does nothing.
fn snap_when_settled(ctx: &egui::Context, state: &mut ResizeState, (ref_w, ref_h): (f32, f32)) {
    let size = ctx.content_rect().size();
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
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::Vec2::new(
            want_w, size.y,
        )));
        state.changed_at = std::time::Instant::now();
    }
}

/// The name a panel frame is saved under, per slot and model.
///
/// The CDJ-3000 uses the unsuffixed name, which existing saved frames use;
/// other models append their slug, because their canvas has a different
/// aspect.
pub fn frame_key(instance_id: u32, model: cdj3k_emu_panel::Model) -> String {
    match model {
        cdj3k_emu_panel::Model::Cdj3k => format!("cdj3k-emu-instance-{}", instance_id),
        other => format!("cdj3k-emu-instance-{}-{}", instance_id, other.slug()),
    }
}

/// Set the process name that the user sees in the Dock tile, the menu bar and
/// Activity Monitor. Call it before [`eframe::run_native`].
pub fn set_app_name(name: &str) {
    let _ = imp::set_app_name(name);
}

/// The compact, fixed size in points of the window whenever it is not a
/// panel: the startup picker and the setup window over it. Both use the same
/// size, so moving between them never resizes the window.
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
        .with_resizable(false)
        .with_visible(!is_headless());
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions {
        viewport,
        centered: true,
        glow_options: eframe::egui_glow::GlowConfiguration {
            vsync: imp::VSYNC,
            ..Default::default()
        },
        ..Default::default()
    };
    #[cfg(target_os = "macos")]
    if is_headless() {
        imp::keep_out_of_dock(&mut options);
    }
    options
}

/// The file types the mod picker accepts: archives, and a folder's `mod.toml`
/// or `loader.sh`.
const MOD_TYPES: &[&str] = &["tgz", "gz", "tar", "toml"];

/// A file dialog the frame loop polls for its answer ([`PendingPick::take`]).
///
/// Each host runs the dialog its own way: on a separate thread where blocking
/// the UI thread would stop the app from answering the desktop (Linux,
/// Windows), and on the UI thread where the toolkit requires it (AppKit).
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

    /// Open a picker for a mod: a `.tgz`, `.tar.gz` or `.tar` archive, or a
    /// folder. Where the host's dialog cannot pick folders, the user picks the
    /// folder's `mod.toml` instead.
    pub fn open_mod(title: &str) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        imp::run_mod_picker(title, MOD_TYPES, tx);
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
