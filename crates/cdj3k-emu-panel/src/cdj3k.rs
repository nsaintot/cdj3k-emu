//! CDJ-3000.
//!
//! The shared frame coordinates are this player's: its MISO encoder writes
//! [`fields`] where they say, and its lamps are the `LED_*` bits below and the
//! RGB bases of [`MOSI`]. The CDJ-3000X's encoder writes the same fields again,
//! 4 bytes further on, through [`write_fields`]; its lamps are a [`MosiMap`]
//! of the same shape, read by the one codec here.

use crate::button::{Btn, POWER_ON};
use crate::frame::{FrameBit, JogBrightness, MosiMap, RgbLamps, StepLedMask};
use crate::lamp::Lamp;
use crate::miso_frame::{self, fields, MisoCodec, PanelState, MISO_SIZE, ROTARY_IDLE};
use crate::mosi_frame::{
    LampState, LedProfile, LedProfiles, MosiCodec, StepLed, AZURE_BLUE_DIES, MOSI_SIZE,
};
use crate::spec::{JogBrake, ModelSpec, TouchSpace, TOUCH_FRAME_RANGE};

/// Idle frame payload (bytes 0..62, before the CRC).
pub const IDLE: [u8; 62] = {
    let mut f = [0u8; 62];
    // b02-b04: header constant
    f[2] = 0x01;
    f[3] = 0x04;
    f[4] = 0x03;
    // b12: device state (0x80=power_on | 0x01=sdcard_closed)
    f[12] = 0x81;
    // b14-b15: rotary encoder at rest - see ROTARY_IDLE
    f[fields::ROTARY] = ROTARY_IDLE.to_le_bytes()[0];
    f[fields::ROTARY + 1] = ROTARY_IDLE.to_le_bytes()[1];
    // b16-b19: LCD touch (0x0000 = no touch, LE)
    // all zero - already set by default
    // b20-b21: unknown
    f[21] = 0x7f;
    // b22-b23: tempo slider idle (0x0000 BE = 0%)
    // all zero - already set by default
    // b24: vinyl speed rotary idle
    // zero - already set by default
    // b26-b27: jog position idle (0xffff LE)
    f[fields::JOG] = 0xff;
    f[fields::JOG + 1] = 0xff;
    // b28-b29: jog velocity (16-bit LE, INVERSE: 0xffff=stopped, 0x0000=max speed)
    f[fields::JOG + 2] = 0xff;
    f[fields::JOG + 3] = 0xff;
    // b30: jog touch state (0x00=none, 0x03=press, 0x04=baseline/idle, 0x0c=turning)
    f[fields::JOG + 4] = 0x04;
    // b32-b43: capacitive sensor
    f[32] = 0x51;
    f[33] = 0x01;
    f[34] = 0xd5;
    f[35] = 0xd6;
    f[36] = 0xd6;
    f[37] = 0xd5;
    f[38] = 0xd5;
    f[39] = 0xd6;
    f[40] = 0xd6;
    f[41] = 0xd5;
    f[42] = 0xd5;
    f[43] = 0xd6;
    f
};

/// The tempo slider's raw value at the centre detent, measured at about
/// `0x7F50`, short of `0xFFFF / 2`.
pub const TEMPO_CENTER: u16 = 0x7F50;

/// Fader travel `0.0..=1.0` as the slider's raw value: piecewise, so the ends
/// reach `0x0000` and `0xFFFF` and the middle lands on [`TEMPO_CENTER`].
pub fn tempo_raw(travel: f32) -> u16 {
    const SPAN: f32 = 0xFFFF as f32;
    let mid_bias = TEMPO_CENTER as f32 - SPAN * 0.5;
    (travel * SPAN + mid_bias * (1.0 - 2.0 * (travel - 0.5).abs())).round() as u16
}

/// The CDJ-3000's analog fields for `state`, written `shift` bytes into `f`.
pub(crate) fn write_fields(f: &mut [u8; MISO_SIZE], state: &PanelState, shift: usize) {
    f[fields::DIRECTION + shift] = state.direction.as_byte();
    let jog = fields::JOG + shift;
    miso_frame::put_le(f, jog, state.jog.pos);
    miso_frame::put_le(f, jog + 2, state.jog.vel);
    f[jog + 4] = state.jog.touch;
    miso_frame::put_le(f, fields::ROTARY + shift, state.rotary);
    miso_frame::put_le(f, fields::TEMPO + shift, tempo_raw(state.tempo));
    f[fields::VINYL + shift] = state.vinyl;
    if let Some((x, y)) = state.touch {
        miso_frame::put_le(f, fields::TOUCH + shift, x);
        miso_frame::put_le(f, fields::TOUCH + shift + 2, y);
    }
}

/// The CDJ-3000's MISO encoder: [`IDLE`], the pressed bits, [`write_fields`],
/// the cleared bits, power at [`POWER_ON`], the CRC little-endian at 62.
#[derive(Debug)]
pub struct Miso;

impl MisoCodec for Miso {
    fn button(&self, btn: Btn) -> Option<FrameBit> {
        if btn.is_shared() {
            btn.bit()
        } else {
            None
        }
    }

    fn encode(&self, state: &PanelState) -> [u8; MISO_SIZE] {
        let mut f = miso_frame::from_idle(&IDLE);
        miso_frame::press(&mut f, state);
        write_fields(&mut f, state, 0);
        miso_frame::clear(&mut f, state);
        if !state.power {
            miso_frame::set_bit(&mut f, POWER_ON, false);
        }
        miso_frame::stamp_crc(&mut f, 62, false);
        f
    }
}

/// The CDJ-3000's lamp bits, in the bitfield every player of the family
/// shifts by its [`MosiMap::bit_shift`]. Single-bit: `(byte_offset, bitmask)`.
pub(crate) type LedBit = FrameBit;

// Byte 2: sync / master LEDs
pub(crate) const LED_KEY_SYNC: LedBit = (2, 0x03);
pub(crate) const LED_BEAT_SYNC: LedBit = (2, 0x08);
pub(crate) const LED_MASTER: LedBit = (2, 0x20);

// Byte 3: mode LEDs + jog illumination
// SLIP and JILMR share bits (0x03); QUANTIZE and JILMW share bits (0x0c).
pub(crate) const LED_SLIP: StepLedMask = StepLedMask::new(3, 0x10, 0x30);
pub(crate) const LED_QUANTIZE: StepLedMask = StepLedMask::new(3, 0x40, 0xc0);
pub(crate) const LED_JOG_WHITE: LedBit = (3, 0x0c); // JILMW
pub(crate) const LED_JOG_RED: LedBit = (3, 0x03); // JILMR

// Byte 4: navigation LEDs
pub(crate) const LED_SOURCE: StepLedMask = StepLedMask::new(4, 0x01, 0x03);
pub(crate) const LED_BROWSE: StepLedMask = StepLedMask::new(4, 0x04, 0x0c);
pub(crate) const LED_TAG_LIST: StepLedMask = StepLedMask::new(4, 0x10, 0x30);
pub(crate) const LED_PLAYLIST: StepLedMask = StepLedMask::new(4, 0x40, 0xc0);

// Byte 5: navigation LEDs (continued; base value 0xc0 always set - IC6003 dim)
pub(crate) const LED_SEARCH: StepLedMask = StepLedMask::new(5, 0x01, 0x03);
pub(crate) const LED_MENU: StepLedMask = StepLedMask::new(5, 0x04, 0x0c);

// Byte 7: transport + loop LEDs
pub(crate) const LED_PLAY: LedBit = (7, 0x01);
pub(crate) const LED_CUE: LedBit = (7, 0x02);
pub(crate) const LED_LOOP_IN: LedBit = (7, 0x08);
pub(crate) const LED_LOOP_OUT: LedBit = (7, 0x10);
pub(crate) const LED_RELOOP: LedBit = (7, 0x20);
pub(crate) const LED_BEAT_JUMP_4: LedBit = (7, 0x40);
pub(crate) const LED_BEAT_JUMP_8: LedBit = (7, 0x80);

// Byte 8: beat-jump direction / tempo / jog mode LEDs
pub(crate) const LED_BEAT_JUMP_NEXT: LedBit = (8, 0x01);
pub(crate) const LED_BEAT_JUMP_PREV: LedBit = (8, 0x02);
pub(crate) const LED_TEMPO_RESET: LedBit = (8, 0x08);
pub(crate) const LED_MASTER_TEMPO: LedBit = (8, 0x10);
pub(crate) const LED_JOG_MODE_CDJ: LedBit = (8, 0x40);
pub(crate) const LED_JOG_MODE_VINYL: LedBit = (8, 0x80);

// Byte 9: encoder / search / rev LEDs
pub(crate) const LED_ENCODER: LedBit = (9, 0x01);
pub(crate) const LED_TRACK_SEARCH: LedBit = (9, 0x04);
pub(crate) const LED_REV: LedBit = (9, 0x10);

/// The step a 2-bit LED field reads at an already-shifted offset.
fn step_at(frame: &[u8; MOSI_SIZE], led: StepLedMask) -> StepLed {
    let bits = frame[led.byte] & led.full;
    if bits == led.full {
        StepLed::Full
    } else if bits & led.medium != 0 {
        StepLed::Medium
    } else {
        StepLed::Off
    }
}

/// The CDJ-3000 family's codec: each lamp at its CDJ-3000 bit moved by
/// [`MosiMap::bit_shift`], or at the RGB base the map names.
impl MosiCodec for MosiMap {
    fn lamp(&self, f: &[u8; MOSI_SIZE], lamp: Lamp) -> Option<LampState> {
        let bit = |(byte, mask): LedBit| Some(LampState::Bit(f[byte + self.bit_shift] & mask != 0));
        let step = |led: StepLedMask| {
            let byte = led.byte + self.bit_shift;
            Some(LampState::Step(step_at(f, StepLedMask { byte, ..led })))
        };
        let rgb = |base: Option<usize>| base.map(|b| LampState::Rgb(f[b], f[b + 1], f[b + 2]));
        let pad = |i: usize| rgb(Some(self.lamps.hot_cue[i]));
        match lamp {
            Lamp::KeySync => bit(LED_KEY_SYNC),
            Lamp::BeatSync => bit(LED_BEAT_SYNC),
            Lamp::Master => bit(LED_MASTER),
            Lamp::Slip => step(LED_SLIP),
            Lamp::Quantize => step(LED_QUANTIZE),
            Lamp::JogRing => Some(LampState::Step(StepLed::from_level(match self.jog {
                JogBrightness::Bits { byte, dim, bright } => {
                    match f[byte + self.bit_shift] & (dim | bright) {
                        0 => 0,
                        v if v == dim => 1,
                        _ => 2,
                    }
                }
                JogBrightness::Level { byte } => f[byte],
            }))),
            Lamp::JogRed => bit(LED_JOG_RED),
            Lamp::Source => step(LED_SOURCE),
            Lamp::Browse => step(LED_BROWSE),
            Lamp::TagList => step(LED_TAG_LIST),
            Lamp::Playlist => step(LED_PLAYLIST),
            Lamp::Search => step(LED_SEARCH),
            Lamp::Menu => step(LED_MENU),
            // A colour on a player whose map gives one, else the bit.
            Lamp::Play => self
                .lamps
                .play
                .map_or_else(|| bit(LED_PLAY), |b| rgb(Some(b))),
            Lamp::Cue => self
                .lamps
                .cue
                .map_or_else(|| bit(LED_CUE), |b| rgb(Some(b))),
            Lamp::LoopIn => bit(LED_LOOP_IN),
            Lamp::LoopOut => bit(LED_LOOP_OUT),
            Lamp::Reloop => bit(LED_RELOOP),
            Lamp::BeatJump4 => bit(LED_BEAT_JUMP_4),
            Lamp::BeatJump8 => bit(LED_BEAT_JUMP_8),
            Lamp::BeatJumpNext => bit(LED_BEAT_JUMP_NEXT),
            Lamp::BeatJumpPrev => bit(LED_BEAT_JUMP_PREV),
            Lamp::TempoReset => bit(LED_TEMPO_RESET),
            Lamp::MasterTempo => bit(LED_MASTER_TEMPO),
            Lamp::JogModeCdj => bit(LED_JOG_MODE_CDJ),
            Lamp::JogModeVinyl => bit(LED_JOG_MODE_VINYL),
            Lamp::Encoder => bit(LED_ENCODER),
            Lamp::TrackSearch => bit(LED_TRACK_SEARCH),
            Lamp::Rev => bit(LED_REV),
            Lamp::HotA => pad(0),
            Lamp::HotB => pad(1),
            Lamp::HotC => pad(2),
            Lamp::HotD => pad(3),
            Lamp::HotE => pad(4),
            Lamp::HotF => pad(5),
            Lamp::HotG => pad(6),
            Lamp::HotH => pad(7),
            Lamp::Slot1 => rgb(self.lamps.slot_1),
            Lamp::Slot2 => rgb(self.lamps.slot_2),
            Lamp::OnAir => rgb(self.lamps.on_air),
            Lamp::Rotary => rgb(self.lamps.rotary),
            Lamp::JogMeter | Lamp::Standby | Lamp::Eject => None,
        }
    }
}

/// The CDJ-3000's lamps.
pub const MOSI: MosiMap = MosiMap {
    bit_shift: 0,
    lamps: RgbLamps {
        slot_1: Some(36),
        slot_2: Some(39),
        on_air: Some(42),
        // PLAY and CUE are bitfield bits here.
        play: None,
        cue: None,
        // The selector ring is unlit.
        rotary: None,
        hot_cue: [12, 15, 18, 21, 24, 27, 30, 33],
    },
    jog: JogBrightness::Bits {
        byte: LED_JOG_WHITE.0,
        dim: 0x04,
        bright: 0x08,
    },
};

pub const SPEC: ModelSpec = ModelSpec {
    title: "CDJ-3000",
    slug: "cdj3k",
    aliases: &["3000", "3k"],
    main_lcd: (1280, 720),
    touch: TouchSpace::SubCpu {
        range: TOUCH_FRAME_RANGE as u16,
    },
    // Not verified against hardware.
    model_env: "CDJ3K-RK3399",
    emmc_index: 1,
    system_rev_env: Some("rev_kernel"),
    firmware_file_names: &["CDJ3K"],
    miso: &Miso,
    mosi: &MOSI,
    // One LED part throughout. Its white `44 78 7f` cuts back the red die,
    // the strongest.
    leds: LedProfiles::uniform(LedProfile {
        dies: AZURE_BLUE_DIES,
        gamma: 1.0,
        white: [0x44, 0x78, 0x7f],
        balance_whites_only: true,
    }),
    jog_brake: JogBrake::Adjustable,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdj3kx;
    use crate::mosi_frame::MOSI_SIZE;

    /// Each lamp of a map names a byte of its own.
    #[test]
    fn a_maps_lamps_do_not_overlap() {
        for (name, map) in [("CDJ-3000", MOSI), ("CDJ-3000X", cdj3kx::MOSI)] {
            let lamps = map.lamps;
            let mut bases: Vec<usize> = lamps.hot_cue.to_vec();
            bases.extend(
                [
                    lamps.slot_1,
                    lamps.slot_2,
                    lamps.on_air,
                    lamps.play,
                    lamps.cue,
                    lamps.rotary,
                ]
                .into_iter()
                .flatten(),
            );
            for (i, a) in bases.iter().enumerate() {
                assert!(a + 2 < MOSI_SIZE, "{name} lamp at {a} runs off the frame");
                for b in &bases[i + 1..] {
                    assert!(a.abs_diff(*b) >= 3, "{name} lamps at {a} and {b} overlap");
                }
            }
        }
    }
}
