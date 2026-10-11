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

/// The minimum wall-clock interval between presented frames. It caps the
/// main viewport at about 60 fps, whatever the vsync rate and however many
/// sources request a repaint.
pub(crate) const MIN_FRAME_INTERVAL: Duration = Duration::from_micros(16_667);

/// The `jog_vel` of a stopped jog. The encoding is inverted: the maximum u16
/// means stopped.
const JOG_VEL_INIT: u16 = 0xffff;

/// The `rotary` value at rest. It must equal the value the guest reads while
/// nothing turns ([`cdj3k_emu_panel::miso_frame::ROTARY_IDLE`]), or the first
/// injected frame reads as one detent.
const ROTARY_NEUTRAL: u16 = cdj3k_emu_panel::miso_frame::ROTARY_IDLE;

/// Initial tempo slider position: 0.0 is full minus, 0.5 is centre and 1.0 is
/// full plus.
const TEMPO_INIT: f32 = 0.5;

/// Puffin profiling HTTP server address. It is bound to localhost only, so a
/// profiling build does not expose the endpoint on other interfaces.
const PUFFIN_SERVER_ADDR: &str = "127.0.0.1:8585";

pub struct CdjApp {
    /// The model whose slate is drawn. It selects the layout and the control set.
    model: Model,

    jog_pos: u16,
    /// The encoding is inverted: `0xffff` is stopped and `0x0000` is maximum
    /// speed.
    jog_vel: u16,
    jog_touch: u8,
    /// The drawn rotation of the platter, in radians. It is not wrapped;
    /// rendering applies `.rem_euclid(TAU)`.
    jog_angle: f32,
    /// Fractional jog ticks not yet sent to the device.
    jog_accum: f32,
    /// The last `jog_angle` fed into the tick accumulator. The encoder uses the
    /// actual angle deltas, so slow drags register.
    jog_angle_last_encoded: f32,
    /// Platter travel in turns, accumulated from the same angle deltas.
    jog_revs: f64,
    /// The platter's turns per second over the last tick, from its angle.
    jog_rps: f32,
    /// Tick accumulator for trackpad haptic feedback. It is separate from
    /// `jog_accum` because a haptic detent is far coarser than a device tick
    /// (3240 per turn).
    jog_haptic_accum: f32,
    /// The platter's angular velocity in rad/s. It decays toward 0 while the
    /// platter is not dragged.
    jog_omega: f32,
    /// While the user drags, the last pointer angle around the wheel center, in
    /// radians with the atan2 convention. `None` when idle.
    jog_drag_prev_angle: Option<f32>,
    /// The press position that anchors a slingshot drag (Ctrl held at drag
    /// start). It is `Some` for the whole slingshot drag; on release, the pull
    /// vector from the anchor to the pointer is projected onto the tangent at
    /// the anchor and applied as an impulse on `jog_omega`. It is `None` for a
    /// normal, rotational drag.
    jog_slingshot_anchor: Option<egui::Pos2>,
    /// `true` when the current drag started in the grip band (nudge mode, no
    /// touch flag), `false` when it started on the center platter (scratch
    /// mode, touch flag set).
    jog_grip_drag: bool,
    /// Seconds left on the touch pulse after a slingshot release. While it is
    /// above zero, `tick_jog` keeps the touch flag set so the firmware sees the
    /// flick over several MISO polls. [`Self::jog_apply_impulse`] sets it when
    /// the anchor is in the scratch zone, and each tick subtracts `dt`. A
    /// slingshot from the grip zone (a bend) does not set it.
    jog_release_touch_pulse_remaining_sec: f32,
    /// Seconds left on the scroll hold. While it is above zero, the scroll's jog
    /// mode stays active.
    jog_scroll_hold_sec: f32,
    /// Whether `jog_touch` currently reports a touch.
    jog_touch_active: bool,
    /// JOG ADJUST rotary value in [0, 1]: `0` is LIGHT (long coast) and `1` is
    /// HEAVY (quick brake).
    jog_adjust: f32,
    jog_screen_popped: bool,
    main_screen_popped: bool,
    debug_screen_popped: bool,
    rotary: u16,
    /// The fader travel, in `0.0..=1.0`. Each player's encoder maps it to its
    /// own slider value; the CDJ-3000 uses `cdj3k::tempo_raw`.
    tempo: f32,
    vinyl_speed: u8,
    direction: cdj3k_emu_panel::Direction,

    lcd_touch: Option<(u16, u16)>,
    /// `true` while `CDJ3K_SCRIPT` holds a touch. The slate and the popout
    /// report their pointer state every frame, which would otherwise overwrite
    /// the scripted coordinate before the guest samples it.
    script_touch: bool,
    /// `true` while a Ctrl+click latch is active; the touch coordinate stays
    /// fixed until Ctrl is released.
    lcd_touch_ctrl_latched: bool,
    /// `true` after a scroll on the LCD, so the next click presses the rotary
    /// and does not touch the screen. Any pointer movement clears it.
    lcd_nav_mode: bool,

    /// The button the pointer holds down, released on pointer-up.
    held_btn: Option<(usize, u8)>,
    /// Buttons latched by Ctrl+click. They stay pressed until Ctrl is released.
    latched_btns: HashSet<(usize, u8)>,

    nav_angle: f32,
    /// Scroll distance below one detent, in pixels. Every
    /// `NAV_SCROLL_PX_PER_TICK` pixels send one rotary tick.
    nav_scroll_accum: f32,
    jog_adjust_scroll_accum: f32,
    vinyl_scroll_accum: f32,
    /// How far the BEAT LOOP encoder has turned. `beat_loop_scroll_accum`
    /// holds its scroll distance below one detent, in pixels.
    beat_loop_angle: f32,
    beat_loop_scroll_accum: f32,
    /// The BEAT LOOP encoder's absolute counter, sent in the frame.
    beat_loop: u16,
    /// Whether the deck is shown tilted with its front face in view. Only a
    /// slate with a front face tilts.
    tilted: bool,
    tilt_input: Option<tilt_input::TiltInput>,
    /// The tilt this frame, from 0 (flat) to 1, set by the slate as it draws.
    /// It is part of the bloom scene key, because the front face's lamps move
    /// with it.
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
    /// The label fills of SLIP, VINYL, SYNC and MASTER, sampled from the
    /// corners of the last jog frame.
    jog_corner_label_colors: [Color32; 4],

    /// Latest LED state received from the guest (updated on each new frame).
    led_state: LedState,

    bloom: Option<bloom::SharedBloom>,
    /// The LCD screen rects captured each frame, in egui coordinates with Y
    /// down. They feed the bloom exclusion mask.
    bloom_excludes: Vec<egui::Rect>,
    /// Controls drawn over the screens in [`Self::bloom_excludes`].
    bloom_keeps: Vec<bloom::BloomKeep>,
    status: String,

    /// The cached static jog wheel geometry: the outer rings, the inner disk and
    /// the knob. It is rebuilt only when the window scale or `jog_adjust`
    /// changes.
    jog_static_cache: Option<ui::draw_cache::JogStaticCache>,

    caches: ui::draw_cache::ShapeCaches,

    fps_smooth: f32,
    /// The state that keeps the window at the chassis aspect ratio from one
    /// frame to the next.
    resize: cdj3k_emu_platform::desktop::ResizeState,
    /// The number of shapes sent to the egui painter in the current frame,
    /// reset on each update.
    frame_shape_count: u64,
    /// The last MISO frame sent to the device. The debug view shows its first
    /// 32 bytes.
    last_miso: [u8; miso_frame::MISO_SIZE],
    debug: DebugView,

    /// When QEMU was last seen transitioning to running. The boot overlay
    /// clears on the first non-zero panel frame, or `BOOT_SHADE_TIMEOUT`
    /// after this.
    qemu_running_since: Option<Instant>,
    qemu_was_running: bool,
    /// The current alpha of the boot and idle shade, in [0, 1]. It ramps
    /// linearly toward the target each frame, so the overlay fades in or out
    /// over 1 s.
    shade_alpha: f32,
    /// `true` until the boot guard clears, while `target_alpha` is above zero.
    /// The main LCD and jog LCD textures are blank while it is set.
    lcds_blanked: bool,
    /// Set when QEMU exits, so the next paint clears the main and jog GL
    /// textures. Otherwise the popouts would keep showing the last frame.
    lcd_textures_need_blank: bool,
}

/// The main and jog LCDs' GL textures, owned here rather than by egui's
/// texture manager.
#[derive(Default)]
struct LcdTextures {
    /// The raw GL texture handle. It is created once and reused for every
    /// dirty upload.
    main_gl: Option<glow::Texture>,
    /// The egui TextureId that `register_native_glow_texture` returns, for use
    /// with `painter.image`.
    main_id: Option<egui::TextureId>,
    /// Set by a model switch whose main LCD size differs; the next upload
    /// re-specifies the texture storage.
    main_stale: bool,
    /// The GL texture for the jog LCD. It is created once with
    /// `TEXTURE_SWIZZLE_R = BLUE`, `TEXTURE_SWIZZLE_B = RED` and
    /// `TEXTURE_SWIZZLE_A = ONE`, so the GPU samples the wire-format XRGB bytes
    /// as RGBA with an alpha of 1.
    jog_gl: Option<glow::Texture>,
    jog_id: Option<egui::TextureId>,
}

/// What the debug viewport shows, and the handles it shares with the panel.
struct DebugView {
    jog_source: &'static str,
    jog_delta_rad: f32,
    jog_dt: f32,
    jog_omega_sample: f32,
    /// Three centred lines drawn above the MISO dump.
    jog_lines: [String; 3],
    /// A snapshot of the state the debug viewport shows, refreshed on each
    /// main update. The deferred debug viewport reads it, because it runs its
    /// own update cycle and cannot borrow `&self`.
    snapshot: Arc<Mutex<ui::DebugSnapshot>>,
    viewport_state: Arc<Mutex<DebugViewportState>>,
    /// Set by the deferred debug viewport when its window's × is clicked.
    wants_close: Arc<std::sync::atomic::AtomicBool>,
}

/// Holds the puffin HTTP server for the life of the process when profiling is
/// enabled. The server must outlive `CdjApp::new`, or no client can connect.
/// It is empty when the app starts without `--profile` or `CDJ3K_PROFILE=1`.
static PUFFIN_SERVER: std::sync::OnceLock<puffin_http::Server> = std::sync::OnceLock::new();

/// Turn puffin's scope recording on and serve it on [`PUFFIN_SERVER_ADDR`].
fn start_profiler() {
    if let Ok(server) = puffin_http::Server::new(PUFFIN_SERVER_ADDR) {
        let _ = PUFFIN_SERVER.set(server);
        eprintln!("cdj3k-emu: the puffin server is listening on {PUFFIN_SERVER_ADDR}");
    } else {
        eprintln!("cdj3k-emu: the puffin server could not bind to {PUFFIN_SERVER_ADDR}");
    }
    puffin::set_scopes_on(true);
}

/// Set up what the panel needs before its first frame, and return the slot's
/// saved panel settings.
fn prepare_panel(egui_ctx: &egui::Context) -> cdj3k_emu_storage::PanelSettings {
    // Open the trackpad haptic actuator through the private MultitouchSupport
    // API. It does nothing, and logs nothing, if the device has no actuator or
    // the private macOS struct layout has changed.
    cdj3k_emu_platform::haptic::init();

    // Disable egui's keyboard zoom and keep the zoom factor at 1.
    egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
    egui_ctx.set_zoom_factor(1.0);
    let instance_id = cdj3k_emu_platform::menu_state::lock().current_instance_id;
    let panel = cdj3k_emu_storage::PanelSettings::load(instance_id);
    // The menu owns the screen-extend flag; restore it before the first frame.
    cdj3k_emu_platform::menu_state::lock().screen_extended = panel.screen_extended;
    panel
}

impl CdjApp {
    /// When `profile` is true, this turns on puffin scope recording and binds a
    /// localhost TCP listener for `puffin_viewer`. When it is false, which is
    /// the default, every `puffin::profile_*` macro costs one relaxed atomic
    /// load and no TCP port is opened.
    pub fn new(model: Model, socket_dir: String, egui_ctx: egui::Context, profile: bool) -> Self {
        if profile {
            start_profiler();
        }
        let panel = prepare_panel(&egui_ctx);
        let repaint_gate = RepaintGate::new(egui_ctx.clone(), MIN_FRAME_INTERVAL);
        // `--no-spawn` never shows the boot shade, so the shade must not fade
        // from opaque either; chassis captures then match from frame to frame.
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

    /// Point the panel at another model. The streams, GL textures and jog
    /// physics carry over. Every shape cache is dropped, because the caches
    /// are keyed on the window scale only.
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
        // The window changes shape for the new canvas, so the aspect snap
        // starts again.
        self.resize = Default::default();
    }

    /// Called when the panel is hidden and the picker is shown. Releases any
    /// held or latched control and drops the tilt's pointer map, which
    /// describes the window as it was last drawn.
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
    /// this key is unchanged from the previous frame and skips the GL work:
    /// the blit, the threshold and the 9-tap separable blur.
    pub(super) fn bloom_scene_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.led_state.frame.hash(&mut h);
        // HashSet iteration order varies, so the latched set is sorted in a
        // stack array before hashing. The key then stays stable across frames
        // without allocating.
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
        // Inject the power-off MISO frame here. On app exit the egui loop has
        // already stopped, so `poll_menu_state` never consumes
        // POWER_OFF_STIMULI_REQUESTED, and the guest would never see the signal
        // during the 8 s EP122 wait in `instance.stop()`.
        self.inject(self.power_off_frame());

        // Mark shutdown so the runtime worker sees it at the top of its next
        // loop iteration and runs `instance.stop()`. That sends
        // POWER_OFF_STIMULI, then ACPI system_powerdown, then QMP quit, each
        // step with its own watchdog.
        cdj3k_emu_platform::menu_state::APP_SHUTDOWN
            .store(true, std::sync::atomic::Ordering::Relaxed);

        // Save GUI-side state in parallel with the worker's shutdown.
        if let (Some(gl), Some(bloom)) = (gl, &self.bloom) {
            bloom.lock().unwrap().destroy(gl);
        }
        // Lock the menu state once; locking it twice in one expression
        // deadlocks on its mutex.
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

        // Wait for the worker to finish its graceful QEMU stop. The budget
        // covers `instance.stop()`: 8 s for the EP122 cleanup, 20 s for the
        // ACPI shutdown and 5 s for QMP quit, plus a small margin.
        let joined = cdj3k_emu_runtime::wait_for_worker(SHUTDOWN_WATCHDOG);
        if !joined {
            // The watchdog elapsed, so fall back to SIGTERM and SIGKILL on the
            // QEMU child. The log line shows when the soft path times out.
            eprintln!(
                "cdj3k-emu: the soft shutdown watchdog ({}s) elapsed; sending SIGTERM, then SIGKILL",
                SHUTDOWN_WATCHDOG.as_secs()
            );
            cdj3k_emu_runtime::kill_qemu_child();
        }
        cdj3k_emu_runtime::cleanup_runtime_files();
        cdj3k_emu_platform::haptic::shutdown();
    }
}

/// The longest time the runtime worker gets for the graceful QEMU stop before
/// the app falls back to SIGTERM and SIGKILL. It covers the 8, 20 and 5 s steps
/// of `instance.stop()` plus loop overhead.
const SHUTDOWN_WATCHDOG: std::time::Duration = std::time::Duration::from_secs(35);

impl CdjApp {
    /// The reference canvas of the active slate. The window keeps its aspect
    /// ratio.
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
