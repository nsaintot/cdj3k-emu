mod bloom;
mod boot_overlay;
mod buttons;
pub(crate) mod firmware_wizard;
mod frame_inject;
mod jog_physics;
mod lcd_texture;
mod lcd_touch;
mod mods_view;
mod picker;
mod screenshot;
mod script;
mod scroll;
pub mod shell;
mod table;
mod theme;
mod tilt_input;
mod ui;
mod update;
mod updater;
mod viewports;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::Color32;

use cdj3k_emu_panel::miso_frame::{self};
use cdj3k_emu_panel::Model;
use cdj3k_emu_platform::menu_state;
use cdj3k_emu_streams::ctrl_stream::{CtrlStream, LedState};
use cdj3k_emu_streams::jog_stream::JogLcdStream;
use cdj3k_emu_streams::main_stream::MainLcdStream;
use cdj3k_emu_streams::RepaintGate;

use viewports::DebugViewportState;

pub(crate) use lcd_touch::LcdTouchCapture;

/// Minimum wall-clock interval between presented frames - caps the main
/// viewport at ~60 fps regardless of vsync rate or how many sources
/// requested a repaint.
pub(crate) const MIN_FRAME_INTERVAL: Duration = Duration::from_micros(16_667);

/// `jog_vel` neutral-position encoding (inverse: max u16 = stopped).
const JOG_VEL_INIT: u16 = 0xffff;

/// `rotary` at rest. Must equal the value the guest reads while nothing turns
/// ([`cdj3k_emu_panel::miso_frame::ROTARY_IDLE`]), or the first injected
/// frame reads as one detent.
const ROTARY_NEUTRAL: u16 = cdj3k_emu_panel::miso_frame::ROTARY_IDLE;

/// Initial tempo slider position (0.0 = full minus, 0.5 = centre, 1.0 = full plus).
const TEMPO_INIT: f32 = 0.5;

/// Puffin profiling HTTP server address. Localhost-only so an enabled
/// profiling build doesn't expose the endpoint on any host interface.
const PUFFIN_SERVER_ADDR: &str = "127.0.0.1:8585";

pub struct CdjApp {
    /// The model whose slate is drawn (selects layout + control set).
    model: Model,

    jog_pos: u16,
    /// Inverse encoding: `0xffff` = stopped, `0x0000` = max speed.
    jog_vel: u16,
    jog_touch: u8,
    /// Visual rotation of the platter (radians). Wraps freely; rendering uses `.rem_euclid(TAU)`.
    jog_angle: f32,
    /// Accumulator for emitting jog ticks to the device (fractional ticks between u16 steps).
    jog_accum: f32,
    /// Last `jog_angle` value fed into the tick accumulator. Lets the encoder use actual
    /// angle deltas (not `jog_omega * dt`) so slow drags register correctly.
    jog_angle_last_encoded: f32,
    /// Platter travel in turns, accumulated from the same angle deltas.
    jog_revs: f64,
    /// The platter's turns per second over the last tick, from its angle.
    jog_rps: f32,
    /// Tick accumulator for trackpad haptic feedback. Independent of `jog_accum`
    /// because the haptic detent stride is far coarser than the device tick
    /// stride (3240/rev). Reset to zero whenever `jog_omega` reaches zero so
    /// the next rotation starts on a clean boundary.
    jog_haptic_accum: f32,
    /// Current angular velocity of the platter, rad/s. Decays toward 0 when not dragged.
    jog_omega: f32,
    /// While the user is dragging, this is the most-recently observed pointer angle
    /// around the wheel center (radians, atan2 convention). `None` when idle.
    jog_drag_prev_angle: Option<f32>,
    /// Press-position anchor for a slingshot drag (Ctrl held at drag start).
    /// `Some` for the lifetime of a slingshot drag; on release the pull vector
    /// from anchor → current pointer is projected onto the tangent at the
    /// anchor and turned into an impulse on `jog_omega`. `None` for normal
    /// (rotational) drags.
    jog_slingshot_anchor: Option<egui::Pos2>,
    /// `true` when the current drag originated in the grip band (nudge mode, no touch flag).
    /// `false` when it started on the center platter (scratch/vinyl mode, touch flag set).
    jog_grip_drag: bool,
    /// Seconds remaining on the slingshot-release touch pulse. While > 0,
    /// `tick_jog` forces `touch_active = true` so the firmware sees a flick
    /// event over several MISO polls (700 Hz polling needs ≥ ~5 ms latched to
    /// reliably catch the transition). Decremented by `dt` each tick.
    /// Set by [`Self::jog_apply_impulse`] when the anchor was in the scratch
    /// zone; ignored for grip-zone (bend) slingshots.
    jog_release_touch_pulse_remaining_sec: f32,
    /// Scroll interaction hold timer (s). While > 0, scroll semantics stay active.
    jog_scroll_hold_sec: f32,
    /// Effective touch semantic currently encoded into `jog_touch`.
    jog_touch_active: bool,
    /// JOG ADJUST rotary value in [0, 1]. `0` = LIGHT (long coast), `1` = HEAVY (quick brake).
    jog_adjust: f32,
    jog_screen_popped: bool,
    main_screen_popped: bool,
    debug_screen_popped: bool,
    rotary: u16,
    /// Fader travel `0.0..=1.0`; the player's encoder maps it to its own
    /// slider value (the CDJ-3000's: `cdj3k::tempo_raw`).
    tempo: f32,
    vinyl_speed: u8,
    direction: cdj3k_emu_panel::Direction,

    lcd_touch: Option<(u16, u16)>,
    /// `true` while `CDJ3K_SCRIPT` holds a touch: the slate and popout report
    /// their own pointer state every frame, which would otherwise overwrite
    /// the scripted coordinate before the guest ever samples it.
    script_touch: bool,
    /// `true` while a Ctrl+click latch is active: touch coordinate is frozen until Ctrl is released.
    lcd_touch_ctrl_latched: bool,
    /// `true` after scrolling on the LCD: next click fires the rotary press instead of a touch.
    /// Cleared by any pointer movement.
    lcd_nav_mode: bool,

    /// Currently physically-held button (released on pointer-up).
    held_btn: Option<(usize, u8)>,
    /// Buttons latched by ctrl+click - stay pressed until ctrl is released.
    latched_btns: HashSet<(usize, u8)>,

    nav_angle: f32,
    /// Sub-detent scroll accumulator (px). Fires a rotary tick each `NAV_SCROLL_PX_PER_TICK` px.
    nav_scroll_accum: f32,
    jog_adjust_scroll_accum: f32,
    vinyl_scroll_accum: f32,
    /// BEAT LOOP encoder: how far it has turned, and its sub-detent scroll
    /// accumulator (px).
    beat_loop_angle: f32,
    beat_loop_scroll_accum: f32,
    /// The BEAT LOOP encoder's absolute counter, sent in the frame.
    beat_loop: u16,
    /// The deck is shown tilted, its front face in view (a slate with one).
    tilted: bool,
    tilt_input: Option<tilt_input::TiltInput>,
    /// Tilt this frame, 0 flat to 1, set by the slate as it draws. Part of the
    /// bloom scene: the front face's lamps move with it.
    tilt_progress: f32,
    tilt_cache: ui::tilt::TiltCache,
    raw_pointer: tilt_input::RawPointer,

    ctrl_stream: CtrlStream,

    display_stream: MainLcdStream,
    textures: LcdTextures,
    /// Frame bits the input script forces low (`clear`), applied last when a
    /// frame is built.
    cleared_bits: Vec<(usize, u8)>,
    /// Buttons the input script holds down (`press`), set in every frame
    /// until the script releases them. Kept apart from `held_btn`, which the
    /// slate's widgets clear on the next frame the pointer is not on them.
    scripted_btns: Vec<(usize, u8)>,
    /// Whether the control channel was up on the previous frame; a rising
    /// edge injects the current frame.
    ctrl_was_ready: bool,

    jog_stream: JogLcdStream,
    /// Corner samples from the last jog frame: SLIP, VINYL, SYNC, MASTER label fills.
    jog_corner_label_colors: [Color32; 4],

    /// Latest LED state received from the guest (updated on each new frame).
    led_state: LedState,

    bloom: Option<bloom::SharedBloom>,
    /// LCD screen rects captured each frame (egui coords, Y-down) - fed to the bloom exclusion mask.
    bloom_excludes: Vec<egui::Rect>,
    /// Controls drawn over the screens in [`Self::bloom_excludes`].
    bloom_keeps: Vec<bloom::BloomKeep>,
    status: String,

    /// Cached static jog wheel geometry (outer rings + inner disk/knob).
    /// Rebuilt only when the window scale or `jog_adjust` changes.
    jog_static_cache: Option<ui::draw_cache::JogStaticCache>,

    caches: ui::draw_cache::ShapeCaches,

    fps_smooth: f32,
    /// What holding the window to the chassis aspect carries between frames.
    resize: cdj3k_emu_platform::desktop::ResizeState,
    /// Shapes submitted to the egui painter in the current frame (reset each update).
    frame_shape_count: u64,
    /// Last MISO frame sent to the device (first 32 bytes displayed for debugging).
    last_miso: [u8; miso_frame::MISO_SIZE],
    debug: DebugView,

    /// When QEMU was last seen transitioning to running. The boot overlay
    /// clears on the first non-zero panel frame, or `BOOT_SHADE_TIMEOUT`
    /// after this.
    qemu_running_since: Option<Instant>,
    qemu_was_running: bool,
    /// Current rendered alpha of the boot/idle shade in [0, 1]. Linearly ramps
    /// toward the target each frame so the overlay fades in/out over 1 s.
    shade_alpha: f32,
    /// `true` while the boot guard has not yet cleared (target_alpha > 0).
    /// Main LCD and jog LCD textures are blanked when this is set.
    lcds_blanked: bool,
    /// Set when QEMU exits so the next paint zeroes the main + jog GL textures.
    /// Without this, popouts would keep displaying the last captured frame.
    lcd_textures_need_blank: bool,
}

/// The main and jog LCDs' GL textures, owned here rather than by egui's
/// texture manager.
#[derive(Default)]
struct LcdTextures {
    /// Raw GL texture handle - created once, re-used for every dirty upload.
    main_gl: Option<glow::Texture>,
    /// egui TextureId returned by `register_native_glow_texture` - used with `painter.image`.
    main_id: Option<egui::TextureId>,
    /// Set by a model switch whose main LCD size differs; the next upload
    /// re-specifies the texture storage.
    main_stale: bool,
    /// GL texture for the jog LCD - owned by us (not egui's tex_manager). Created once with
    /// `TEXTURE_SWIZZLE_R = BLUE`, `TEXTURE_SWIZZLE_B = RED`, `TEXTURE_SWIZZLE_A = ONE` so
    /// the GPU samples the wire-format XRGB bytes as RGBA(A=1) for free.
    jog_gl: Option<glow::Texture>,
    jog_id: Option<egui::TextureId>,
}

/// What the debug viewport shows, and the handles it shares with the panel.
struct DebugView {
    jog_source: &'static str,
    jog_delta_rad: f32,
    jog_dt: f32,
    jog_omega_sample: f32,
    /// Rendered above the MISO dump (3 centred lines).
    jog_lines: [String; 3],
    /// Snapshot of debug-relevant state, refreshed each main update. Read by
    /// the deferred debug viewport (which runs in its own update cycle and
    /// therefore cannot borrow `&self`).
    snapshot: Arc<Mutex<ui::DebugSnapshot>>,
    viewport_state: Arc<Mutex<DebugViewportState>>,
    /// Set by the deferred debug viewport when its window's × is clicked.
    wants_close: Arc<std::sync::atomic::AtomicBool>,
}

/// Owns the puffin HTTP server for the life of the process when profiling is
/// enabled.  The server must outlive `CdjApp::new` (otherwise no client can
/// connect) but never needs to be reclaimed - storing it here makes the
/// lifetime explicit instead of via `mem::forget`.  Empty when launched
/// without `--profile` / `CDJ3K_PROFILE=1`.
static PUFFIN_SERVER: std::sync::OnceLock<puffin_http::Server> = std::sync::OnceLock::new();

/// Turn puffin's scope recording on and serve it on [`PUFFIN_SERVER_ADDR`].
fn start_profiler() {
    if let Ok(server) = puffin_http::Server::new(PUFFIN_SERVER_ADDR) {
        let _ = PUFFIN_SERVER.set(server);
        eprintln!("cdj3k-emu: puffin server listening on {PUFFIN_SERVER_ADDR}");
    } else {
        eprintln!("cdj3k-emu: puffin server failed to bind {PUFFIN_SERVER_ADDR}");
    }
    puffin::set_scopes_on(true);
}

/// Set up what the panel needs before its first frame, and return the slot's
/// saved panel settings.
fn prepare_panel(egui_ctx: &egui::Context) -> cdj3k_emu_storage::PanelSettings {
    // Open the trackpad haptic actuator (private MultitouchSupport API).
    // No-op + silent failure if the device has no actuator or the macOS
    // private struct layout has drifted.
    cdj3k_emu_platform::haptic::init();

    // Prevent resizing the window with the keyboard.
    egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
    egui_ctx.set_zoom_factor(1.0);
    let instance_id = cdj3k_emu_platform::menu_state::lock().current_instance_id;
    let panel = cdj3k_emu_storage::PanelSettings::load(instance_id);
    // The menu owns the screen-extend flag; restored before the first frame.
    cdj3k_emu_platform::menu_state::lock().screen_extended = panel.screen_extended;
    panel
}

impl CdjApp {
    /// `profile`: when true, enables puffin scope recording and binds a
    /// localhost TCP listener for `puffin_viewer` to connect to.  When
    /// false (shipping default), every `puffin::profile_*` macro becomes
    /// a single relaxed-atomic load and no TCP port is opened.
    pub fn new(model: Model, socket_dir: String, egui_ctx: egui::Context, profile: bool) -> Self {
        if profile {
            start_profiler();
        }
        let panel = prepare_panel(&egui_ctx);
        let repaint_gate = RepaintGate::new(egui_ctx.clone(), MIN_FRAME_INTERVAL);
        // `--no-spawn` never shows the boot shade, so it must not fade in from
        // opaque either (keeps chassis captures deterministic frame to frame).
        let ui_only = menu_state::lock().ui_only;
        Self {
            model,
            jog_pos: 0,
            jog_vel: JOG_VEL_INIT,
            jog_touch: 0x00,
            jog_angle: 0.0,
            jog_accum: 0.0,
            jog_angle_last_encoded: 0.0,
            jog_revs: 0.0,
            jog_rps: 0.0,
            jog_haptic_accum: 0.0,
            jog_omega: 0.0,
            jog_drag_prev_angle: None,
            jog_slingshot_anchor: None,
            jog_grip_drag: false,
            jog_release_touch_pulse_remaining_sec: 0.0,
            jog_scroll_hold_sec: 0.0,
            jog_touch_active: false,
            jog_adjust: panel.jog_adjust,
            jog_screen_popped: false,
            main_screen_popped: false,
            debug_screen_popped: false,
            rotary: ROTARY_NEUTRAL,
            tempo: TEMPO_INIT,
            vinyl_speed: panel.vinyl_speed,
            direction: cdj3k_emu_panel::Direction::Forward,
            lcd_touch: None,
            script_touch: false,
            lcd_touch_ctrl_latched: false,
            lcd_nav_mode: false,
            held_btn: None,
            latched_btns: HashSet::new(),
            nav_angle: 0.0,
            nav_scroll_accum: 0.0,
            jog_adjust_scroll_accum: 0.0,
            vinyl_scroll_accum: 0.0,
            beat_loop_angle: 0.0,
            beat_loop_scroll_accum: 0.0,
            beat_loop: 0,
            tilted: false,
            tilt_input: None,
            tilt_progress: 0.0,
            tilt_cache: Default::default(),
            raw_pointer: Default::default(),
            ctrl_stream: CtrlStream::new(&socket_dir, repaint_gate.clone()),
            display_stream: MainLcdStream::new(&socket_dir, repaint_gate.clone()),
            textures: Default::default(),
            cleared_bits: Vec::new(),
            scripted_btns: Vec::new(),
            ctrl_was_ready: false,
            jog_stream: JogLcdStream::new(&socket_dir, repaint_gate.clone()),
            jog_corner_label_colors: [ui::COL_BTN_TEXT; 4],
            led_state: LedState::default(),
            bloom: None,
            bloom_excludes: Vec::new(),
            bloom_keeps: Vec::new(),
            status: "Starting...".to_owned(),
            jog_static_cache: None,
            caches: Default::default(),
            fps_smooth: 60.0,
            resize: Default::default(),
            frame_shape_count: 0,
            last_miso: [0u8; miso_frame::MISO_SIZE],
            debug: Default::default(),
            qemu_running_since: None,
            qemu_was_running: false,
            shade_alpha: if ui_only { 0.0 } else { 1.0 },
            lcds_blanked: !ui_only,
            lcd_textures_need_blank: false,
        }
    }

    pub fn model(&self) -> Model {
        self.model
    }

    /// Re-target the panel at another model. The streams, GL textures and
    /// jog physics carry over; every shape cache is dropped because the
    /// caches key on the window scale only, not on the slate.
    pub fn switch_model(&mut self, model: Model) {
        if model.main_lcd() != self.model.main_lcd() {
            self.textures.main_stale = true;
        }
        self.model = model;
        self.on_leave_panel();
        self.tilted = false;
        self.tilt_progress = 0.0;
        // Frame bits and LEDs are positions in the previous model's frames.
        self.cleared_bits.clear();
        self.scripted_btns.clear();
        self.last_miso = [0u8; miso_frame::MISO_SIZE];
        self.led_state = LedState::default();
        self.jog_static_cache = None;
        self.caches = Default::default();
        // The window is re-shaped for the new canvas; let the aspect snap
        // re-evaluate from scratch.
        self.resize = Default::default();
    }

    /// The panel is hidden (picker shown): release any held or latched
    /// control, and drop the tilt's pointer map, which belongs to the window as
    /// last drawn.
    pub(crate) fn on_leave_panel(&mut self) {
        self.tilt_input = None;
        self.raw_pointer = Default::default();
        if self.held_btn.is_some() || !self.latched_btns.is_empty() || self.lcd_touch.is_some() {
            self.held_btn = None;
            self.latched_btns.clear();
            self.lcd_touch = None;
            self.lcd_touch_ctrl_latched = false;
            self.inject(self.build_current_frame());
        }
    }

    /// Hash of every input that influences bloom-relevant pixels for the
    /// current frame. The bloom pipeline reuses its cached blur texture when
    /// this key is unchanged from the previous frame, skipping the GL work
    /// (blit + threshold + 9-tap separable blur).
    pub(super) fn bloom_scene_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.led_state.frame.hash(&mut h);
        // `held_btn` and `latched_btns` are `(usize, u8)` tuples - hash
        // them directly.  HashSet iteration order is non-deterministic
        // so we sort the latched set into a stack array before hashing
        // to keep the key stable across frames with no allocation in
        // the steady state.
        self.held_btn.hash(&mut h);
        let mut latched: [(usize, u8); 32] = [(0, 0); 32];
        let n = self.latched_btns.len().min(latched.len());
        for (slot, b) in latched.iter_mut().zip(self.latched_btns.iter()) {
            *slot = *b;
        }
        latched[..n].sort_unstable();
        n.hash(&mut h);
        latched[..n].hash(&mut h);
        ((self.shade_alpha.clamp(0.0, 1.0) * 255.0) as u8).hash(&mut h);
        self.tilt_progress.to_bits().hash(&mut h);
        h.finish()
    }
}

impl CdjApp {
    /// App teardown: power the guest off, stop the runtime worker, persist
    /// the GUI-side settings and release the runtime files.
    pub fn on_exit(&mut self, gl: Option<&glow::Context>) {
        // Inject the power-off MISO frame directly: on app exit the egui loop
        // is already stopped, so `poll_menu_state` will never consume
        // POWER_OFF_STIMULI_REQUESTED. Without this, instance.stop()'s 8 s
        // EP122 wait elapses with the guest never having seen the signal.
        self.inject(self.power_off_frame());

        // Mark shutdown so the runtime worker observes it at the top of its
        // next loop iteration and runs `instance.stop()` (graceful
        // POWER_OFF_STIMULI → ACPI system_powerdown → QMP quit, with its own
        // per-step watchdogs).
        cdj3k_emu_platform::menu_state::APP_SHUTDOWN
            .store(true, std::sync::atomic::Ordering::Relaxed);

        // Save GUI-side state in parallel with the worker's shutdown.
        if let (Some(gl), Some(bloom)) = (gl, &self.bloom) {
            bloom.lock().unwrap().destroy(gl);
        }
        // One borrow of the menu state: taking it twice in one expression
        // would deadlock on its own mutex.
        let (screen_extended, instance_id) = {
            let s = cdj3k_emu_platform::menu_state::lock();
            (s.screen_extended, s.current_instance_id)
        };
        let _ = cdj3k_emu_storage::PanelSettings {
            screen_extended,
            jog_adjust: self.jog_adjust,
            vinyl_speed: self.vinyl_speed,
        }
        .save(instance_id);

        // Wait for the worker to finish its graceful QEMU stop. Total budget
        // matches `instance.stop()` (8 s EP122 cleanup + 20 s ACPI shutdown +
        // 5 s QMP quit) plus a small loop-overhead margin.
        let joined = cdj3k_emu_runtime::wait_for_worker(SHUTDOWN_WATCHDOG);
        if !joined {
            // Watchdog elapsed: fall back to SIGTERM/SIGKILL on the QEMU child.
            // Loud so we know if the soft path keeps timing out.
            eprintln!(
                "cdj3k-emu: soft shutdown watchdog ({}s) elapsed - sending SIGTERM/SIGKILL",
                SHUTDOWN_WATCHDOG.as_secs()
            );
            cdj3k_emu_runtime::kill_qemu_child();
        }
        cdj3k_emu_runtime::cleanup_runtime_files();
        cdj3k_emu_platform::haptic::shutdown();
    }
}

/// Maximum wall-clock time we let the runtime worker take to drive the
/// graceful QEMU stop sequence before we fall back to SIGTERM/SIGKILL.
/// Sized to cover `instance.stop()`'s 8 + 20 + 5 s budget plus loop overhead.
const SHUTDOWN_WATCHDOG: std::time::Duration = std::time::Duration::from_secs(35);

impl CdjApp {
    /// Reference canvas of the active slate (the window is aspect-locked to it).
    pub fn ref_canvas(&self) -> (f32, f32) {
        ui::slate::for_model(self.model).ref_canvas
    }
}

impl Default for DebugView {
    fn default() -> Self {
        Self {
            jog_source: "none",
            jog_delta_rad: 0.0,
            jog_dt: 0.0,
            jog_omega_sample: 0.0,
            jog_lines: Default::default(),
            snapshot: Default::default(),
            viewport_state: Default::default(),
            wants_close: Default::default(),
        }
    }
}
