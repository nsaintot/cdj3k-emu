//! The panel inputs and readings that the control API uses. The shell's
//! control host (`shell/control.rs`) calls these.

use std::f32::consts::TAU;
use std::time::{Duration, Instant};

use cdj3k_emu_control::{Command, JogGrip, JogMove, Knob, Lever};
use cdj3k_emu_panel::{Btn, Direction, Lamp, LampState, MosiFrame, StepLed};
use serde_json::{json, Value};

use super::jog_physics::JOG_OMEGA_MOVING_THRESHOLD;
use super::ui::NAV_DETENT_COUNT;
use super::CdjApp;

/// How long one API jog step keeps the hand on the platter. It outlasts the
/// slowest `logic` pass, about 100 ms while the window is hidden.
const API_JOG_HOLD_SEC: f32 = 0.25;
/// When `ui` has not ticked the jog for this long, `logic` ticks it.
const JOG_TICK_IDLE: Duration = Duration::from_millis(40);
/// The longest step a `logic` tick integrates.
const JOG_TICK_MAX_DT: f32 = 0.25;

/// What the control API holds on the panel.
#[derive(Default)]
pub(crate) struct ApiInputs {
    /// Whether the API holds a touch. The slate and the popout report their
    /// pointer state every frame, which would otherwise overwrite the API's
    /// coordinate before the guest samples it.
    pub(super) touch: bool,
    /// Buttons the API holds down, set in every frame until it releases them.
    /// They are kept apart from `held_btn`, which the slate's widgets clear on
    /// the next frame the pointer is not on them.
    pub(super) btns: Vec<(usize, u8)>,
    /// The jog turn the API is making.
    jog: Option<ApiJog>,
}

/// A jog turn the control API is making: the angle still to turn, at a
/// steady rate, by the top plate or the side.
struct ApiJog {
    left_rad: f32,
    rate: f32,
    top: bool,
    last: Instant,
    ends: Instant,
}

impl CdjApp {
    /// Clear `bit` in every frame when `on` is true, or stop clearing it, and
    /// inject the frame.
    pub(super) fn set_bit_cleared(&mut self, bit: (usize, u8), on: bool) {
        self.cleared_bits.retain(|b| *b != bit);
        if on {
            self.cleared_bits.push(bit);
        }
        self.inject(self.build_current_frame());
    }

    /// Hold `btn` down in every frame when `on` is true, or release it, and
    /// inject the frame.
    pub(super) fn set_api_btn(&mut self, btn: (usize, u8), on: bool) {
        self.api.btns.retain(|b| *b != btn);
        if on {
            self.api.btns.push(btn);
        }
        self.inject(self.build_current_frame());
    }

    /// Touch the main LCD at `(x, y)` in fractions of the display, or let go
    /// with `None`.
    pub(super) fn set_api_touch(&mut self, at: Option<(f32, f32)>) {
        self.api.touch = at.is_some();
        self.lcd_touch_ctrl_latched = false;
        self.lcd_touch = at.map(|(x, y)| self.model.touch_units(x, y));
        self.inject(self.build_current_frame());
    }

    /// Turn the rotary selector by `detents`, one frame per detent. A
    /// positive count moves the rotary counter up, which moves down a list.
    pub(super) fn turn_rotary(&mut self, detents: i32) {
        let detent_rad = TAU / NAV_DETENT_COUNT;
        for _ in 0..detents.unsigned_abs() {
            if detents > 0 {
                self.rotary = self.rotary.wrapping_add(1);
                self.nav_angle -= detent_rad;
            } else {
                self.rotary = self.rotary.wrapping_sub(1);
                self.nav_angle += detent_rad;
            }
            self.inject_rotary();
        }
    }

    /// Turn the BEAT LOOP encoder by `detents`, one frame per detent. A
    /// positive count moves its counter up, as `turn_rotary` does.
    pub(super) fn turn_beat_loop(&mut self, detents: i32) {
        let detent_rad = TAU / NAV_DETENT_COUNT;
        for _ in 0..detents.unsigned_abs() {
            if detents > 0 {
                self.beat_loop = self.beat_loop.wrapping_add(1);
                self.beat_loop_angle -= detent_rad;
            } else {
                self.beat_loop = self.beat_loop.wrapping_sub(1);
                self.beat_loop_angle += detent_rad;
            }
            self.inject_rotary();
        }
    }

    /// Move the rotary selector, the tempo fader, the deck's tilt, the BEAT
    /// LOOP encoder, a knob or the direction lever as `command` asks.
    pub(super) fn move_control(&mut self, command: Command) -> Result<(), String> {
        match command {
            Command::Rotary(n) => self.turn_rotary(n),
            Command::Tempo(pos) => self.set_tempo(pos),
            Command::Tilt => self.toggle_tilt(),
            Command::Encoder(n) => self.turn_beat_loop(n),
            Command::Knob(knob, pos) => self.set_knob(knob, pos),
            Command::Direction(lever) => self.set_direction(lever),
            other => return Err(format!("{other:?} is not handled here")),
        }
        Ok(())
    }

    /// Set a knob to `pos`, from 0 to 1.
    pub(super) fn set_knob(&mut self, knob: Knob, pos: f32) {
        let pos = pos.clamp(0.0, 1.0);
        match knob {
            Knob::VinylSpeed => self.vinyl_speed = (pos * 255.0).round() as u8,
            Knob::JogAdjust => self.set_jog_adjust(pos),
        }
        self.inject_vinyl();
    }

    /// Put the direction lever in `lever`'s position.
    pub(super) fn set_direction(&mut self, lever: Lever) {
        self.direction = match lever {
            Lever::Forward => Direction::Forward,
            Lever::Reverse => Direction::Reverse,
            Lever::SlipReverse => Direction::SlipReverse,
        };
        self.inject(self.build_current_frame());
    }

    /// Move the tempo fader to `pos`, from 0 (full minus) to 1 (full plus).
    pub(super) fn set_tempo(&mut self, pos: f32) {
        self.tempo = pos.clamp(0.0, 1.0);
        self.inject_tempo();
    }

    /// Start a jog move. A turn replaces a turn under way.
    pub(super) fn start_api_jog(&mut self, mv: JogMove) {
        let now = Instant::now();
        match mv {
            JogMove::Turn {
                degrees,
                time,
                grip,
            } => {
                let rad = degrees.to_radians();
                self.api.jog = Some(ApiJog {
                    left_rad: rad,
                    rate: rad / time.as_secs_f32(),
                    top: grip == JogGrip::Top,
                    last: now,
                    ends: now + time,
                });
            }
            JogMove::Spin { rps, grip } => {
                self.api.jog = None;
                self.jog_apply_impulse(rps * TAU, grip == JogGrip::Top);
            }
        }
    }

    /// End the API's jog turn where it is: the platter stops and the hand lets
    /// go.
    pub(super) fn stop_api_jog(&mut self) {
        if self.api.jog.take().is_some() {
            self.jog_scroll_hold_sec = 0.0;
            self.jog_apply_impulse(0.0, false);
        }
    }

    /// Whether an API jog turn is under way.
    pub(super) fn api_jog_active(&self) -> bool {
        self.api.jog.is_some()
    }

    /// Whether the platter moves or the API turns it, so the jog needs ticks.
    pub(super) fn jog_busy(&self) -> bool {
        self.api.jog.is_some()
            || self.jog_angular_velocity().abs() > JOG_OMEGA_MOVING_THRESHOLD
            || self.jog_scroll_hold_sec > 0.0
    }

    /// Advance the API's jog turn to now, as a scroll sample does. At the end
    /// of the turn the platter stops and the hand lets go.
    pub(super) fn step_api_jog(&mut self) {
        let Some(mut jog) = self.api.jog.take() else {
            return;
        };
        let now = Instant::now();
        let dt = now.duration_since(jog.last).as_secs_f32();
        jog.last = now;
        let done = now >= jog.ends;
        let delta = if done {
            jog.left_rad
        } else {
            let step = jog.rate * dt;
            if step.abs() > jog.left_rad.abs() {
                jog.left_rad
            } else {
                step
            }
        };
        jog.left_rad -= delta;
        self.jog_scroll_sample(delta, dt, !jog.top);
        if done {
            self.jog_scroll_hold_sec = 0.0;
            self.jog_apply_impulse(0.0, false);
        } else {
            self.jog_scroll_hold_sec = API_JOG_HOLD_SEC;
            self.api.jog = Some(jog);
        }
    }

    /// Tick the jog when `ui` has not done so lately, as while the window is
    /// hidden.
    pub(super) fn tick_jog_if_idle(&mut self) {
        let idle = self.jog_ticked_at.elapsed();
        if idle >= JOG_TICK_IDLE {
            self.tick_jog(idle.as_secs_f32().min(JOG_TICK_MAX_DT));
        }
    }

    /// Tilt the deck, or bring it back. It does nothing on a deck without a
    /// front face.
    pub(super) fn toggle_tilt(&mut self) {
        self.tilted = !self.tilted;
    }

    /// The latest LED frame from the guest. It is read from the stream, so
    /// it is current while the window is hidden and draws nothing.
    pub(super) fn live_mosi(&self) -> MosiFrame {
        self.model.mosi_frame(self.ctrl_stream.peek_latest_mosi())
    }

    /// Whether the player app drives the panel on the current QEMU run.
    pub(super) fn panel_driven(&self) -> bool {
        self.ctrl_stream.panel_driven()
    }

    /// The names of the buttons this model has.
    pub(super) fn button_names(&self) -> Vec<&'static str> {
        all_buttons()
            .filter(|b| self.model.button(*b).is_some())
            .map(Btn::name)
            .collect()
    }

    /// A lamp's value as `lamps_json` shows it, or `None` when the model has
    /// no such lamp.
    pub(super) fn lamp_value(&self, lamp: Lamp) -> Option<Value> {
        self.live_mosi().lamp(lamp).map(lamp_json)
    }

    /// Every lamp this model has, with its state.
    pub(super) fn lamps_json(&self) -> serde_json::Map<String, Value> {
        let mosi = self.live_mosi();
        Lamp::ALL
            .iter()
            .filter_map(|l| Some((l.name().to_string(), lamp_json(mosi.lamp(*l)?))))
            .collect()
    }

    /// What the control API has pressed, cleared and touched, plus the frames
    /// on the wire.
    pub(super) fn inputs_json(&self) -> Value {
        let names = |bits: &[(usize, u8)]| -> Vec<String> {
            bits.iter().map(|b| self.bit_name(*b)).collect()
        };
        json!({
            "held": names(&self.api.btns),
            "cleared": names(&self.cleared_bits),
            "touch": self.api.touch.then_some(self.lcd_touch).flatten(),
            "tempo": self.tempo,
            "rotary": self.rotary,
            "tilted": self.tilted,
            "miso": hex(&self.last_miso),
            "mosi": hex(&self.ctrl_stream.peek_latest_mosi()),
        })
    }

    /// The button name of `bit` on this model, or the raw `byte:0xmask` form.
    pub(super) fn bit_name(&self, bit: (usize, u8)) -> String {
        all_buttons()
            .find(|b| self.model.button(*b) == Some(bit))
            .map_or_else(
                || format!("{}:{:#04x}", bit.0, bit.1),
                |b| b.name().to_string(),
            )
    }
}

fn all_buttons() -> impl Iterator<Item = Btn> {
    Btn::SHARED
        .iter()
        .chain(Btn::EXTRA)
        .chain(Btn::OWN)
        .copied()
}

pub(crate) fn lamp_json(state: LampState) -> Value {
    match state {
        LampState::Bit(on) => on.into(),
        LampState::Step(StepLed::Off) => "off".into(),
        LampState::Step(StepLed::Medium) => "medium".into(),
        LampState::Step(StepLed::Full) => "full".into(),
        LampState::Level(v) => v.into(),
        LampState::Rgb(r, g, b) => json!([r, g, b]),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
