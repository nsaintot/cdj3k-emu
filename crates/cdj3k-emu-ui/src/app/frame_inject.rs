//! MISO frame synthesis and per-channel inject helpers.
//!
//! Every state-changing path on `CdjApp` calls [`CdjApp::inject`] with a
//! freshly-built frame from [`CdjApp::build_current_frame`], so every frame
//! carries the held buttons. The frame is the player's own encoding of
//! [`CdjApp::panel_state`].

use cdj3k_emu_panel::miso_frame::{self, JogState, PanelState};
use cdj3k_emu_panel::MosiFrame;

use super::CdjApp;

impl CdjApp {
    pub(super) fn inject(&mut self, frame_bytes: [u8; miso_frame::MISO_SIZE]) {
        puffin::profile_function!();
        self.last_miso = frame_bytes;
        self.ctrl_stream.inject(&frame_bytes);
    }

    /// The full current control state, in no deck's coordinates.
    pub(super) fn panel_state(&self) -> PanelState {
        PanelState {
            pressed: self
                .held_btn
                .iter()
                .chain(&self.latched_btns)
                .chain(&self.scripted_btns)
                .copied()
                .collect(),
            cleared: self.cleared_bits.clone(),
            tempo: self.tempo,
            jog: JogState {
                pos: self.jog_pos,
                vel: self.jog_vel,
                touch: self.jog_touch,
                revs: self.jog_revs,
                rps: self.jog_rps,
                // The CDJ-3000's touch byte carries the hand in bit 0, the
                // release pulse included.
                touched: self.jog_touch & 0x01 != 0,
            },
            rotary: self.rotary,
            beat_loop: self.beat_loop,
            direction: self.direction,
            vinyl: self.vinyl_speed,
            touch: self.lcd_touch,
            power: true,
        }
    }

    /// The MISO frame for the full current control state. All inject paths
    /// build their frame here.
    pub(super) fn build_current_frame(&self) -> [u8; miso_frame::MISO_SIZE] {
        miso_frame::encode(self.model, &self.panel_state())
    }

    /// The current control state with the power flag dropped: the frame the
    /// guest reads as power off.
    pub(super) fn power_off_frame(&self) -> [u8; miso_frame::MISO_SIZE] {
        let state = PanelState {
            power: false,
            ..self.panel_state()
        };
        miso_frame::encode(self.model, &state)
    }

    pub(super) fn inject_jog(&mut self) {
        self.inject(self.build_current_frame());
    }

    pub(super) fn inject_rotary(&mut self) {
        self.inject(self.build_current_frame());
    }

    pub(super) fn inject_tempo(&mut self) {
        self.inject(self.build_current_frame());
    }

    /// The latest LED frame, read through the map of the player on screen.
    pub(crate) fn mosi(&self) -> MosiFrame {
        MosiFrame::new(self.led_state.frame, self.model)
    }

    /// A button by written name (see [`miso_frame::button_by_name`]).
    pub(super) fn btn_by_name(&self, name: &str) -> Option<(usize, u8)> {
        miso_frame::button_by_name(self.model, name)
    }

    /// Force `bit` low in every frame (`on`) or stop doing so, and inject.
    pub(super) fn set_bit_cleared(&mut self, bit: (usize, u8), on: bool) {
        self.cleared_bits.retain(|b| *b != bit);
        if on {
            self.cleared_bits.push(bit);
        }
        self.inject(self.build_current_frame());
    }

    /// Hold `btn` down in every frame (`on`) or release it, and inject.
    pub(super) fn set_btn_scripted(&mut self, btn: (usize, u8), on: bool) {
        self.scripted_btns.retain(|b| *b != btn);
        if on {
            self.scripted_btns.push(btn);
        }
        self.inject(self.build_current_frame());
    }

    pub(super) fn inject_touch(&mut self, _x: u16, _y: u16) {
        // self.lcd_touch is already updated by the caller.
        self.inject(self.build_current_frame());
    }

    pub(super) fn inject_vinyl(&mut self) {
        self.inject(self.build_current_frame());
    }
}
