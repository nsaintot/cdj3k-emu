//! Jog wheel physics, angular-state encoding, and input integration.
//!
//! Owns the linear brake model, velocity decay, MISO encoding of
//! `jog_pos` / `jog_vel` / `jog_touch`, and the entry points fed by
//! drag and scroll input samples.

use super::CdjApp;

/// Reference angular speed (rad/s) for the brake stop-time table.
/// TAU = 1 rev/s; the EP122 jog-load-check reference speed is ≈1 rev/s.
pub(super) const JOG_BRAKE_REF_OMEGA: f32 = std::f32::consts::TAU;

/// Stop-time (seconds from 1 rev/s to rest) for each of the 13 `jog_adjust`
/// detent positions, indexed 0 (LIGHT) → 12 (HEAVY).
///
/// Source: EP122TestMode jog-load-check, measured at ≈1 rev/s reference
/// speed. Linear brake model confirmed (R² > 0.97 across all positions).
pub(super) const JOG_BRAKE_STOP_TIMES_SEC: [f32; 13] = [
    0.817, 0.717, 0.415, 0.318, 0.274, 0.224, 0.204, 0.154, 0.108, 0.096, 0.089, 0.082, 0.081,
];

/// Absolute angular-velocity threshold used only to classify `jog_touch`
/// (`0x0c` turning vs `0x03` held-still) while the user is dragging.
/// Does not affect the brake; the linear model reaches zero on its own.
pub(super) const JOG_OMEGA_MOVING_THRESHOLD: f32 = 0.01;

/// Power-law velocity encoding: `vel = JOG_VEL_COEFF / speed_rps^JOG_VEL_EXP`
/// At 1 rps → ~315, at rest → `JOG_VEL_STOPPED`.
pub(super) const JOG_VEL_COEFF: f32 = 315.0;
pub(super) const JOG_VEL_EXP: f32 = 0.906;

/// Ticks per revolution for the device `jog_pos` u16 encoding.
pub(super) const JOG_TICKS_PER_REV: f32 = 3240.0;

// ── Haptic tunables ──────────────────────────────────────────────────────────
// Tweak these to taste; they only affect Force Touch trackpads / Magic Mouse 2.
// Devices without haptic hardware silently ignore the trigger.

/// Number of haptic clicks felt per full platter rotation.
pub(super) const HAPTIC_CLICKS_PER_REV: f32 = 24.0;

/// Number of `MTActuatorActuate` calls per logical click.
pub(super) const HAPTIC_PULSES_PER_CLICK: u32 = 1;

pub(super) const HAPTIC_WAVEFORM_SLOW: i32 = 1;
pub(super) const HAPTIC_WAVEFORM_MEDIUM: i32 = 4;
pub(super) const HAPTIC_WAVEFORM_FAST: i32 = 6;

/// Angular-speed thresholds (rev/s) for picking the waveform tier above.
/// `speed_rps < SLOW_MAX` → slow waveform, `< MED_MAX` → medium, else fast.
/// CDJ play speed is ~0.55 rev/s (33 RPM), so default play sits in medium;
/// scratch nudges bump up to fast.
pub(super) const HAPTIC_SPEED_SLOW_MAX_RPS: f32 = 2.0;
pub(super) const HAPTIC_SPEED_MEDIUM_MAX_RPS: f32 = 8.0;

/// Device-encoder ticks between haptic clicks, derived from
/// [`HAPTIC_CLICKS_PER_REV`].  E.g. at 4 clicks/rev with 3240 ticks/rev,
/// this is 810 ticks ≈ 90° of platter rotation between clicks.
const HAPTIC_TICKS_PER_CLICK: f32 = JOG_TICKS_PER_REV / HAPTIC_CLICKS_PER_REV;

/// Time constant (s) for blending observed input velocity into `jog_omega`.
/// Smaller values track pointer/scroll samples more tightly.
pub(super) const JOG_INPUT_RESPONSE_TIME_SEC: f32 = 0.03;

/// Debounce/hold for scroll jog semantics (touch or grip) to avoid mode flicker.
pub(super) const JOG_SCROLL_HOLD_SEC: f32 = 0.08;

/// Duration of the scratch-zone slingshot release touch pulse (seconds).
pub(super) const JOG_RELEASE_TOUCH_PULSE_SEC: f32 = 0.5;

// ── MISO encoding constants ────────────────────────────────────────────────

/// `jog_vel` value emitted when the platter is fully at rest (inverse encoding).
const JOG_VEL_STOPPED: u16 = 0xffff;
/// Speed-RPS floor below which the platter is treated as stopped.
const JOG_VEL_REST_RPS: f32 = 1.0e-3;
/// Maximum encodable `jog_vel` (`0xffff` reserved as "stopped").
const JOG_VEL_MAX: f32 = 65534.0;
/// Minimum dt clamp (s) used when integrating physics - guards against frame stalls.
const DT_FLOOR: f32 = 1.0e-4;
/// Minimum brake denominator clamp - guards against `JOG_BRAKE_STOP_TIMES_SEC[i] = 0`.
const BRAKE_STOP_TIME_FLOOR_SEC: f32 = 1.0e-3;

// `jog_touch` byte encoding (matches what the device's input stack expects).
const TOUCH_NONE: u8 = 0x00;
const TOUCH_HELD_STATIONARY: u8 = 0x03;
const TOUCH_REST_BASELINE: u8 = 0x04;
const TOUCH_HELD_FORWARD: u8 = 0x07;
const TOUCH_FREE_BACKWARD: u8 = 0x08;
const TOUCH_HELD_BACKWARD: u8 = 0x0b;
const TOUCH_FREE_FORWARD: u8 = 0x0c;

/// Width (chars) of the centred debug-line strings rendered above the MISO dump.
const DBG_LINE_WIDTH: usize = 56;

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

impl CdjApp {
    /// Integrates the jog platter one frame.
    ///
    /// The platter always applies **linear** (constant) angular deceleration,
    /// matching the jog behavior: `ω -= brake · sign(ω) · dt`.
    /// `ω` hits zero in finite time `|ω₀| / brake`. `brake` (rad/s²) is
    /// derived from the stop-time parameters in [`JOG_BRAKE_STOP_TIMES_SEC`].
    ///
    /// Updates `jog_pos` / `jog_vel` / `jog_touch` and calls
    /// [`Self::inject_jog`] when any of them changed this frame.
    pub(super) fn tick_jog(&mut self, dt: f32) {
        puffin::profile_function!();
        let dragging = self.jog_drag_prev_angle.is_some();
        self.jog_scroll_hold_sec = (self.jog_scroll_hold_sec - dt).max(0.0);
        let scroll_active = self.jog_scroll_hold_sec > 0.0;
        if !dragging && !scroll_active {
            // Clear transient mode once the scroll/drag session is fully over.
            self.jog_grip_drag = false;
        }
        let grip = self.jog_grip_drag;
        // Release-pulse: latch touch=true for JOG_RELEASE_TOUCH_PULSE_SEC after
        // a scratch-zone slingshot so firmware sees the flick over several
        // MISO polls. Short enough to stay below the held-platter brake.
        let release_pulse = self.jog_release_touch_pulse_remaining_sec > 0.0;
        self.jog_release_touch_pulse_remaining_sec =
            (self.jog_release_touch_pulse_remaining_sec - dt).max(0.0);
        let touch_active = ((dragging || scroll_active) && !grip) || release_pulse;

        // Interpolate brake from the 13 jog_adjust detent positions.
        let adj_f = self.jog_adjust * (JOG_BRAKE_STOP_TIMES_SEC.len() - 1) as f32;
        let lo = (adj_f as usize).min(JOG_BRAKE_STOP_TIMES_SEC.len() - 1);
        let hi = (lo + 1).min(JOG_BRAKE_STOP_TIMES_SEC.len() - 1);
        let stop_time = lerp(
            JOG_BRAKE_STOP_TIMES_SEC[lo],
            JOG_BRAKE_STOP_TIMES_SEC[hi],
            adj_f.fract(),
        );
        let brake = JOG_BRAKE_REF_OMEGA / stop_time.max(BRAKE_STOP_TIME_FLOOR_SEC);
        let dec = brake * dt.max(DT_FLOOR);

        if self.jog_omega != 0.0 {
            if !dragging && !scroll_active {
                // Free coast only: integrate omega into position. During
                // active drag/scroll, jog_apply_delta already moves jog_angle
                // directly; integrating omega here too would double-count.
                self.jog_angle += self.jog_omega * dt;
            }
            self.jog_apply_friction(dt, brake);
        }

        self.jog_update_encoded_state(touch_active);
        self.jog_touch_active = touch_active;

        if self.debug_screen_popped {
            self.refresh_jog_debug_lines(dragging, grip, brake, dec);
        }
    }

    fn refresh_jog_debug_lines(&mut self, dragging: bool, grip: bool, brake: f32, dec: f32) {
        puffin::profile_function!();
        let w = DBG_LINE_WIDTH;
        self.jog_dbg_lines[0] = format!(
            "{:^w$}",
            format!(
                "src={} drag={} touch={} grip={} jog_scroll_hold_sec={:.2}",
                self.jog_dbg_last_source,
                dragging,
                self.jog_touch_active,
                grip,
                self.jog_scroll_hold_sec
            )
        );
        self.jog_dbg_lines[1] = format!(
            "{:^w$}",
            format!(
                "dθ={:+.4} dt={:.4} ωs={:+.2} ω={:+.2}",
                self.jog_dbg_last_delta_rad,
                self.jog_dbg_last_dt,
                self.jog_dbg_last_omega_sample,
                self.jog_omega
            )
        );
        self.jog_dbg_lines[2] = format!(
            "{:^w$}",
            format!(
                "adj={:.2} brake={:.2} dec={:.3} ω={:+.2}rad/s ωs={:+.2}",
                self.jog_adjust, brake, dec, self.jog_omega, self.jog_dbg_last_omega_sample
            )
        );
    }

    /// Encodes `jog_pos` / `jog_vel` / `jog_touch` from current kinematics.
    fn jog_update_encoded_state(&mut self, touch_active: bool) {
        puffin::profile_function!();

        // Position: free-running wrapping u16. Use the actual angle delta
        // (not omega · dt) so slow drags still register single ticks.
        let angle_delta = self.jog_angle - self.jog_angle_last_encoded;
        self.jog_angle_last_encoded = self.jog_angle;
        let tick_delta = angle_delta * JOG_TICKS_PER_REV / std::f32::consts::TAU;
        self.jog_accum += tick_delta;
        let ticks = self.jog_accum.trunc() as i32;
        self.jog_accum -= ticks as f32;
        let new_pos = self.jog_pos.wrapping_add(ticks as u16);

        // Haptic detents - one Taptic Engine pulse per HAPTIC_TICKS_PER_CLICK
        // ticks of motion.  Pure distance-driven: every detent crossing
        // fires regardless of speed.  Slow drag → individual clicks; fast
        // spin → the dispatcher thread paces real IOKit calls at
        // MIN_ACTUATE_INTERVAL and drops excess queue entries.
        //
        // `actuate()` is a non-blocking try_send that returns ~immediately
        // and no-ops when the actuator isn't available, so no upstream
        // arming/gating is needed - if we crossed a detent, we ask for the
        // click; the platform layer decides what to do with it.
        self.jog_haptic_accum += tick_delta.abs();
        if self.jog_haptic_accum >= HAPTIC_TICKS_PER_CLICK {
            let speed_rps = self.jog_omega.abs() / std::f32::consts::TAU;
            let waveform = if speed_rps < HAPTIC_SPEED_SLOW_MAX_RPS {
                HAPTIC_WAVEFORM_SLOW
            } else if speed_rps < HAPTIC_SPEED_MEDIUM_MAX_RPS {
                HAPTIC_WAVEFORM_MEDIUM
            } else {
                HAPTIC_WAVEFORM_FAST
            };

            // Drain the accumulator regardless of the toggle - otherwise
            // turning haptics back on after a long spin would fire a backlog
            // of stored clicks at once.  When disabled, we still drain but
            // skip the actuator call.
            let enabled = cdj3k_emu_platform::menu_state::lock().haptic_enabled;
            while self.jog_haptic_accum >= HAPTIC_TICKS_PER_CLICK {
                self.jog_haptic_accum -= HAPTIC_TICKS_PER_CLICK;
                if enabled {
                    for _ in 0..HAPTIC_PULSES_PER_CLICK {
                        cdj3k_emu_platform::haptic::actuate(waveform);
                    }
                }
            }
        }

        // Velocity: power-law inverse encoding.
        let speed_rps = self.jog_omega.abs() / std::f32::consts::TAU;
        let new_vel: u16 = if speed_rps < JOG_VEL_REST_RPS {
            JOG_VEL_STOPPED
        } else {
            (JOG_VEL_COEFF / speed_rps.powf(JOG_VEL_EXP)).clamp(0.0, JOG_VEL_MAX) as u16
        };

        // Touch byte: combine touch flag with movement direction.
        let moving_fwd = self.jog_omega > JOG_OMEGA_MOVING_THRESHOLD;
        let moving_back = self.jog_omega < -JOG_OMEGA_MOVING_THRESHOLD;
        let new_touch = if touch_active {
            // Center platter drag: scratch/vinyl mode - touch flag set.
            if moving_fwd {
                TOUCH_HELD_FORWARD
            } else if moving_back {
                TOUCH_HELD_BACKWARD
            } else {
                TOUCH_HELD_STATIONARY
            }
        } else if moving_fwd {
            TOUCH_FREE_FORWARD
        } else if moving_back {
            TOUCH_FREE_BACKWARD
        } else {
            // Natural coast stop: emit `TOUCH_NONE` exactly on the frame
            // we settle to rest, then hold `TOUCH_REST_BASELINE`.
            if self.jog_touch == TOUCH_FREE_BACKWARD || self.jog_touch == TOUCH_FREE_FORWARD {
                TOUCH_NONE
            } else {
                TOUCH_REST_BASELINE
            }
        };

        let changed =
            new_pos != self.jog_pos || new_vel != self.jog_vel || new_touch != self.jog_touch;
        self.jog_pos = new_pos;
        self.jog_vel = new_vel;
        self.jog_touch = new_touch;
        if changed {
            self.inject_jog();
        }
    }

    /// Linear brake derived from `jog_adjust`, shared by all input methods.
    fn jog_apply_friction(&mut self, dt: f32, brake: f32) {
        let dec = brake * dt;
        if self.jog_omega.abs() <= dec {
            self.jog_omega = 0.0;
            // Intentionally NOT resetting `jog_haptic_accum` here.  At very
            // slow scroll inputs the brake decays `jog_omega` to zero
            // between every event, so resetting the accumulator on omega→0
            // would wipe the slow-motion progress and starve the haptic
            // feedback.  A stale partial accumulator carried across a long
            // pause is at most one detent-stride of "early" click on
            // resumption, which is imperceptible.
        } else {
            self.jog_omega -= dec * self.jog_omega.signum();
        }
    }

    /// Visual rotation of the platter, modulo TAU (radians, clockwise).
    /// Applies a -π/2 phase so zero is rendered at 12 o'clock.
    pub(super) fn jog_display_angle(&self) -> f32 {
        (self.jog_angle - std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::TAU)
    }

    /// Current JOG ADJUST value, clamped to `[0, 1]`.
    pub(super) fn jog_adjust(&self) -> f32 {
        self.jog_adjust.clamp(0.0, 1.0)
    }

    /// Current physical jog angular velocity in rad/s.
    pub(super) fn jog_angular_velocity(&self) -> f32 {
        self.jog_omega
    }

    pub(super) fn set_jog_adjust(&mut self, v: f32) {
        self.jog_adjust = v.clamp(0.0, 1.0);
    }

    /// Feeds an observed pointer-angle sample (atan2 convention, radians) for
    /// rotational drag on the platter. The first call after a press registers
    /// the baseline; subsequent calls compute the signed angular delta and
    /// update `jog_angle` + `jog_omega`.
    pub(super) fn jog_drag_sample(&mut self, pointer_angle: f32, dt: f32) {
        if let Some(prev) = self.jog_drag_prev_angle {
            // Wrap delta to shortest-path in [-π, π] so crossing the ±π seam works.
            let mut d = pointer_angle - prev;
            let pi = std::f32::consts::PI;
            if d > pi {
                d -= std::f32::consts::TAU;
            } else if d < -pi {
                d += std::f32::consts::TAU;
            }
            self.jog_apply_delta(d, dt, "drag");
        }
        self.jog_drag_prev_angle = Some(pointer_angle);
    }

    /// Applies an angular increment to the jog and updates angular velocity.
    /// Shared by drag and wheel-scroll control paths.
    fn jog_apply_delta(&mut self, delta_rad: f32, dt: f32, source: &'static str) {
        let dt = dt.max(DT_FLOOR);
        // Frame-rate-independent response.
        let alpha = 1.0 - (-dt / JOG_INPUT_RESPONSE_TIME_SEC).exp();
        let omega_sample = delta_rad / dt;
        self.jog_angle += delta_rad;
        self.jog_omega += (omega_sample - self.jog_omega) * alpha;
        self.jog_dbg_last_source = source;
        self.jog_dbg_last_delta_rad = delta_rad;
        self.jog_dbg_last_dt = dt;
        self.jog_dbg_last_omega_sample = omega_sample;
        // Haptic is purely distance-driven (in `jog_update_encoded_state`).
        // We intentionally do NOT fire one click per input event here -
        // doing so makes haptic track the trackpad's event rate (~60-90 Hz)
        // rather than the angular speed of the platter, which feels like
        // "ceiling hit instantly" for any active scroll.
    }

    /// Feeds a wheel-scroll jog sample (already converted to radians).
    /// `grip=true` emits non-touch jog bytes, `grip=false` emits touch-mode bytes.
    pub(super) fn jog_scroll_sample(&mut self, delta_rad: f32, dt: f32, grip: bool) {
        self.jog_grip_drag = grip;
        self.jog_scroll_hold_sec = JOG_SCROLL_HOLD_SEC;
        self.jog_apply_delta(delta_rad, dt, "scroll");
    }

    /// Ends rotational drag: clears the pointer-angle baseline so free-spin
    /// takes over. Leaves `jog_omega` at its last dragged value as the
    /// release impulse.
    pub(super) fn jog_drag_release(&mut self) {
        self.jog_drag_prev_angle = None;
    }

    pub(super) fn jog_is_dragging(&self) -> bool {
        self.jog_drag_prev_angle.is_some()
    }

    /// Replaces the current angular velocity with `omega` (rad/s) and ends any
    /// active rotational drag so free-spin/coast takes over immediately.
    /// `scratch_mode = true` (slingshot anchored on the center platter)
    /// arms a one-tick touch pulse so the firmware sees a flick event;
    /// `false` (anchored in the grip ring) is a pure bend impulse with no
    /// touch byte change. Used by the Ctrl-slingshot release path.
    pub(super) fn jog_apply_impulse(&mut self, omega: f32, scratch_mode: bool) {
        self.jog_drag_prev_angle = None;
        self.jog_omega = omega;
        if scratch_mode {
            self.jog_release_touch_pulse_remaining_sec = JOG_RELEASE_TOUCH_PULSE_SEC;
        }
    }

    pub(super) fn set_jog_grip_drag(&mut self, grip: bool) {
        self.jog_grip_drag = grip;
    }
}
