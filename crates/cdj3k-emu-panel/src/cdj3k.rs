//! CDJ-3000.
//!
//! The shared frame coordinates are the CDJ-3000's. Its MISO encoder writes
//! each field at the offset given in [`fields`], and its lamps are the `LED_*`
//! bits below and the RGB bases in [`MOSI`]. The CDJ-3000X's encoder writes the
//! same fields 4 bytes later through [`write_fields`]; its lamps are a
//! [`MosiMap`] of the same shape, decoded by the codec in this file.

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
    // b02-b03: header constant. b04: direction rocker, 0x03 (FWD).
    f[2] = 0x01;
    f[3] = 0x04;
    f[4] = 0x03;
    // b12: device state; 0x80 is power on and 0x01 is SD card cover closed.
    f[12] = 0x81;
    // b14-b15: rotary encoder at rest (see ROTARY_IDLE)
    f[fields::ROTARY] = ROTARY_IDLE.to_le_bytes()[0];
    f[fields::ROTARY + 1] = ROTARY_IDLE.to_le_bytes()[1];
    // b16-b19: LCD touch, LE; 0x0000 means no touch.
    // b20-b21: unknown
    f[21] = 0x7f;
    // b22-b23: tempo slider idle, 0x0000 LE (0%)
    // b24: vinyl speed rotary idle, 0x00
    // b26-b27: jog position idle (0xffff LE)
    f[fields::JOG] = 0xff;
    f[fields::JOG + 1] = 0xff;
    // b28-b29: jog velocity, 16-bit LE and inverted: 0xffff is stopped and
    // 0x0000 is maximum speed.
    f[fields::JOG + 2] = 0xff;
    f[fields::JOG + 3] = 0xff;
    // b30: jog touch state (0x00 none, 0x03 press, 0x04 idle, 0x0c turning)
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

/// Converts fader travel `0.0..=1.0` to the slider's raw value. The mapping is
/// piecewise, so the ends reach `0x0000` and `0xFFFF` and the middle lands on
/// [`TEMPO_CENTER`].
pub fn tempo_raw(travel: f32) -> u16 {
    const SPAN: f32 = 0xFFFF as f32;
    let mid_bias = TEMPO_CENTER as f32 - SPAN * 0.5;
    (travel * SPAN + mid_bias * (1.0 - 2.0 * (travel - 0.5).abs())).round() as u16
}

/// Writes the CDJ-3000's analog fields for `state` into `f`, offset by `shift`
/// bytes.
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

/// The CDJ-3000's MISO encoder. It starts from [`IDLE`], sets the pressed bits,
/// writes the fields with [`write_fields`], clears the released bits, clears
/// [`POWER_ON`] when the power is off and writes the CRC little-endian at byte 62.
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
/// shifts by its [`MosiMap::bit_shift`]. Each one is `(byte_offset, bitmask)`.
pub(crate) type LedBit = FrameBit;

// Byte 2: sync and master LEDs
pub(crate) const LED_KEY_SYNC: LedBit = (2, 0x03);
pub(crate) const LED_BEAT_SYNC: LedBit = (2, 0x08);
pub(crate) const LED_MASTER: LedBit = (2, 0x20);

// Byte 3: mode LEDs and jog illumination
// SLIP and QUANTIZE share the byte with the red (JILMR) and white (JILMW) jog
// illumination fields.
pub(crate) const LED_SLIP: StepLedMask = StepLedMask::new(3, 0x10, 0x30);
pub(crate) const LED_QUANTIZE: StepLedMask = StepLedMask::new(3, 0x40, 0xc0);
pub(crate) const LED_JOG_WHITE: LedBit = (3, 0x0c); // JILMW
pub(crate) const LED_JOG_RED: LedBit = (3, 0x03); // JILMR

// Byte 4: navigation LEDs
pub(crate) const LED_SOURCE: StepLedMask = StepLedMask::new(4, 0x01, 0x03);
pub(crate) const LED_BROWSE: StepLedMask = StepLedMask::new(4, 0x04, 0x0c);
pub(crate) const LED_TAG_LIST: StepLedMask = StepLedMask::new(4, 0x10, 0x30);
pub(crate) const LED_PLAYLIST: StepLedMask = StepLedMask::new(4, 0x40, 0xc0);

// Byte 5: more navigation LEDs. The base value 0xc0, the IC6003 dimming, is
// always set.
pub(crate) const LED_SEARCH: StepLedMask = StepLedMask::new(5, 0x01, 0x03);
pub(crate) const LED_MENU: StepLedMask = StepLedMask::new(5, 0x04, 0x0c);

// Byte 7: transport and loop LEDs
pub(crate) const LED_PLAY: LedBit = (7, 0x01);
pub(crate) const LED_CUE: LedBit = (7, 0x02);
pub(crate) const LED_LOOP_IN: LedBit = (7, 0x08);
pub(crate) const LED_LOOP_OUT: LedBit = (7, 0x10);
pub(crate) const LED_RELOOP: LedBit = (7, 0x20);
pub(crate) const LED_BEAT_JUMP_4: LedBit = (7, 0x40);
pub(crate) const LED_BEAT_JUMP_8: LedBit = (7, 0x80);

// Byte 8: beat jump direction, tempo and jog mode LEDs
pub(crate) const LED_BEAT_JUMP_NEXT: LedBit = (8, 0x01);
pub(crate) const LED_BEAT_JUMP_PREV: LedBit = (8, 0x02);
pub(crate) const LED_TEMPO_RESET: LedBit = (8, 0x08);
pub(crate) const LED_MASTER_TEMPO: LedBit = (8, 0x10);
pub(crate) const LED_JOG_MODE_CDJ: LedBit = (8, 0x40);
pub(crate) const LED_JOG_MODE_VINYL: LedBit = (8, 0x80);

// Byte 9: encoder, search and REV LEDs
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

/// The jog ring's step, from the level `jog` encodes: 0 off, 1 dim, 2 full.
fn jog_step(frame: &[u8; MOSI_SIZE], jog: JogBrightness, bit_shift: usize) -> StepLed {
    StepLed::from_level(match jog {
        JogBrightness::Bits { byte, dim, bright } => {
            match frame[byte + bit_shift] & (dim | bright) {
                0 => 0,
                v if v == dim => 1,
                _ => 2,
            }
        }
        JogBrightness::Level { byte } => frame[byte],
    })
}

/// Where the CDJ-3000 family keeps a lamp.
#[derive(Copy, Clone)]
enum Wire {
    Bit(LedBit),
    Step(StepLedMask),
    /// The ring's level, as [`MosiMap::jog`] encodes it.
    JogRing,
    /// A colour at the base the map gives, or the bit when the map gives none.
    RgbOrBit(fn(&RgbLamps) -> Option<usize>, LedBit),
    /// HOT CUE pad, A = 0.
    Pad(usize),
    Rgb(fn(&RgbLamps) -> Option<usize>),
    Absent,
}

/// Each lamp's wire, in [`Lamp::ALL`] order: `WIRING[lamp as usize]`.
const WIRING: [(Lamp, Wire); Lamp::ALL.len()] = [
    (Lamp::KeySync, Wire::Bit(LED_KEY_SYNC)),
    (Lamp::BeatSync, Wire::Bit(LED_BEAT_SYNC)),
    (Lamp::Master, Wire::Bit(LED_MASTER)),
    (Lamp::Slip, Wire::Step(LED_SLIP)),
    (Lamp::Quantize, Wire::Step(LED_QUANTIZE)),
    (Lamp::JogRing, Wire::JogRing),
    (Lamp::JogRed, Wire::Bit(LED_JOG_RED)),
    (Lamp::JogMeter, Wire::Absent),
    (Lamp::Source, Wire::Step(LED_SOURCE)),
    (Lamp::Browse, Wire::Step(LED_BROWSE)),
    (Lamp::TagList, Wire::Step(LED_TAG_LIST)),
    (Lamp::Playlist, Wire::Step(LED_PLAYLIST)),
    (Lamp::Search, Wire::Step(LED_SEARCH)),
    (Lamp::Menu, Wire::Step(LED_MENU)),
    (Lamp::Play, Wire::RgbOrBit(|l| l.play, LED_PLAY)),
    (Lamp::Cue, Wire::RgbOrBit(|l| l.cue, LED_CUE)),
    (Lamp::LoopIn, Wire::Bit(LED_LOOP_IN)),
    (Lamp::LoopOut, Wire::Bit(LED_LOOP_OUT)),
    (Lamp::Reloop, Wire::Bit(LED_RELOOP)),
    (Lamp::BeatJump4, Wire::Bit(LED_BEAT_JUMP_4)),
    (Lamp::BeatJump8, Wire::Bit(LED_BEAT_JUMP_8)),
    (Lamp::BeatJumpNext, Wire::Bit(LED_BEAT_JUMP_NEXT)),
    (Lamp::BeatJumpPrev, Wire::Bit(LED_BEAT_JUMP_PREV)),
    (Lamp::TempoReset, Wire::Bit(LED_TEMPO_RESET)),
    (Lamp::MasterTempo, Wire::Bit(LED_MASTER_TEMPO)),
    (Lamp::JogModeCdj, Wire::Bit(LED_JOG_MODE_CDJ)),
    (Lamp::JogModeVinyl, Wire::Bit(LED_JOG_MODE_VINYL)),
    (Lamp::Encoder, Wire::Bit(LED_ENCODER)),
    (Lamp::TrackSearch, Wire::Bit(LED_TRACK_SEARCH)),
    (Lamp::Rev, Wire::Bit(LED_REV)),
    (Lamp::HotA, Wire::Pad(0)),
    (Lamp::HotB, Wire::Pad(1)),
    (Lamp::HotC, Wire::Pad(2)),
    (Lamp::HotD, Wire::Pad(3)),
    (Lamp::HotE, Wire::Pad(4)),
    (Lamp::HotF, Wire::Pad(5)),
    (Lamp::HotG, Wire::Pad(6)),
    (Lamp::HotH, Wire::Pad(7)),
    (Lamp::Slot1, Wire::Rgb(|l| l.slot_1)),
    (Lamp::Slot2, Wire::Rgb(|l| l.slot_2)),
    (Lamp::OnAir, Wire::Rgb(|l| l.on_air)),
    (Lamp::Rotary, Wire::Rgb(|l| l.rotary)),
    (Lamp::Standby, Wire::Absent),
    (Lamp::Eject, Wire::Absent),
];

/// The CDJ-3000 family's codec: each lamp at its CDJ-3000 bit moved by
/// [`MosiMap::bit_shift`], or at the RGB base the map names.
impl MosiCodec for MosiMap {
    fn lamp(&self, f: &[u8; MOSI_SIZE], lamp: Lamp) -> Option<LampState> {
        let bit = |(byte, mask): LedBit| Some(LampState::Bit(f[byte + self.bit_shift] & mask != 0));
        let rgb = |base: Option<usize>| base.map(|b| LampState::Rgb(f[b], f[b + 1], f[b + 2]));
        match WIRING[lamp as usize].1 {
            Wire::Bit(b) => bit(b),
            Wire::Step(led) => {
                let byte = led.byte + self.bit_shift;
                Some(LampState::Step(step_at(f, StepLedMask { byte, ..led })))
            }
            Wire::JogRing => Some(LampState::Step(jog_step(f, self.jog, self.bit_shift))),
            Wire::RgbOrBit(base, b) => base(&self.lamps).map_or_else(|| bit(b), |at| rgb(Some(at))),
            Wire::Pad(i) => rgb(Some(self.lamps.hot_cue[i])),
            Wire::Rgb(base) => rgb(base(&self.lamps)),
            Wire::Absent => None,
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
    // The player uses one LED part throughout. Its white point `44 78 7f`
    // reduces the red die, which is the strongest.
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

    /// `WIRING[lamp as usize]` is that lamp's wire.
    #[test]
    fn wiring_follows_lamp_order() {
        for (i, (lamp, _)) in WIRING.iter().enumerate() {
            assert_eq!(*lamp, Lamp::ALL[i]);
            assert_eq!(*lamp as usize, i, "{lamp:?}");
        }
    }
}
