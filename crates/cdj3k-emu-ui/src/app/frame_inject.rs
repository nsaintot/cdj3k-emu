//! MISO frame synthesis and per-channel inject helpers.
//!
//! Every state-changing path on `CdjApp` calls [`CdjApp::inject`] with a
//! freshly-built frame from [`CdjApp::build_current_frame`] so that, e.g.,
//! a jog update doesn't silently clear a held button.

use cdj3k_emu_subucom::miso_frame::{self, MisoFrame};

use super::CdjApp;

/// EP122 dead-zone center for the tempo slider (raw u16 value).
///
/// The dead zone is **not** at exactly `0xFFFF / 2` - measured ≈ `0x7F50`.
/// `build_current_frame` uses a piecewise mapping so:
/// - `tempo = 0.0` → `0x0000`
/// - `tempo = 1.0` → `0xFFFF`
/// - `tempo = 0.5` → `TEMPO_CENTER` (lands inside the dead zone)
pub(super) const TEMPO_CENTER: u16 = 0x7F50;

/// Tempo span: full u16 range mapped from `tempo` ∈ [0, 1].
const TEMPO_SPAN: f32 = 0xFFFF as f32;

/// LCD touch coordinate normalisation range: `[0, 1000]` for both axes.
pub(super) const LCD_TOUCH_RANGE: f32 = 1000.0;

impl CdjApp {
    pub(super) fn inject(&mut self, frame_bytes: [u8; miso_frame::MISO_SIZE]) {
        puffin::profile_function!();
        self.last_miso = frame_bytes;
        self.ctrl_stream.inject(&frame_bytes);
    }

    /// Build a MISO frame that reflects the full current control state.
    /// All inject paths must use this instead of `MisoFrame::idle()`.
    pub(super) fn build_current_frame(&self) -> MisoFrame {
        let mut f = MisoFrame::idle();
        if let Some(btn) = self.held_btn {
            f.set_btn(btn, true);
        }
        for &btn in &self.latched_btns {
            f.set_btn(btn, true);
        }
        f.set_direction(self.direction);
        f.set_jog(self.jog_pos, self.jog_vel, self.jog_touch);
        f.set_rotary(self.rotary);

        // Piecewise tempo mapping centred on the dead zone.
        let mid_bias = TEMPO_CENTER as f32 - TEMPO_SPAN * 0.5;
        let raw_tempo = (self.tempo * TEMPO_SPAN
            + mid_bias * (1.0 - 2.0 * (self.tempo - 0.5).abs()))
        .round() as u16;
        f.set_tempo(raw_tempo);
        f.set_vinyl(self.vinyl_speed);
        if let Some((x, y)) = self.lcd_touch {
            f.set_touch(x, y);
        }
        f
    }

    pub(super) fn inject_jog(&mut self) {
        self.inject(self.build_current_frame().finalize());
    }

    pub(super) fn inject_rotary(&mut self) {
        self.inject(self.build_current_frame().finalize());
    }

    pub(super) fn inject_tempo(&mut self) {
        self.inject(self.build_current_frame().finalize());
    }

    pub(super) fn inject_touch(&mut self, _x: u16, _y: u16) {
        // self.lcd_touch is already updated by the caller.
        self.inject(self.build_current_frame().finalize());
    }

    pub(super) fn inject_vinyl(&mut self) {
        self.inject(self.build_current_frame().finalize());
    }
}
