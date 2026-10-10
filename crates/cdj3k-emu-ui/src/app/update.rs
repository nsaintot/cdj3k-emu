//! Runs one panel frame: it reads the streams, ticks the physics, draws the
//! slate, the shade and the pop-outs, and queues the bloom pass.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::Color32;

use cdj3k_emu_platform::menu_state;
use cdj3k_emu_streams::ctrl_stream::LedState;

use super::jog_physics::JOG_OMEGA_MOVING_THRESHOLD;
use super::{bloom, boot_overlay, ui, CdjApp};

/// The panel frame is all zeroes until the guest drives it. If the guest
/// never does, drop the shade this long after QEMU starts.
const BOOT_SHADE_TIMEOUT: Duration = Duration::from_secs(5);

/// EMA factor for the smoothed FPS estimate.
const FPS_EMA_ALPHA: f32 = 0.1;

/// The repaint interval that UI animation requests, which caps it at 60 Hz.
const ANIM_REPAINT_INTERVAL: Duration = Duration::from_millis(16);

/// Linear ramp speed of the boot and idle shade alpha, in alpha units per
/// second. At 1.0, a full fade from 0 to 1 takes 1 s.
const SHADE_FADE_SPEED: f32 = 1.0;

impl CdjApp {
    /// Runs one panel frame: it reads the LCD and LED streams, ticks the jog
    /// physics, and draws the slate, the boot shade and the pop-out viewports.
    /// The shell calls it while the panel screen is up.
    pub fn update(&mut self, ctx: &egui::Context, root: &mut egui::Ui, frame: &mut eframe::Frame) {
        // Do not sleep here. With vsync disabled in NativeOptions, eframe
        // paints only when something requests a repaint, and the gate's
        // metronome thread paces those requests so that `update()` runs at
        // most about 60 times a second. Skip the global profiler's mutex when
        // profiling is off; `set_scopes_on` stays false unless `--profile` or
        // `CDJ3K_PROFILE=1` turns it on in `CdjApp::new`.
        if puffin::are_scopes_on() {
            puffin::GlobalProfiler::lock().new_frame();
        }

        let ref_canvas = self.ref_canvas();
        cdj3k_emu_platform::desktop::apply_resize_constraints(
            ctx,
            frame,
            &mut self.resize,
            Some(ref_canvas),
        );

        self.poll_menu_state();
        self.ingest_lcd_streams(frame);
        let disp_state = stream_state_label(
            self.display_stream.is_connected(),
            self.textures.main_id.is_some(),
            "lcd",
        );
        let jog_state = stream_state_label(
            self.jog_stream.is_connected(),
            self.textures.jog_id.is_some(),
            "jog",
        );
        self.sync_ctrl_stream();
        self.tick_input(ctx);

        {
            puffin::profile_scope!("status_fmt");
            self.status = format!(
                "{}  {}  adj={:.2} ω={:+.2}rad/s",
                disp_state, jog_state, self.jog_adjust, self.jog_omega
            );
        }

        self.schedule_animation(ctx);
        let (booting, target_alpha) = self.tick_boot_shade(ctx);
        self.lcds_blanked = target_alpha > 0.0;
        self.draw_panel(ctx, root, booting);
        self.show_popouts(ctx, target_alpha);

        // Queue the bloom composite last. It thresholds and blurs scene_tex
        // and writes the result to the screen.
        self.queue_bloom_pass(ctx);
    }

    /// Upload whatever the main and jog LCD streams brought since the last
    /// frame.
    fn ingest_lcd_streams(&mut self, frame: &mut eframe::Frame) {
        // Blank the LCD textures after QEMU exits, so the pop-outs go black
        // and do not hold the last captured frame.
        if self.lcd_textures_need_blank {
            if let Some(gl) = frame.gl().cloned() {
                self.blank_lcd_textures(&gl);
                self.lcd_textures_need_blank = false;
            }
        }

        if let Some(dirty) = self.display_stream.take() {
            puffin::profile_scope!("texture_upload_display");
            self.upload_display_dirty(frame, &dirty);
            // Create the bloom pipeline once a GL context exists.
            if self.bloom.is_none() {
                if let Some(gl) = frame.gl() {
                    self.bloom = Some(Arc::new(Mutex::new(bloom::BloomPipeline::new(gl))));
                }
            }
        }

        if let Some(jf) = self.jog_stream.take() {
            puffin::profile_scope!("texture_upload_jog");
            // The stream's `sample_corner_rgba` already gives corner_rgba in RGBA
            // order.
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
    }

    /// Send the model's frame on the control channel's rising edge, and take
    /// the guest's latest LED state.
    fn sync_ctrl_stream(&mut self) {
        // The guest's sub-CPU module serves its own idle frame until the
        // host's first one arrives: send the model's frame as soon as the
        // control channel is up.
        let ctrl_ready = self.ctrl_stream.is_ready();
        if ctrl_ready && !self.ctrl_was_ready {
            self.inject(self.build_current_frame());
        }
        self.ctrl_was_ready = ctrl_ready;

        if let Some(ls) = {
            puffin::profile_scope!("ctrl_take");
            self.ctrl_stream.take()
        } {
            self.led_state = ls;
        }
    }

    /// Step the frame clock and the jog physics, and drop Ctrl-latched
    /// buttons once Ctrl is up.
    fn tick_input(&mut self, ctx: &egui::Context) {
        let dt = ctx.input(|i| i.stable_dt.max(1.0e-4));
        self.fps_smooth = FPS_EMA_ALPHA * (1.0 / dt) + (1.0 - FPS_EMA_ALPHA) * self.fps_smooth;

        {
            puffin::profile_scope!("tick_jog");
            self.tick_jog(dt);
        }

        // Release all Ctrl-latched buttons when Ctrl is no longer held.
        let ctrl = ctx.input(|i| i.modifiers.ctrl);
        if !ctrl && !self.latched_btns.is_empty() {
            puffin::profile_scope!("latched_clear_inject");
            self.latched_btns.clear();
            self.inject(self.build_current_frame());
        }
    }

    /// Schedule repaints only for UI animation. The stream threads request
    /// their own repaints for new display, jog and LED data through
    /// `request_repaint_after`. Requesting them here as well would create a
    /// feedback loop: data arrives, the stream wakes the UI, `update`
    /// schedules another repaint 16 ms later, and an extra frame is drawn
    /// with no new data.
    fn schedule_animation(&self, ctx: &egui::Context) {
        let jog_motion = self.jog_omega.abs() > JOG_OMEGA_MOVING_THRESHOLD
            || self.jog_is_dragging()
            || self.jog_scroll_hold_sec > 0.0;
        if jog_motion || self.held_btn.is_some() || !self.latched_btns.is_empty() {
            ctx.request_repaint_after(ANIM_REPAINT_INTERVAL);
        }
    }

    /// Draws the slate and the boot shade over it.
    fn draw_panel(&mut self, ctx: &egui::Context, root: &mut egui::Ui, booting: bool) {
        // egui is immediate-mode, so shapes must be submitted again on each
        // update to stay visible. The shape caches make this cheap when
        // nothing changes.
        self.bloom_excludes.clear();
        self.bloom_keeps.clear();
        {
            puffin::profile_scope!("draw_chrome");
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(ui::panel_bg()))
                .show(root, |ui| {
                    self.draw_ui(ui);
                });
        }

        if self.shade_alpha > 0.0 {
            puffin::profile_scope!("shade_draw");
            boot_overlay::paint_boot_shade(ctx, self.shade_alpha, booting);
        }
    }

    /// Shows the debug, jog and main pop-out viewports, and mirrors their
    /// state to the menu.
    fn show_popouts(&mut self, ctx: &egui::Context, target_alpha: f32) {
        // Refresh the snapshot that the deferred debug viewport reads. This
        // is cheap: a few clones and array copies. It runs outside the
        // secondary-viewports gate so the snapshot stays current while booting.
        if let Ok(mut g) = self.debug.snapshot.lock() {
            *g = self.debug_snapshot();
        }

        // Keep the secondary viewports hidden until the shade is fully gone,
        // because the shade overlay does not cover them.
        if self.shade_alpha == 0.0 && target_alpha == 0.0 {
            puffin::profile_scope!("secondary_viewports");
            self.show_debug_viewport(ctx);
            self.show_jog_viewport(ctx);
            self.show_main_viewport(ctx);
        }

        // Mirror the viewport state back to the shared state, so the menu
        // reflects a pop-out closed with its × button.
        let mut s = menu_state::lock();
        s.jog_screen_popped = self.jog_screen_popped;
        s.main_screen_popped = self.main_screen_popped;
        s.debug_screen_popped = self.debug_screen_popped;
    }

    /// Read the menu state that the native macOS menu or the egui menu sets:
    /// the pop-out toggles and power-off requests.
    fn poll_menu_state(&mut self) {
        let (jog, main, debug) = {
            let s = menu_state::lock();
            (
                s.jog_screen_popped,
                s.main_screen_popped,
                s.debug_screen_popped,
            )
        };
        self.jog_screen_popped = jog;
        self.main_screen_popped = main;
        self.debug_screen_popped = debug;
        self.send_requested_power_off();
    }

    /// Send the power-off frame that the runtime worker asked for when it
    /// began stopping QEMU. The shell also calls this while the window is
    /// hidden, so that the guest can shut down cleanly before the worker kills
    /// QEMU 8 s later.
    pub fn send_requested_power_off(&mut self) {
        if std::mem::take(&mut menu_state::lock().power_off_stimuli_requested) {
            self.inject(self.power_off_frame());
        }
    }

    /// Step the boot and idle shade alpha toward its target. Returns
    /// `(booting, target_alpha)` for the callers that depend on them.
    fn tick_boot_shade(&mut self, ctx: &egui::Context) -> (bool, f32) {
        let (qemu_running, shade_forced, ui_only) = {
            let s = menu_state::lock();
            (s.qemu_running, s.shade_forced, s.ui_only)
        };
        self.track_qemu(qemu_running);

        let panel_driven = self.led_state.frame.iter().any(|&b| b != 0);
        let waited = self
            .qemu_running_since
            .is_some_and(|t| t.elapsed() >= BOOT_SHADE_TIMEOUT);
        let booting = (qemu_running && !panel_driven && !waited) || shade_forced;
        menu_state::lock().guest_booted = qemu_running && !booting;
        // `--no-spawn` has no guest to wait for, so the chassis stays unshaded.
        let target_alpha: f32 = if ui_only {
            0.0
        } else if !qemu_running || booting {
            1.0
        } else {
            0.0
        };
        self.ramp_shade(ctx, target_alpha);
        (booting, target_alpha)
    }

    /// Record when QEMU starts, and blank the LCD textures when it exits so
    /// the pop-out windows go black.
    fn track_qemu(&mut self, qemu_running: bool) {
        if qemu_running && !self.qemu_was_running {
            self.qemu_running_since = Some(Instant::now());
        }
        let qemu_just_exited = self.qemu_was_running && !qemu_running;
        self.qemu_was_running = qemu_running;
        if qemu_just_exited {
            self.lcd_textures_need_blank = true;
            self.qemu_running_since = None;
            self.led_state = LedState::default();
        }
    }

    /// Ramp the shade alpha linearly toward `target_alpha` at
    /// [`SHADE_FADE_SPEED`].
    fn ramp_shade(&mut self, ctx: &egui::Context, target_alpha: f32) {
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
    }

    /// Append a deferred bloom pass to the topmost layer, so it runs after all
    /// the chrome has been submitted.
    fn queue_bloom_pass(&self, ctx: &egui::Context) {
        let Some(bloom_arc) = self.bloom.clone() else {
            return;
        };
        let mask = bloom::BloomMask {
            excludes: self.bloom_excludes.clone(),
            keeps: self.bloom_keeps.clone(),
            chrome: cdj3k_emu_platform::menu::chrome_rects(ctx),
        };
        let scene_key = self.bloom_scene_key();
        let cb = eframe::egui_glow::CallbackFn::new(move |info, painter| {
            puffin::profile_scope!("bloom_gl");
            let [w, h] = info.screen_size_px;
            // If a previous bloom run panicked and poisoned the lock, skip
            // this frame's bloom; propagating the panic through the GL
            // callback would tear down the renderer.
            let Ok(mut bloom) = bloom_arc.lock() else {
                return;
            };
            bloom.run(
                painter.gl(),
                w as i32,
                h as i32,
                info.pixels_per_point,
                &mask,
                scene_key,
            );
        });
        // `Order::Debug` is above the `Order::Foreground` that a menu draws
        // in, and the pass covers the whole viewport; the mask's chrome rects
        // keep the bloom off the menu strip and every open panel.
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Debug,
            egui::Id::new("bloom_pass"),
        ))
        .add(egui::Shape::Callback(egui::PaintCallback {
            rect: ctx.content_rect(),
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
