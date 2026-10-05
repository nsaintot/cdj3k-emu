//! CDJ-1500X.
//!
//! Its MISO frame is its own: the app reads 32 bytes from
//! `/dev/subucom_spi3.0`, a non-zero format version at byte 0, data to byte
//! 23 and a CRC-16/X-25 over them, big-endian at 24. Bytes 26-31 are not
//! checked: the touch words ride at 26 (see the guest shim's touch path) and
//! the emulator's power flag at 30 (the guest forwarder's power-off check).
//! Buttons are set while pressed. The two encoders (BROWSE at 20-21, BEAT
//! LOOP at 8-9) are absolute counters, big-endian; the deck differentiates
//! them and swallows the first frame, so where they start does not matter.
//! Byte 19 bit 2 flags the jog microcontroller as failed ("JOG uCom : NG" in
//! service mode) and stays clear; bits 0 and 1 are the jog's touch and its
//! direction of rotation.
//! The tempo fader is two 14-bit words, big-endian in the low 14 bits: the
//! wiper at 4-5 and the unit's centre-detent reading at 6-7. The deck places
//! the wiper on a 0-1023 scale, 511 at the detent, linear from 0 up to it and
//! from it up to [`TEMPO_TOP`]. The jog is a position counter at 12-13 and
//! the time between its encoder pulses at 14-15, in microseconds, both
//! big-endian, `0xFFFF` stopped. The deck reads `1111.11 / word` as the jog's
//! speed in seconds of track a second, moves the track by it over time and
//! ignores the counter: the period has to follow the platter's actual motion,
//! slow drags included. One platter turn is [`JOG_SPEED_PER_TURN`] seconds of
//! track.
//!
//! Its MOSI frame is 32 bytes on the same node, 0-13 meaningful: PLAY, CUE
//! and the power lamp as bits, the hot cue pads and the EJECT lamp as 2-bit
//! levels (3 is full), the jog's ring meter as a value, the front casing's
//! light in the USB cavity as a colour, then two state bytes and a big-endian
//! CRC.
//! Byte 8 and the auto-standby request (byte 8 = 2, byte 10 = 0x53) are deck
//! state for the sub-CPU; byte 9 bits 0-2 are the sub-CPU test pattern
//! (service mode page 1) and bit 3 is always set.
//! Service mode's LED ALL ON page sends `c0 ff ff 63 ff ff ff 00 00 e8`.
//! BROWSE, BEAT LOOP and CALL/DELETE are not lit; the phone pad's bar is not
//! driven (always lit white).
//!
//! Service mode holds SHIFT + HOT CUE B (`pre-setting.sh`). Its page 1
//! releases the page keys (BACK, TAG TRACK) once DELETE has been pressed and
//! released on it.

use crate::button::Btn;
use crate::cdj3kx;
use crate::frame::FrameBit;
use crate::lamp::Lamp;
use crate::miso_frame::{self, MisoCodec, PanelState, MISO_SIZE};
use crate::mosi_frame::{LampState, MosiCodec, StepLed, MOSI_SIZE};
use crate::spec::{JogBrake, ModelSpec, TouchSpace};

/// The emulator's power flag; clear, the guest forwarder powers the deck off.
pub const POWER_FLAG: FrameBit = (30, 0x80);

/// Where the two big-endian encoder counters sit: BROWSE (rotarySelector) and
/// BEAT LOOP.
const BROWSE_AT: usize = 20;
const BEAT_LOOP_AT: usize = 8;

/// Where the tempo wiper and the detent reading sit.
const TEMPO_AT: usize = 4;
const TEMPO_DETENT_AT: usize = 6;
/// The wiper at the end of travel.
pub const TEMPO_TOP: u16 = 0x3FF0;
/// The detent reading: half travel, so both halves of the fader span the same.
pub const TEMPO_DETENT: u16 = TEMPO_TOP / 2;

/// The jog's position counter and pulse period, and the touch and forward
/// bits beside the jog microcontroller's failure flag (19 b2, kept clear).
const JOG_POS_AT: usize = 12;
const JOG_PERIOD_AT: usize = 14;
const JOG_TOUCH: FrameBit = (19, 0x01);
const JOG_FORWARD: FrameBit = (19, 0x02);
/// The deck's jog speed for the platter turning once a second, in seconds of
/// track a second, matched by hand so a turn of the platter on screen follows
/// the hub ring (which the deck turns once per 1.8 s of track).
pub const JOG_SPEED_PER_TURN: f64 = 0.8;
/// Encoder pulses a turn: the deck reads `1111.11 / period_us` as its speed,
/// 900 pulses a second per unit.
pub const JOG_PULSES_PER_REV: f64 = 900.0 * JOG_SPEED_PER_TURN;
/// Seconds the platter takes to stop from one turn a second: service mode's
/// jog load check wants its speed to fall from 3 to 1.5 in 100-150 ms, and
/// this brake takes 125.
const JOG_STOP_SECS: f32 = 0.125 * JOG_SPEED_PER_TURN as f32 / 1.5;
/// The period word of a stopped jog.
const JOG_STOPPED: u16 = 0xFFFF;

/// The jog's period word for `rps` turns a second.
fn jog_period(rps: f32) -> u16 {
    let us = 1.0e6 / (JOG_PULSES_PER_REV * f64::from(rps.abs()));
    if us.is_finite() && us < f64::from(JOG_STOPPED) {
        us.round() as u16
    } else {
        JOG_STOPPED
    }
}

/// A 14-bit tempo word at `at`, high byte first.
fn put_tempo_word(f: &mut [u8; MISO_SIZE], at: usize, v: u16) {
    miso_frame::put_be(f, at, v & 0x3FFF);
}

/// Idle frame payload: the format version and the power flag, nothing
/// pressed.
pub const IDLE: [u8; 62] = {
    let mut f = [0u8; 62];
    f[0] = 0x01;
    f[POWER_FLAG.0] = POWER_FLAG.1;
    f
};

/// Every button, by bit, from the EP166 `RxFormatField` dispatcher. Pads A-H
/// run down byte 18 from b7; the two encoder pushes are RELOOP (BEAT LOOP) and
/// ROTARY_PRESS (BROWSE). DELETE is the app's CALL/DELETE control. EJECT is
/// [`Btn::UsbStop`]; the physical power button at 23 b0 is not drawn.
const BUTTONS: &[(Btn, FrameBit)] = &[
    (Btn::Reloop, (10, 0x80)),
    (Btn::Delete, (16, 0x08)),
    (Btn::Play, (16, 0x40)),
    (Btn::Cue, (16, 0x80)),
    (Btn::Shift, (17, 0x01)),
    (Btn::HotA, (18, 0x80)),
    (Btn::HotB, (18, 0x40)),
    (Btn::HotC, (18, 0x20)),
    (Btn::HotD, (18, 0x10)),
    (Btn::HotE, (18, 0x08)),
    (Btn::HotF, (18, 0x04)),
    (Btn::HotG, (18, 0x02)),
    (Btn::HotH, (18, 0x01)),
    (Btn::Back, (22, 0x01)),
    (Btn::TagTrack, (22, 0x02)),
    (Btn::RotaryPress, (22, 0x80)),
    (Btn::UsbStop, (23, 0x02)),
];

/// Where the frame's CRC sits: over every byte before it, big-endian.
const CRC_AT: usize = 24;

/// Where the shim reads the touch words, u16 LE X then Y.
const TOUCH_AT: usize = 26;

/// The CDJ-1500X's MISO encoder: [`IDLE`], the pressed bits, the touch
/// words, the cleared bits, power at [`POWER_FLAG`], the CRC.
#[derive(Debug)]
pub struct Miso;

impl MisoCodec for Miso {
    fn button(&self, btn: Btn) -> Option<FrameBit> {
        BUTTONS.iter().find(|(b, _)| *b == btn).map(|(_, bit)| *bit)
    }

    fn encode(&self, state: &PanelState) -> [u8; MISO_SIZE] {
        let mut f = miso_frame::from_idle(&IDLE);
        miso_frame::press(&mut f, state);
        miso_frame::put_be(&mut f, BROWSE_AT, state.rotary);
        miso_frame::put_be(&mut f, BEAT_LOOP_AT, state.beat_loop);
        let wiper = (state.tempo.clamp(0.0, 1.0) * TEMPO_TOP as f32).round() as u16;
        put_tempo_word(&mut f, TEMPO_AT, wiper);
        put_tempo_word(&mut f, TEMPO_DETENT_AT, TEMPO_DETENT);
        let jog = &state.jog;
        let pulses = (jog.revs * JOG_PULSES_PER_REV).floor().rem_euclid(65536.0) as u16;
        miso_frame::put_be(&mut f, JOG_POS_AT, pulses);
        miso_frame::put_be(&mut f, JOG_PERIOD_AT, jog_period(jog.rps));
        miso_frame::set_bit(&mut f, JOG_TOUCH, jog.touched);
        miso_frame::set_bit(&mut f, JOG_FORWARD, jog.rps > 0.0);
        if let Some((x, y)) = state.touch {
            miso_frame::put_le(&mut f, TOUCH_AT, x);
            miso_frame::put_le(&mut f, TOUCH_AT + 2, y);
        }
        miso_frame::clear(&mut f, state);
        if !state.power {
            miso_frame::set_bit(&mut f, POWER_FLAG, false);
        }
        miso_frame::stamp_crc(&mut f, CRC_AT, true);
        f
    }
}

/// Lamp bits, each read off service mode's operator check, which lights a
/// control's lamp until the control is pressed. The power lamp left of DELETE
/// goes out with DELETE.
const LED_PLAY: FrameBit = (0, 0x40);
const LED_CUE: FrameBit = (0, 0x80);
const LED_STANDBY: FrameBit = (9, 0x20);

/// Where the hot cue pads' 2-bit steps start: A-D in byte 1, E-H in byte 2,
/// the high pair first.
const PADS_AT: usize = 1;
/// The EJECT key's lamp, byte 9 bits 6-7.
const EJECT_STEP: (usize, u32) = (9, 6);
/// The jog's ring meter: 0 off to 0x63.
const JOG_METER_AT: usize = 3;
/// The jog hub's LED ring as byte 3 drives it: 72 bars lit two at a time, so
/// 36 positions.
pub const JOG_RING_POSITIONS: u8 = 36;
/// Byte 3 for the first position; the next 35 follow it.
const JOG_RING_FIRST: u8 = 0x29;
/// Byte 3 with every bar lit (service mode's LED ALL ON).
const JOG_RING_ALL: u8 = 0x63;

/// What the jog hub's LED ring shows.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum JogRing {
    Off,
    /// Every bar.
    All,
    /// The pair of bars at this position, 0 at the top and clockwise.
    At(u8),
}

/// The ring for a byte-3 meter value. A value outside the positions and
/// [`JogRing::All`] has not been seen lit and reads off.
pub fn jog_ring(meter: u8) -> JogRing {
    match meter {
        JOG_RING_ALL => JogRing::All,
        v if (JOG_RING_FIRST..JOG_RING_FIRST + JOG_RING_POSITIONS).contains(&v) => {
            JogRing::At(v - JOG_RING_FIRST)
        }
        _ => JogRing::Off,
    }
}

/// The front casing's light in the USB cavity, R G B.
const SLOT_AT: usize = 4;

/// The CDJ-1500X's MOSI decoder.
#[derive(Debug)]
pub struct Mosi;

impl MosiCodec for Mosi {
    fn lamp(&self, f: &[u8; MOSI_SIZE], lamp: Lamp) -> Option<LampState> {
        let bit = |(byte, mask): FrameBit| Some(LampState::Bit(f[byte] & mask != 0));
        // A 2-bit level: 0 off, 3 full, between dim.
        let step = |byte: usize, shift: u32| {
            Some(LampState::Step(match (f[byte] >> shift) & 0x03 {
                0 => StepLed::Off,
                3 => StepLed::Full,
                _ => StepLed::Medium,
            }))
        };
        let pad = |i: usize| step(PADS_AT + i / 4, 6 - 2 * (i % 4) as u32);
        match lamp {
            Lamp::Play => bit(LED_PLAY),
            Lamp::Cue => bit(LED_CUE),
            Lamp::Standby => bit(LED_STANDBY),
            Lamp::Slot1 => Some(LampState::Rgb(f[SLOT_AT], f[SLOT_AT + 1], f[SLOT_AT + 2])),
            Lamp::Eject => step(EJECT_STEP.0, EJECT_STEP.1),
            Lamp::JogMeter => Some(LampState::Level(f[JOG_METER_AT])),
            Lamp::HotA => pad(0),
            Lamp::HotB => pad(1),
            Lamp::HotC => pad(2),
            Lamp::HotD => pad(3),
            Lamp::HotE => pad(4),
            Lamp::HotF => pad(5),
            Lamp::HotG => pad(6),
            Lamp::HotH => pad(7),
            _ => None,
        }
    }
}

pub const SPEC: ModelSpec = ModelSpec {
    title: "CDJ-1500X",
    slug: "cdj1500x",
    aliases: &["1500x", "1k5x"],
    // The CDJ-3000X's 10.1-inch panel.
    main_lcd: (1280, 800),
    touch: TouchSpace::ScreenPixels,
    // Read from a real unit: the product UUID.
    model_env: "6516d11b-fad9-4f28-9a01-fd2af5e1ef40",
    emmc_index: 0,
    // The dumped unit's environment carries no `rev_*` variable.
    system_rev_env: None,
    // `CDJ1500Xv110.UPD`; the updater also takes `EP166v….UPD`.
    firmware_file_names: &["CDJ1500X", "EP166"],
    miso: &Miso,
    mosi: &Mosi,
    leds: cdj3kx::SPEC.leds,
    // No JOG ADJUST knob: a fixed brake that passes service mode's jog load
    // check.
    jog_brake: JogBrake::Fixed {
        stop_secs: JOG_STOP_SECS,
    },
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::miso_frame::JogState;
    use crate::{Model, MosiFrame};

    /// Each button is listed once, at a bit of its own, before the CRC, and
    /// resolves through the model.
    #[test]
    fn each_button_has_its_own_bit_before_the_crc() {
        for (i, (a, bit)) in BUTTONS.iter().enumerate() {
            assert_eq!(miso_frame::button(Model::Cdj1500x, *a), Some(*bit));
            assert!(bit.0 < CRC_AT, "{} is past the CRC", a.name());
            for (b, other) in &BUTTONS[i + 1..] {
                assert_ne!(a, b, "{} is listed twice", a.name());
                assert_ne!(bit, other, "{} and {} share a bit", a.name(), b.name());
            }
        }
    }

    /// The period word follows the speed service mode's load check reads,
    /// `1111.11 / word`, at [`JOG_SPEED_PER_TURN`] a turn a second, and a
    /// stopped jog sends `0xFFFF`.
    #[test]
    fn the_jog_period_is_the_pulse_time_at_the_platters_speed() {
        let rps = (11.111 / JOG_SPEED_PER_TURN) as f32;
        assert_eq!(jog_period(0.0), JOG_STOPPED);
        assert_eq!(jog_period(rps), 100);
        assert_eq!(jog_period(-rps), 100);
        assert_eq!(jog_period(0.001), JOG_STOPPED);
        let at = |revs: f64, rps: f32, touched: bool| {
            miso_frame::encode(
                Model::Cdj1500x,
                &PanelState {
                    jog: JogState {
                        revs,
                        rps,
                        touched,
                        ..PanelState::at_rest().jog
                    },
                    ..PanelState::at_rest()
                },
            )
        };
        let f = at(2.0, 1.0, true);
        assert_eq!(
            u16::from_be_bytes([f[12], f[13]]),
            (2.0 * JOG_PULSES_PER_REV) as u16
        );
        assert_eq!(f[19], 0x03);
        // Backward from the start wraps the counter; the jog uCom flag stays
        // clear.
        let f = at(-1.0 / JOG_PULSES_PER_REV, -1.0, false);
        assert_eq!(u16::from_be_bytes([f[12], f[13]]), 0xFFFF);
        assert_eq!(f[19], 0x00);
    }

    /// The window service mode's jog load check passes for the time the jog's
    /// speed takes to fall from 3 to 1.5.
    const JOG_LOAD_CHECK_MS: std::ops::RangeInclusive<f32> = 100.0..=150.0;

    /// The fixed brake passes the factory jog load check: the deck's speed
    /// falling from 3 to 1.5 is the platter slowing by
    /// `1.5 / JOG_SPEED_PER_TURN` turns a second, which under linear
    /// deceleration takes that many `stop_secs`.
    #[test]
    fn the_fixed_brake_passes_the_jog_load_check() {
        let JogBrake::Fixed { stop_secs } = SPEC.jog_brake else {
            panic!("the CDJ-1500X has no JOG ADJUST knob");
        };
        let ms = 1.5 / JOG_SPEED_PER_TURN as f32 * stop_secs * 1000.0;
        assert!(JOG_LOAD_CHECK_MS.contains(&ms), "{ms} ms");
    }

    /// The fader's ends and its detent land where service mode reads 0, 1023
    /// and 511: the wiper at 0, [`TEMPO_TOP`] and the detent reading.
    #[test]
    fn the_tempo_wiper_spans_the_travel_around_the_detent() {
        let word = |f: &[u8; MISO_SIZE], at: usize| u16::from_be_bytes([f[at], f[at + 1]]);
        for (travel, wiper) in [(0.0, 0), (0.5, TEMPO_DETENT), (1.0, TEMPO_TOP)] {
            let f = miso_frame::encode(
                Model::Cdj1500x,
                &PanelState {
                    tempo: travel,
                    ..PanelState::at_rest()
                },
            );
            assert_eq!(word(&f, TEMPO_AT), wiper, "travel {travel}");
            assert_eq!(word(&f, TEMPO_DETENT_AT), TEMPO_DETENT);
        }
    }

    /// Playback walks byte 3 from 0x29 to 0x4C, one position per value.
    #[test]
    fn the_jog_ring_has_thirty_six_positions() {
        assert_eq!(jog_ring(0), JogRing::Off);
        assert_eq!(jog_ring(0x29), JogRing::At(0));
        assert_eq!(jog_ring(0x4c), JogRing::At(JOG_RING_POSITIONS - 1));
        assert_eq!(jog_ring(0x4d), JogRing::Off);
        assert_eq!(jog_ring(0x63), JogRing::All);
    }

    /// Each pad reads its own 2-bit level, the high pair first.
    #[test]
    fn each_pad_reads_its_own_level() {
        for (i, pad) in Lamp::HOT_CUES.into_iter().enumerate() {
            for (level, want) in [
                (1, StepLed::Medium),
                (2, StepLed::Medium),
                (3, StepLed::Full),
            ] {
                let mut raw = [0u8; MOSI_SIZE];
                raw[PADS_AT + i / 4] = level << (6 - 2 * (i % 4));
                let f = MosiFrame::new(raw, Model::Cdj1500x);
                assert_eq!(f.step(pad), want, "{pad:?}");
                for &other in Lamp::ALL.iter().filter(|&&l| l != pad) {
                    assert!(!f.lit(other), "{pad:?} lights {other:?}");
                }
            }
        }
    }

    /// LED ALL ON, as service mode sends it, lights every lamp at full.
    #[test]
    fn led_all_on_lights_every_lamp() {
        let mut raw = [0u8; MOSI_SIZE];
        raw[..14].copy_from_slice(&[
            0xc0, 0xff, 0xff, 0x63, 0xff, 0xff, 0xff, 0x00, 0x00, 0xe8, 0x00, 0x00, 0xe0, 0xcd,
        ]);
        let f = MosiFrame::new(raw, Model::Cdj1500x);
        for pad in Lamp::HOT_CUES {
            assert_eq!(f.step(pad), StepLed::Full, "{pad:?}");
        }
        assert_eq!(f.step(Lamp::Eject), StepLed::Full);
        assert!(f.lit(Lamp::Cue));
        assert!(f.lit(Lamp::Play));
        assert!(f.lit(Lamp::Standby));
        assert_eq!(f.lamp(Lamp::JogMeter), Some(LampState::Level(0x63)));
        assert_eq!(f.rgb(Lamp::Slot1), Some((0xff, 0xff, 0xff)));
    }

    /// Each control's lamp goes out with the control in the operator check:
    /// the frames are the ones read after PLAY, CUE, EJECT and DELETE.
    #[test]
    fn the_operator_check_clears_each_lamp_with_its_control() {
        let frame = |head: [u8; 10]| {
            let mut raw = [0u8; MOSI_SIZE];
            raw[..10].copy_from_slice(&head);
            MosiFrame::new(raw, Model::Cdj1500x)
        };
        let all = frame([0xc0, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0xeb]);
        for lamp in [Lamp::Play, Lamp::Cue, Lamp::Eject, Lamp::Standby] {
            assert!(all.lit(lamp), "{lamp:?}");
        }
        let no_play = frame([0x80, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0xeb]);
        assert!(!no_play.lit(Lamp::Play) && no_play.lit(Lamp::Cue));
        let no_cue = frame([0x00, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0xeb]);
        assert!(!no_cue.lit(Lamp::Cue) && no_cue.lit(Lamp::Eject));
        let no_usb = frame([0x00, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0x2b]);
        assert!(!no_usb.lit(Lamp::Eject) && no_usb.lit(Lamp::Standby));
        let none = frame([0x00, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0x0b]);
        assert!(!none.lit(Lamp::Standby));
    }

    /// The meter and the cavity light's colour land where the TX builders
    /// write them.
    #[test]
    fn the_other_lamps_land_where_ep166_writes_them() {
        let mut raw = [0u8; MOSI_SIZE];
        raw[3] = 0x31;
        raw[4..7].copy_from_slice(&[0x10, 0x20, 0x30]);
        let f = MosiFrame::new(raw, Model::Cdj1500x);
        assert_eq!(f.lamp(Lamp::JogMeter), Some(LampState::Level(0x31)));
        assert_eq!(f.rgb(Lamp::Slot1), Some((0x10, 0x20, 0x30)));
        for pad in Lamp::HOT_CUES {
            assert!(!f.lit(pad), "{pad:?}");
        }
        // The state bytes and the CRC are no lamp.
        let mut raw = [0u8; MOSI_SIZE];
        raw[10..14].copy_from_slice(&[0x53, 0x55, 0xff, 0xff]);
        let f = MosiFrame::new(raw, Model::Cdj1500x);
        for &lamp in Lamp::ALL {
            assert!(!f.lit(lamp), "{lamp:?}");
        }
    }
}
