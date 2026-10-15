//! Jog wheel physics, the encoding of its angular state, and its input.
//!
//! This module holds the linear brake model, the velocity decay, the MISO
//! encoding of `jog_pos`, `jog_vel` and `jog_touch`, and the entry points that
//! drag and scroll samples feed.

use cdj3k_emu_panel::JogBrake;

use super::CdjApp;

/// Reference angular speed, in rad/s, for the brake stop-time table. TAU is one
/// turn per second, the reference speed of the EP122 jog load check.
pub(super) const JOG_BRAKE_REF_OMEGA: f32 = std::f32::consts::TAU;

/// Stop time, in seconds from 1 rev/s to rest, for each of the 13 `jog_adjust`
/// detent positions, from 0 (LIGHT) to 12 (HEAVY).
///
/// The brake is linear at every position. The values are measured; 0.318 is
/// not 1/π.
#[allow(clippy::approx_constant)]
pub(super) const JOG_BRAKE_STOP_TIMES_SEC: [f32; 13] = [
    0.817, 0.717, 0.415, 0.318, 0.274, 0.224, 0.204, 0.154, 0.108, 0.096, 0.089, 0.082, 0.081,
];

/// The angular speed, in rad/s, above which `jog_touch` reports the platter as
/// moving (such as `0x07` or `0x0c`) and not still (`0x03`). It does not
/// affect the brake.
pub(super) const JOG_OMEGA_MOVING_THRESHOLD: f32 = 0.01;

/// Power-law velocity encoding: `vel = JOG_VEL_COEFF / speed_rps^JOG_VEL_EXP`.
/// At 1 rps the value is about 315; at rest it is `JOG_VEL_STOPPED`.
pub(super) const JOG_VEL_COEFF: f32 = 315.0;
pub(super) const JOG_VEL_EXP: f32 = 0.906;

/// Ticks per turn in the device's u16 `jog_pos`.
pub(super) const JOG_TICKS_PER_REV: f32 = 3240.0;

// ── Haptic tunables ──────────────────────────────────────────────────────────
// These are matters of taste. They affect only Force Touch trackpads and the
// Magic Mouse 2; a device without haptic hardware ignores the trigger.

/// Number of haptic clicks per full turn of the platter.
pub(super) const HAPTIC_CLICKS_PER_REV: f32 = 24.0;

/// Number of `MTActuatorActuate` calls per logical click.
pub(super) const HAPTIC_PULSES_PER_CLICK: u32 = 1;

pub(super) const HAPTIC_WAVEFORM_SLOW: i32 = 1;
pub(super) const HAPTIC_WAVEFORM_MEDIUM: i32 = 4;
pub(super) const HAPTIC_WAVEFORM_FAST: i32 = 6;

/// Angular-speed thresholds, in rev/s, that pick the waveform tier. Below
/// `HAPTIC_SPEED_SLOW_MAX_RPS` the waveform is slow, below
/// `HAPTIC_SPEED_MEDIUM_MAX_RPS` it is medium, and above that it is fast. CDJ
/// play speed is about 0.55 rev/s (33 RPM), so normal play uses the slow
/// tier.
pub(super) const HAPTIC_SPEED_SLOW_MAX_RPS: f32 = 2.0;
pub(super) const HAPTIC_SPEED_MEDIUM_MAX_RPS: f32 = 8.0;

/// Device encoder ticks between haptic clicks, derived from
/// [`HAPTIC_CLICKS_PER_REV`]. At 24 clicks and 3240 ticks per turn, this is
/// 135 ticks, or 15° of platter rotation.
const HAPTIC_TICKS_PER_CLICK: f32 = JOG_TICKS_PER_REV / HAPTIC_CLICKS_PER_REV;

/// Time constant, in seconds, for blending the observed input velocity into
/// `jog_omega`. A smaller value follows pointer and scroll samples more
/// closely.
pub(super) const JOG_INPUT_RESPONSE_TIME_SEC: f32 = 0.03;

/// How long a scroll keeps its jog mode (touch or grip), so the mode does not
/// flicker.
pub(super) const JOG_SCROLL_HOLD_SEC: f32 = 0.08;

/// Duration, in seconds, of the touch pulse after a slingshot is released from
/// the scratch zone.
pub(super) const JOG_RELEASE_TOUCH_PULSE_SEC: f32 = 0.5;

// ── MISO encoding constants ────────────────────────────────────────────────

/// The `jog_vel` sent when the platter is at rest. The encoding is inverted.
const JOG_VEL_STOPPED: u16 = 0xffff;
/// The speed, in rev/s, below which the platter counts as stopped.
const JOG_VEL_REST_RPS: f32 = 1.0e-3;
/// The largest moving `jog_vel`; `0xffff` is reserved for a stopped platter.
const JOG_VEL_MAX: f32 = 65534.0;
/// The smallest dt, in seconds, used to integrate the physics.
const DT_FLOOR: f32 = 1.0e-4;
/// The smallest stop time used as the brake's denominator, so a zero entry in
/// `JOG_BRAKE_STOP_TIMES_SEC` cannot divide by zero.
const BRAKE_STOP_TIME_FLOOR_SEC: f32 = 1.0e-3;

// The `jog_touch` byte values that the device's input stack expects.
const TOUCH_NONE: u8 = 0x00;
const TOUCH_HELD_STATIONARY: u8 = 0x03;
const TOUCH_REST_BASELINE: u8 = 0x04;
const TOUCH_HELD_FORWARD: u8 = 0x07;
const TOUCH_FREE_BACKWARD: u8 = 0x08;
const TOUCH_HELD_BACKWARD: u8 = 0x0b;
const TOUCH_FREE_FORWARD: u8 = 0x0c;

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

impl CdjApp {
    /// Integrates the jog platter one frame.
    ///
    /// The platter always decelerates at a constant rate, as the real jog
    /// does: `ω -= brake · sign(ω) · dt`. `ω` reaches zero after `|ω₀| / brake`.
    /// `brake`, in rad/s², comes from the player's fixed stop time or from the
    /// JOG ADJUST knob's stop time in [`JOG_BRAKE_STOP_TIMES_SEC`].
    ///
    /// Updates `jog_pos`, `jog_vel` and `jog_touch`, and calls
    /// [`Self::inject_jog`] when any of them changed this frame.
    pub(super) fn tick_jog(&mut self, dt: f32) {
        puffin::profile_function!();
        self.jog_ticked_at = std::time::Instant::now();
        let dragging = self.jog_drag_prev_angle.is_some();
        self.jog_scroll_hold_sec = (self.jog_scroll_hold_sec - dt).max(0.0);
        let scroll_active = self.jog_scroll_hold_sec > 0.0;
        if !dragging && !scroll_active {
            // Clear the grip mode once the scroll or drag has ended.
            self.jog_grip_drag = false;
        }
        let grip = self.jog_grip_drag;
        // After a scratch-zone slingshot, keep the touch flag set for
        // JOG_RELEASE_TOUCH_PULSE_SEC so the firmware sees the flick over
        // several MISO polls. The pulse is short enough to stay below the
        // held-platter brake.
        let release_pulse = self.jog_release_touch_pulse_remaining_sec > 0.0;
        self.jog_release_touch_pulse_remaining_sec =
            (self.jog_release_touch_pulse_remaining_sec - dt).max(0.0);
        let touch_active = ((dragging || scroll_active) && !grip) || release_pulse;

        // Use the model's fixed stop time, or interpolate the knob's stop time
        // between its 13 detents.
        let stop_time = match self.model.spec().jog_brake {
            JogBrake::Fixed { stop_secs } => stop_secs,
            JogBrake::Adjustable => {
                let adj_f = self.jog_adjust * (JOG_BRAKE_STOP_TIMES_SEC.len() - 1) as f32;
                let lo = (adj_f as usize).min(JOG_BRAKE_STOP_TIMES_SEC.len() - 1);
                let hi = (lo + 1).min(JOG_BRAKE_STOP_TIMES_SEC.len() - 1);
                lerp(
                    JOG_BRAKE_STOP_TIMES_SEC[lo],
                    JOG_BRAKE_STOP_TIMES_SEC[hi],
                    adj_f.fract(),
                )
            }
        };
        let brake = JOG_BRAKE_REF_OMEGA / stop_time.max(BRAKE_STOP_TIME_FLOOR_SEC);

        if self.jog_omega != 0.0 {
            if !dragging && !scroll_active {
                // Integrate omega into the angle only while coasting. During
                // a drag or scroll, jog_apply_delta already moves jog_angle,
                // so integrating here as well would count the motion twice.
                self.jog_angle += self.jog_omega * dt;
            }
            self.jog_apply_friction(dt, brake);
        }

        self.jog_update_encoded_state(touch_active, dt);
        self.jog_touch_active = touch_active;

        self.debug.jog_brake = brake;
    }

    /// Encodes `jog_pos`, `jog_vel` and `jog_touch` from the current motion.
    fn jog_update_encoded_state(&mut self, touch_active: bool, dt: f32) {
        puffin::profile_function!();

        let (new_pos, new_rps, tick_delta) = self.jog_advance_position(dt);
        self.jog_haptic_detents(tick_delta);
        let new_vel = encode_jog_vel(self.jog_omega);
        let new_touch = encode_jog_touch(touch_active, self.jog_omega, self.jog_touch);

        let changed = new_pos != self.jog_pos
            || new_vel != self.jog_vel
            || new_touch != self.jog_touch
            || new_rps != self.jog_rps;
        self.jog_rps = new_rps;
        self.jog_pos = new_pos;
        self.jog_vel = new_vel;
        self.jog_touch = new_touch;
        if changed {
            self.inject_jog();
        }
    }

    /// Advances the position, a free-running wrapping u16. It uses the actual
    /// angle delta so that slow drags still register single ticks. Returns the
    /// new `jog_pos`, the platter's turns per second this tick (its coast while
    /// free, the hand's motion while held) and the ticks it moved.
    fn jog_advance_position(&mut self, dt: f32) -> (u16, f32, f32) {
        let angle_delta = self.jog_angle - self.jog_angle_last_encoded;
        self.jog_angle_last_encoded = self.jog_angle;
        self.jog_revs += f64::from(angle_delta / std::f32::consts::TAU);
        let new_rps = angle_delta / std::f32::consts::TAU / dt.max(DT_FLOOR);
        let tick_delta = angle_delta * JOG_TICKS_PER_REV / std::f32::consts::TAU;
        self.jog_accum += tick_delta;
        let ticks = self.jog_accum.trunc() as i32;
        self.jog_accum -= ticks as f32;
        (self.jog_pos.wrapping_add(ticks as u16), new_rps, tick_delta)
    }

    /// Fires one Taptic Engine pulse per HAPTIC_TICKS_PER_CLICK ticks of
    /// motion, at any speed. A slow drag gives individual clicks; on a fast
    /// spin, the dispatcher thread paces the IOKit calls at
    /// MIN_ACTUATE_INTERVAL and drops the excess.
    ///
    /// `actuate()` is a non-blocking `try_send` that does nothing without an
    /// actuator, so this function requests a click at every detent crossing
    /// and the platform layer decides what to do with it.
    fn jog_haptic_detents(&mut self, tick_delta: f32) {
        self.jog_haptic_accum += tick_delta.abs();
        if self.jog_haptic_accum < HAPTIC_TICKS_PER_CLICK {
            return;
        }
        let waveform = haptic_waveform(self.jog_omega.abs() / std::f32::consts::TAU);

        // Drain the accumulator even when haptics are off; otherwise turning
        // them back on after a long spin would fire the stored clicks at once.
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

    /// Applies the linear brake to `jog_omega` for every input method.
    fn jog_apply_friction(&mut self, dt: f32, brake: f32) {
        let dec = brake * dt;
        if self.jog_omega.abs() <= dec {
            self.jog_omega = 0.0;
            // Do not reset `jog_haptic_accum` here. At very slow scroll input,
            // the brake brings `jog_omega` to zero between events, so a reset
            // at zero would discard the slow progress and no click would fire.
            // A partial accumulator kept across a long pause makes the next
            // click at most one detent early.
        } else {
            self.jog_omega -= dec * self.jog_omega.signum();
        }
    }

    /// The platter's drawn rotation, in radians clockwise, modulo TAU. A -π/2
    /// phase puts zero at 12 o'clock.
    pub(super) fn jog_display_angle(&self) -> f32 {
        (self.jog_angle - std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::TAU)
    }

    /// Current JOG ADJUST value, clamped to `[0, 1]`.
    pub(super) fn jog_adjust(&self) -> f32 {
        self.jog_adjust.clamp(0.0, 1.0)
    }

    /// The platter's angular velocity, in rad/s.
    pub(super) fn jog_angular_velocity(&self) -> f32 {
        self.jog_omega
    }

    pub(super) fn set_jog_adjust(&mut self, v: f32) {
        self.jog_adjust = v.clamp(0.0, 1.0);
    }

    /// Feeds a pointer angle, in radians with the atan2 convention, for a
    /// rotational drag on the platter. The first call after a press sets the
    /// baseline; later calls compute the signed angular delta and update
    /// `jog_angle` and `jog_omega`.
    pub(super) fn jog_drag_sample(&mut self, pointer_angle: f32, dt: f32) {
        if let Some(prev) = self.jog_drag_prev_angle {
            // Wrap the delta into [-π, π], so crossing the ±π seam takes the
            // short way.
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

    /// Applies an angular increment to the jog and updates its angular
    /// velocity, for both drag and wheel-scroll input.
    fn jog_apply_delta(&mut self, delta_rad: f32, dt: f32, source: &'static str) {
        let dt = dt.max(DT_FLOOR);
        // The response does not depend on the frame rate.
        let alpha = 1.0 - (-dt / JOG_INPUT_RESPONSE_TIME_SEC).exp();
        let omega_sample = delta_rad / dt;
        self.jog_angle += delta_rad;
        self.jog_omega += (omega_sample - self.jog_omega) * alpha;
        self.debug.jog_source = source;
        self.debug.jog_delta_rad = delta_rad;
        self.debug.jog_dt = dt;
        self.debug.jog_omega_sample = omega_sample;
        // Haptics depend only on distance (see `jog_update_encoded_state`).
        // Do not fire a click per input event: the clicks would follow the
        // trackpad's event rate of about 60-90 Hz and not the platter's
        // speed, and every scroll would feel saturated at once.
    }

    /// Feeds a wheel-scroll jog sample, already converted to radians. With
    /// `grip` set the jog bytes carry no touch; without it they carry a touch.
    pub(super) fn jog_scroll_sample(&mut self, delta_rad: f32, dt: f32, grip: bool) {
        self.jog_grip_drag = grip;
        self.jog_scroll_hold_sec = JOG_SCROLL_HOLD_SEC;
        self.jog_apply_delta(delta_rad, dt, "scroll");
    }

    /// Ends a rotational drag. It clears the pointer-angle baseline so the
    /// platter spins freely, and leaves `jog_omega` at its last dragged value
    /// as the release impulse.
    pub(super) fn jog_drag_release(&mut self) {
        self.jog_drag_prev_angle = None;
    }

    pub(super) fn jog_is_dragging(&self) -> bool {
        self.jog_drag_prev_angle.is_some()
    }

    /// Replaces the angular velocity with `omega`, in rad/s, and ends any
    /// rotational drag so the platter coasts at once. Used when a Ctrl
    /// slingshot is released. With `scratch_mode` set (the slingshot was
    /// anchored on the center platter), it starts a touch pulse of
    /// JOG_RELEASE_TOUCH_PULSE_SEC so the firmware sees a flick. Without it
    /// (anchored in the grip ring), it is a bend with no change to the touch
    /// byte.
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

/// The haptic waveform tier for a platter turning at `speed_rps`.
fn haptic_waveform(speed_rps: f32) -> i32 {
    if speed_rps < HAPTIC_SPEED_SLOW_MAX_RPS {
        HAPTIC_WAVEFORM_SLOW
    } else if speed_rps < HAPTIC_SPEED_MEDIUM_MAX_RPS {
        HAPTIC_WAVEFORM_MEDIUM
    } else {
        HAPTIC_WAVEFORM_FAST
    }
}

/// Encodes the velocity with the inverted power law.
fn encode_jog_vel(omega: f32) -> u16 {
    let speed_rps = omega.abs() / std::f32::consts::TAU;
    if speed_rps < JOG_VEL_REST_RPS {
        JOG_VEL_STOPPED
    } else {
        (JOG_VEL_COEFF / speed_rps.powf(JOG_VEL_EXP)).clamp(0.0, JOG_VEL_MAX) as u16
    }
}

/// Encodes the touch byte from the touch flag and the direction of motion.
/// `prev` is the byte last sent.
fn encode_jog_touch(touch_active: bool, omega: f32, prev: u8) -> u8 {
    let moving_fwd = omega > JOG_OMEGA_MOVING_THRESHOLD;
    let moving_back = omega < -JOG_OMEGA_MOVING_THRESHOLD;
    if touch_active {
        // The touch flag is set, as in scratch mode.
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
    } else if prev == TOUCH_FREE_BACKWARD || prev == TOUCH_FREE_FORWARD {
        // When a coast ends, send `TOUCH_NONE` on the frame the platter comes
        // to rest, then hold `TOUCH_REST_BASELINE`.
        TOUCH_NONE
    } else {
        TOUCH_REST_BASELINE
    }
}
