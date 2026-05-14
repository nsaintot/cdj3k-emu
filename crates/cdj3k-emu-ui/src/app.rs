mod bloom;
mod boot_overlay;
mod buttons;
mod firmware_wizard;
mod frame_inject;
mod jog_physics;
mod lcd_texture;
mod lcd_touch;
mod ui;
mod viewports;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use egui::Color32;

use cdj3k_emu_platform::menu_state;
use cdj3k_emu_streams::ctrl_stream::{CtrlStream, LedState};
use cdj3k_emu_streams::jog_stream::JogLcdStream;
use cdj3k_emu_streams::main_stream::MainLcdStream;
use cdj3k_emu_streams::RepaintGate;
use cdj3k_emu_subucom::miso_frame::{self};

use firmware_wizard::FirmwareWizard;
use jog_physics::JOG_OMEGA_MOVING_THRESHOLD;
use viewports::DebugViewportState;

pub(crate) use lcd_touch::LcdTouchCapture;

/// Minimum wall-clock interval between presented frames - caps the main
/// viewport at ~60 fps regardless of vsync rate or how many sources
/// requested a repaint.
pub(crate) const MIN_FRAME_INTERVAL: Duration = Duration::from_micros(16_667);

/// Fresh main-LCD generations to observe after QEMU starts before the boot
/// spinner overlay is removed.
const BOOT_FRAMES_THRESHOLD: u32 = 15;

/// Frames the inner window size must remain stable before the aspect-snap
/// fires (so brief pauses during a drag don't trigger a mid-drag resize).
const STABLE_FRAMES_REQUIRED: u32 = 8;

/// Movement threshold (px²) below which a frame is considered "stable" for
/// the aspect-snap detector.
const STABLE_SIZE_TOL_SQ: f32 = 0.25;

/// Aspect-ratio tolerance before the snap fires (relative).
const ASPECT_SNAP_TOL: f32 = 0.001;

/// EMA factor for the smoothed FPS estimate.
const FPS_EMA_ALPHA: f32 = 0.1;

/// Repaint cadence requested by UI animation (60 Hz cap).
const ANIM_REPAINT_INTERVAL: Duration = Duration::from_millis(16);

/// Linear ramp speed for the boot/idle shade alpha (units of alpha per
/// second; 1.0 here means the full 0→1 fade takes 1 s).
const SHADE_FADE_SPEED: f32 = 1.0;

/// `jog_vel` neutral-position encoding (inverse: max u16 = stopped).
const JOG_VEL_INIT: u16 = 0xffff;

/// `rotary` neutral-position encoding (LE: +1 = 0x0000, −1 = 0xfeff).
const ROTARY_NEUTRAL: u16 = 0xffff;

/// Initial tempo slider position (0.0 = full minus, 0.5 = centre, 1.0 = full plus).
const TEMPO_INIT: f32 = 0.5;

/// Puffin profiling HTTP server address. Localhost-only so an enabled
/// profiling build doesn't expose the endpoint on any host interface.
const PUFFIN_SERVER_ADDR: &str = "127.0.0.1:8585";

pub struct CdjApp {
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
    /// `0.0..=1.0`, piecewise-mapped to `0x0000..=0xFFFF` (centre → `TEMPO_CENTER`).
    tempo: f32,
    vinyl_speed: u8,
    direction: cdj3k_emu_subucom::Direction,

    lcd_touch: Option<(u16, u16)>,
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

    ctrl_stream: CtrlStream,

    display_stream: MainLcdStream,
    /// Raw GL texture handle - created once, re-used for every dirty upload.
    display_gl_tex: Option<glow::Texture>,
    /// egui TextureId returned by `register_native_glow_texture` - used with `painter.image`.
    display_tex_id: Option<egui::TextureId>,

    jog_stream: JogLcdStream,
    /// GL texture for the jog LCD - owned by us (not egui's tex_manager). Created once with
    /// `TEXTURE_SWIZZLE_R = BLUE`, `TEXTURE_SWIZZLE_B = RED`, `TEXTURE_SWIZZLE_A = ONE` so
    /// the GPU samples the wire-format XRGB bytes as RGBA(A=1) for free.
    jog_gl_tex: Option<glow::Texture>,
    jog_tex_id: Option<egui::TextureId>,
    /// Corner samples from the last jog frame: SLIP, VINYL, SYNC, MASTER label fills.
    jog_corner_label_colors: [Color32; 4],

    /// Latest LED state received from the guest (updated on each new frame).
    led_state: LedState,

    bloom: Option<bloom::SharedBloom>,
    /// LCD screen rects captured each frame (egui coords, Y-down) - fed to the bloom exclusion mask.
    bloom_excludes: Vec<egui::Rect>,

    status: String,

    /// Cached static jog wheel geometry (outer rings + inner disk/knob).
    /// Rebuilt only when the window scale or `jog_adjust` changes.
    jog_static_cache: Option<ui::draw_cache::JogStaticCache>,

    // ── Shape caches ──────────────────────────────────────────────────────
    btn_cache: ui::draw_cache::BtnShapeCache,
    chassis_bg_cache: ui::draw_cache::StaticShapeCache,
    chassis_lcd_overlay_cache: ui::draw_cache::StaticShapeCache,
    top_statics_cache: ui::draw_cache::StaticShapeCache,
    left_statics_cache: ui::draw_cache::StaticShapeCache,
    right_statics_cache: ui::draw_cache::StaticShapeCache,
    jog_statics_cache: ui::draw_cache::StaticShapeCache,
    jog_ring_lights_cache: ui::draw_cache::ShapeCache<(i32, i32, i32, u8, bool)>,
    jog_corner_labels_cache: ui::draw_cache::ShapeCache<(i32, i32, i32, i32, [u8; 16])>,
    grip_cache: ui::draw_cache::ShapeCache<(i32, i32, i32, i32)>,

    fps_smooth: f32,
    /// Previous frame's inner window size. Used to detect "stable" (not mid-drag)
    /// so the aspect-ratio snap only fires after a resize settles.
    last_inner_size: egui::Vec2,
    stable_frames: u32,
    /// Shapes submitted to the egui painter in the current frame (reset each update).
    frame_shape_count: u64,
    /// Last MISO frame sent to the device (first 32 bytes displayed for debugging).
    last_miso: [u8; miso_frame::MISO_SIZE],
    jog_dbg_last_source: &'static str,
    jog_dbg_last_delta_rad: f32,
    jog_dbg_last_dt: f32,
    jog_dbg_last_omega_sample: f32,
    /// Rendered above the MISO dump (3 centred lines).
    jog_dbg_lines: [String; 3],

    wizard: FirmwareWizard,

    /// Frame count from `MainLcdStream` at the moment QEMU was last seen
    /// transitioning to running. The boot overlay clears once
    /// `frames_seen() - frame_baseline_at_boot >= BOOT_FRAMES_THRESHOLD`.
    frame_baseline_at_boot: u32,
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

    /// Snapshot of debug-relevant state, refreshed each main update. Read by
    /// the deferred debug viewport (which runs in its own update cycle and
    /// therefore cannot borrow `&self`).
    debug_snapshot: Arc<Mutex<ui::DebugSnapshot>>,
    debug_viewport_state: Arc<Mutex<DebugViewportState>>,
    /// Set by the deferred debug viewport when its window's × is clicked.
    debug_wants_close: Arc<std::sync::atomic::AtomicBool>,
    /// `true` once the user has triggered a close (red button / Cmd-Q / menu).
    /// While set, we cancel the actual window close each frame and keep
    /// painting so the boot shade can fade in over the going-away UI; we
    /// re-issue the close once the runtime worker has finished its graceful
    /// stop, at which point `on_exit` runs without a multi-second freeze.
    shutdown_in_progress: bool,
}

/// Owns the puffin HTTP server for the life of the process when profiling is
/// enabled.  The server must outlive `CdjApp::new` (otherwise no client can
/// connect) but never needs to be reclaimed - storing it here makes the
/// lifetime explicit instead of via `mem::forget`.  Empty when launched
/// without `--profile` / `CDJ3K_PROFILE=1`.
static PUFFIN_SERVER: std::sync::OnceLock<puffin_http::Server> = std::sync::OnceLock::new();

impl CdjApp {
    /// `profile`: when true, enables puffin scope recording and binds a
    /// localhost TCP listener for `puffin_viewer` to connect to.  When
    /// false (shipping default), every `puffin::profile_*` macro becomes
    /// a single relaxed-atomic load and no TCP port is opened.
    pub fn new(socket_dir: String, egui_ctx: egui::Context, profile: bool) -> Self {
        if profile {
            if let Ok(server) = puffin_http::Server::new(PUFFIN_SERVER_ADDR) {
                let _ = PUFFIN_SERVER.set(server);
                eprintln!("cdj3k-emu: puffin server listening on {PUFFIN_SERVER_ADDR}");
            } else {
                eprintln!("cdj3k-emu: puffin server failed to bind {PUFFIN_SERVER_ADDR}");
            }
            puffin::set_scopes_on(true);
        }

        // Open the trackpad haptic actuator (private MultitouchSupport API).
        // No-op + silent failure if the device has no actuator or the macOS
        // private struct layout has drifted.
        cdj3k_emu_platform::haptic::init();

        // Prevent resizing the window with the keyboard.
        egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        egui_ctx.set_zoom_factor(1.0);
        let app_settings = cdj3k_emu_storage::AppSettings::load();
        let repaint_gate = RepaintGate::new(egui_ctx.clone(), MIN_FRAME_INTERVAL);
        Self {
            jog_pos: 0,
            jog_vel: JOG_VEL_INIT,
            jog_touch: 0x00,
            jog_angle: 0.0,
            jog_accum: 0.0,
            jog_angle_last_encoded: 0.0,
            jog_haptic_accum: 0.0,
            jog_omega: 0.0,
            jog_drag_prev_angle: None,
            jog_slingshot_anchor: None,
            jog_grip_drag: false,
            jog_release_touch_pulse_remaining_sec: 0.0,
            jog_scroll_hold_sec: 0.0,
            jog_touch_active: false,
            jog_adjust: app_settings.jog_adjust,
            jog_screen_popped: false,
            main_screen_popped: false,
            debug_screen_popped: false,
            rotary: ROTARY_NEUTRAL,
            tempo: TEMPO_INIT,
            vinyl_speed: app_settings.vinyl_speed,
            direction: cdj3k_emu_subucom::Direction::Forward,
            lcd_touch: None,
            lcd_touch_ctrl_latched: false,
            lcd_nav_mode: false,
            held_btn: None,
            latched_btns: HashSet::new(),
            nav_angle: 0.0,
            nav_scroll_accum: 0.0,
            jog_adjust_scroll_accum: 0.0,
            vinyl_scroll_accum: 0.0,
            ctrl_stream: CtrlStream::new(&socket_dir, repaint_gate.clone()),
            display_stream: MainLcdStream::new(&socket_dir, repaint_gate.clone()),
            display_gl_tex: None,
            display_tex_id: None,
            jog_stream: JogLcdStream::new(&socket_dir, repaint_gate.clone()),
            jog_gl_tex: None,
            jog_tex_id: None,
            jog_corner_label_colors: [ui::COL_BTN_TEXT; 4],
            led_state: LedState::default(),
            bloom: None,
            bloom_excludes: Vec::new(),
            status: "Starting...".to_owned(),
            jog_static_cache: None,
            btn_cache: ui::draw_cache::BtnShapeCache::new(),
            chassis_bg_cache: ui::draw_cache::StaticShapeCache::new(),
            chassis_lcd_overlay_cache: ui::draw_cache::StaticShapeCache::new(),
            top_statics_cache: ui::draw_cache::StaticShapeCache::new(),
            left_statics_cache: ui::draw_cache::StaticShapeCache::new(),
            right_statics_cache: ui::draw_cache::StaticShapeCache::new(),
            jog_statics_cache: ui::draw_cache::StaticShapeCache::new(),
            jog_ring_lights_cache: ui::draw_cache::ShapeCache::new(),
            jog_corner_labels_cache: ui::draw_cache::ShapeCache::new(),
            grip_cache: ui::draw_cache::ShapeCache::new(),
            fps_smooth: 60.0,
            last_inner_size: egui::Vec2::ZERO,
            stable_frames: 0,
            frame_shape_count: 0,
            last_miso: [0u8; miso_frame::MISO_SIZE],
            jog_dbg_last_source: "none",
            jog_dbg_last_delta_rad: 0.0,
            jog_dbg_last_dt: 0.0,
            jog_dbg_last_omega_sample: 0.0,
            jog_dbg_lines: [String::new(), String::new(), String::new()],
            wizard: FirmwareWizard::new(),
            frame_baseline_at_boot: 0,
            qemu_was_running: false,
            shade_alpha: 1.0,
            lcds_blanked: true,
            lcd_textures_need_blank: false,
            debug_snapshot: Arc::new(Mutex::new(ui::DebugSnapshot::default())),
            debug_viewport_state: Arc::new(Mutex::new(DebugViewportState::default())),
            debug_wants_close: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            shutdown_in_progress: false,
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
        h.finish()
    }
}

static MENU_SETUP: std::sync::Once = std::sync::Once::new();

impl eframe::App for CdjApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // No sleep here. With vsync disabled in NativeOptions, eframe only
        // paints when something requested a repaint - the gate's metronome
        // thread paces those requests so this `update()` runs at most ~60 Hz.
        // Skip the global-profiler mutex hit entirely when profiling is off.
        // `set_scopes_on` defaults to false in shipping builds (we only flip
        // it via `--profile` / `CDJ3K_PROFILE=1` in `CdjApp::new`).
        if puffin::are_scopes_on() {
            puffin::GlobalProfiler::lock().new_frame();
        }

        // Graceful close: instead of blocking inside `on_exit` for the full
        // runtime-stop budget (which freezes the window for 1-2 s), defer
        // the actual close until the worker has finished.  On the first
        // close request we set APP_SHUTDOWN + shade_forced, cancel the
        // close so egui keeps painting (letting the shade fade in over the
        // going-away UI), and re-issue the close once the worker is done.
        if ctx.input(|i| i.viewport().close_requested()) {
            if !self.shutdown_in_progress {
                self.shutdown_in_progress = true;
                cdj3k_emu_platform::menu_state::APP_SHUTDOWN
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                cdj3k_emu_platform::menu_state::lock().shade_forced = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else if !cdj3k_emu_runtime::worker_is_finished() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
            // else: worker is done - let the close proceed into on_exit.
        }

        if self.shutdown_in_progress {
            if cdj3k_emu_runtime::worker_is_finished() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                ctx.request_repaint();
            }
        }

        #[cfg(target_os = "macos")]
        cdj3k_emu_platform::desktop::apply_macos_resize_constraints_from_frame(frame);

        self.snap_window_aspect(ctx);

        MENU_SETUP.call_once(cdj3k_emu_platform::menu::setup_menu);
        self.poll_menu_state();

        // Blank LCD textures after a QEMU exit so popouts go black instead
        // of holding the last captured frame.
        if self.lcd_textures_need_blank {
            if let Some(gl) = frame.gl().cloned() {
                self.blank_lcd_textures(&gl);
                self.lcd_textures_need_blank = false;
            }
        }

        if let Some(dirty) = self.display_stream.take() {
            puffin::profile_scope!("texture_upload_display");
            self.upload_display_dirty(frame, &dirty);
            // Init bloom pipeline once we have a GL context.
            if self.bloom.is_none() {
                if let Some(gl) = frame.gl() {
                    self.bloom = Some(Arc::new(Mutex::new(bloom::BloomPipeline::new(gl))));
                }
            }
        }

        if let Some(jf) = self.jog_stream.take() {
            puffin::profile_scope!("texture_upload_jog");
            // corner_rgba is already swapped to RGBA by the stream's `sample_corner_rgba`.
            self.jog_corner_label_colors = std::array::from_fn(|i| {
                Color32::from_rgba_unmultiplied(
                    jf.corner_rgba[i][0],
                    jf.corner_rgba[i][1],
                    jf.corner_rgba[i][2],
                    jf.corner_rgba[i][3],
                )
            });
            self.upload_jog_dirty(frame, &jf);
        }

        let disp_state = stream_state_label(
            self.display_stream.is_connected(),
            self.display_tex_id.is_some(),
            "lcd",
        );
        let jog_state = stream_state_label(
            self.jog_stream.is_connected(),
            self.jog_tex_id.is_some(),
            "jog",
        );

        if let Some(ls) = {
            puffin::profile_scope!("ctrl_take");
            self.ctrl_stream.take()
        } {
            self.led_state = ls;
        }

        let dt = ctx.input(|i| i.stable_dt.max(1.0e-4));
        self.fps_smooth = FPS_EMA_ALPHA * (1.0 / dt) + (1.0 - FPS_EMA_ALPHA) * self.fps_smooth;

        {
            puffin::profile_scope!("tick_jog");
            self.tick_jog(dt);
        }

        // Release all ctrl-latched buttons when ctrl is no longer held.
        let ctrl = ctx.input(|i| i.modifiers.ctrl);
        if !ctrl && !self.latched_btns.is_empty() {
            puffin::profile_scope!("latched_clear_inject");
            self.latched_btns.clear();
            self.inject(self.build_current_frame().finalize());
        }

        {
            puffin::profile_scope!("status_fmt");
            self.status = format!(
                "{}  {}  adj={:.2} ω={:+.2}rad/s",
                disp_state, jog_state, self.jog_adjust, self.jog_omega
            );
        }

        // Schedule repaints only for pure UI animation - stream threads own
        // the wakeups for new_display/new_jog/new_led via their own
        // `request_repaint_after`. Echoing those here would create a
        // feedback loop (data arrives → stream wakes UI → update schedules
        // another repaint 16 ms later → extra frame with no new data).
        let jog_motion = self.jog_omega.abs() > JOG_OMEGA_MOVING_THRESHOLD
            || self.jog_is_dragging()
            || self.jog_scroll_hold_sec > 0.0;
        if jog_motion || self.held_btn.is_some() || !self.latched_btns.is_empty() {
            ctx.request_repaint_after(ANIM_REPAINT_INTERVAL);
        }

        let (booting, target_alpha) = self.tick_boot_shade(ctx);
        self.lcds_blanked = target_alpha > 0.0;

        // ── Chrome ────────────────────────────────────────────────────────
        // egui is immediate-mode: shapes must be re-submitted each update to
        // stay visible. The shape caches make this cheap when nothing changes.
        self.bloom_excludes.clear();
        {
            puffin::profile_scope!("draw_chrome");
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(ui::panel_bg()))
                .show(ctx, |ui| {
                    self.draw_ui(ui);
                });
        }

        if self.shade_alpha > 0.0 {
            puffin::profile_scope!("shade_draw");
            boot_overlay::paint_boot_shade(ctx, self.shade_alpha, booting);
        }

        // Refresh the snapshot consumed by the deferred debug viewport.
        // Cheap (a handful of clones + array copies); kept outside the
        // secondary-viewports gate so the snapshot stays current even while booting.
        if let Ok(mut g) = self.debug_snapshot.lock() {
            *g = self.debug_snapshot();
        }

        // The wizard is exempt from the shade gate - it's the tool that
        // provisions firmware, so it must be reachable precisely when QEMU
        // isn't running.
        self.wizard.show(ctx);

        // Secondary viewports: stay hidden until the shade is fully gone -
        // they aren't covered by the shade overlay.
        if self.shade_alpha == 0.0 && target_alpha == 0.0 {
            puffin::profile_scope!("secondary_viewports");
            self.show_debug_viewport(ctx);
            self.show_jog_viewport(ctx);
            self.show_main_viewport(ctx);
        }

        // Mirror viewport state back to shared state so the menu reflects close
        // events triggered by clicking ×.
        {
            let mut s = menu_state::lock();
            s.jog_screen_popped = self.jog_screen_popped;
            s.main_screen_popped = self.main_screen_popped;
            s.debug_screen_popped = self.debug_screen_popped;
        }

        cdj3k_emu_platform::menu::sync_menu();

        // Bloom composite - fires last; thresholds + blurs scene_tex, writes
        // to screen.
        self.queue_bloom_pass(ctx);
    }

    fn on_exit(&mut self, gl: Option<&glow::Context>) {
        // Inject the power-off MISO frame directly: on app exit the egui loop
        // is already stopped, so `poll_menu_state` will never consume
        // POWER_OFF_STIMULI_REQUESTED. Without this, instance.stop()'s 8 s
        // EP122 wait elapses with the guest never having seen the signal.
        let mut f = self.build_current_frame();
        f.set_power(false);
        self.inject(f.finalize());

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
        let _ = cdj3k_emu_storage::AppSettings {
            jog_adjust: self.jog_adjust,
            vinyl_speed: self.vinyl_speed,
        }
        .save();

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
    /// Snap the inner window aspect to the layout reference once a resize
    /// has settled. `setContentAspectRatio` only constrains *future*
    /// resizes; if the window comes up at the wrong aspect (e.g. autosaved
    /// from a prior layout), it stays letterboxed without this.
    fn snap_window_aspect(&mut self, ctx: &egui::Context) {
        use cdj3k_emu_platform::desktop::{LAYOUT_REF_H, LAYOUT_REF_W};
        let size = ctx.screen_rect().size();
        if (size - self.last_inner_size).length_sq() < STABLE_SIZE_TOL_SQ {
            self.stable_frames = self.stable_frames.saturating_add(1);
        } else {
            self.stable_frames = 0;
        }
        self.last_inner_size = size;
        if self.stable_frames < STABLE_FRAMES_REQUIRED {
            return;
        }
        let target_aspect = LAYOUT_REF_W / LAYOUT_REF_H;
        let current_aspect = size.x / size.y;
        if (current_aspect - target_aspect).abs() > ASPECT_SNAP_TOL {
            let new_w = size.y * target_aspect;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::Vec2::new(
                new_w, size.y,
            )));
            self.stable_frames = 0;
        }
    }

    /// Pull menu state (set by the native macOS menu or the egui menu).
    /// Also handles power-off and wizard-open requests.
    fn poll_menu_state(&mut self) {
        let (jog, main, debug, want_wizard, want_poweroff) = {
            let mut s = menu_state::lock();
            (
                s.jog_screen_popped,
                s.main_screen_popped,
                s.debug_screen_popped,
                std::mem::take(&mut s.firmware_wizard_requested),
                std::mem::take(&mut s.power_off_stimuli_requested),
            )
        };
        self.jog_screen_popped = jog;
        self.main_screen_popped = main;
        self.debug_screen_popped = debug;
        if want_wizard {
            self.wizard.open = true;
        }
        if want_poweroff {
            let mut f = self.build_current_frame();
            f.set_power(false);
            self.inject(f.finalize());
        }
    }

    /// Step the boot/idle shade alpha toward its target. Returns `(booting,
    /// target_alpha)` for downstream gating.
    fn tick_boot_shade(&mut self, ctx: &egui::Context) -> (bool, f32) {
        let (qemu_running, shade_forced) = {
            let s = menu_state::lock();
            (s.qemu_running, s.shade_forced)
        };
        if qemu_running && !self.qemu_was_running {
            self.frame_baseline_at_boot = self.display_stream.frames_seen();
        }
        // QEMU just exited: blank LCD textures so popout windows go black.
        let qemu_just_exited = self.qemu_was_running && !qemu_running;
        self.qemu_was_running = qemu_running;
        if qemu_just_exited {
            self.lcd_textures_need_blank = true;
        }

        let frames_since_boot = self
            .display_stream
            .frames_seen()
            .saturating_sub(self.frame_baseline_at_boot);
        let booting = (qemu_running && frames_since_boot < BOOT_FRAMES_THRESHOLD) || shade_forced;
        let target_alpha: f32 = if !qemu_running || booting { 1.0 } else { 0.0 };

        // Linear ramp toward target (configurable speed).
        let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 0.1);
        let prev_alpha = self.shade_alpha;
        if (target_alpha - prev_alpha).abs() > f32::EPSILON {
            let step = dt * SHADE_FADE_SPEED;
            self.shade_alpha = if target_alpha > prev_alpha {
                (prev_alpha + step).min(target_alpha)
            } else {
                (prev_alpha - step).max(target_alpha)
            };
            ctx.request_repaint_after(ANIM_REPAINT_INTERVAL);
        }
        (booting, target_alpha)
    }

    /// Append a deferred bloom pass to the topmost layer (runs after all
    /// chrome has been submitted).
    fn queue_bloom_pass(&self, ctx: &egui::Context) {
        let Some(bloom_arc) = self.bloom.clone() else {
            return;
        };
        let excludes = self.bloom_excludes.clone();
        let scene_key = self.bloom_scene_key();
        let cb = eframe::egui_glow::CallbackFn::new(move |info, painter| {
            puffin::profile_scope!("bloom_gl");
            let [w, h] = info.screen_size_px;
            // Lock-poison fallback: if a previous bloom run panicked, just
            // skip this frame's bloom rather than propagating the panic
            // through the GL callback (which would tear down the renderer).
            let Ok(mut bloom) = bloom_arc.lock() else {
                return;
            };
            bloom.run(
                painter.gl(),
                w as i32,
                h as i32,
                info.pixels_per_point,
                &excludes,
                scene_key,
            );
        });
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Debug,
            egui::Id::new("bloom_pass"),
        ))
        .add(egui::Shape::Callback(egui::PaintCallback {
            rect: ctx.screen_rect(),
            callback: Arc::new(cb),
        }));
    }
}

fn stream_state_label(connected: bool, has_tex: bool, prefix: &'static str) -> String {
    let suffix = match (connected, has_tex) {
        (true, true) => "OK",
        (true, false) => "conn",
        _ => "wait",
    };
    format!("{prefix}:{suffix}")
}
