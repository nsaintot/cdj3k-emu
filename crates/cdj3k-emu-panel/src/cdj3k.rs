//! CDJ-3000.
//!
//! The shared frame coordinates are this player's, so both its shifts are 0
//! and its RGB lamps sit at the bases the `mosi_frame` constants name.

use crate::frame::{JogBrightness, MisoMap, MosiMap, RgbLamps};
use crate::miso_frame::{fields, ROTARY_IDLE};
use crate::mosi_frame as led;
use crate::mosi_frame::{LedProfile, LedProfiles, AZURE_BLUE_DIES};
use crate::spec::{ModelSpec, TouchSpace, TOUCH_FRAME_RANGE};

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
    firmware_file_names: &["CDJ3K"],
    miso: MisoMap {
        shift: 0,
        idle: &IDLE,
        extra: &[],
        power_mirrors: &[],
    },
    mosi: MosiMap {
        bit_shift: 0,
        lamps: RgbLamps {
            slot_1: Some(36),
            slot_2: Some(39),
            on_air: Some(42),
            // PLAY and CUE are bitfield bits here, not colours.
            play: None,
            cue: None,
            // The selector ring is unlit.
            rotary: None,
            hot_cue: [12, 15, 18, 21, 24, 27, 30, 33],
        },
        jog: JogBrightness::Bits {
            byte: led::LED_JOG_WHITE.0,
            dim: 0x04,
            bright: 0x08,
        },
    },
    // One LED part throughout. Its white `44 78 7f` cuts back the red die,
    // the strongest.
    leds: LedProfiles::uniform(LedProfile {
        dies: AZURE_BLUE_DIES,
        gamma: 1.0,
        white: [0x44, 0x78, 0x7f],
        balance_whites_only: true,
    }),
};
