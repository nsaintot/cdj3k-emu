//! CDJ-3000X.
//!
//! Its MISO frame is the CDJ-3000's frame moved 4 bytes later, after a u16
//! format version and 2 pad bytes; its MOSI bitfield is moved 6 bytes later.
//! The hot cue pads and the media slots sit 12 bytes later than on the
//! CDJ-3000. PLAY and CUE, which are single bits on the CDJ-3000, are RGB
//! lamps at 18 and 21. The selector ring lights at 54, which is the CDJ-3000's
//! ON AIR offset moved 12 bytes later.

use crate::button::{Btn, POWER_ON};
use crate::cdj3k;
use crate::frame::{FrameBit, JogBrightness, MosiMap, RgbLamps};
use crate::miso_frame::{self, MisoCodec, PanelState, MISO_SIZE};
use crate::mosi_frame::{LedProfile, LedProfiles, PRIMARY_DIES, TEAL_GREEN_DIES};
use crate::spec::{Controls, JogBrake, ModelSpec, TouchSpace};

/// Offset of the CDJ-3000's MISO frame inside this one.
pub const MISO_SHIFT: usize = 4;

/// Offset of the CDJ-3000's MOSI bitfield inside this one.
pub const BIT_SHIFT: usize = 6;

/// The idle frame payload: format version 1 at bytes 0-1, padding at 2-3, then the
/// CDJ-3000's [`cdj3k::IDLE`](crate::cdj3k::IDLE) shifted by [`MISO_SHIFT`]
/// (`01 04` at 6-7, direction 8, buttons 9-16, rotary 18-19, touch 20-23,
/// tempo 26-27, vinyl 28, jog 30-34, cap sensor 36-47).
///
/// Byte 16 keeps only the power-on bit: the CDJ-3000's SD-cover bit is
/// [`Btn::Usb2Stop`] here. Byte 12 bit 7, which no button of this player uses,
/// stays set for the guest forwarder's power-off check, which reads the
/// CDJ-3000 offset (see [`Miso`]).
pub const IDLE: [u8; 62] = {
    let mut f = [0u8; 62];
    f[0] = 0x01;
    let mut i = 0;
    while i + MISO_SHIFT < 62 {
        f[i + MISO_SHIFT] = crate::cdj3k::IDLE[i];
        i += 1;
    }
    f[12] = 0x80;
    f[16] = 0x80;
    f
};

/// The CDJ-3000X's MISO encoder. It writes the CDJ-3000's fields and buttons
/// [`MISO_SHIFT`] bytes later, after [`IDLE`]'s version word, and the CRC
/// little-endian at byte 62. Power off clears both the shifted POWER ON bit and
/// the CDJ-3000's, which the guest forwarder reads on every RK3399 deck.
#[derive(Debug)]
pub struct Miso;

/// POWER ON in this player's own frame.
const POWER_ON_SHIFTED: FrameBit = (POWER_ON.0 + MISO_SHIFT, POWER_ON.1);

impl MisoCodec for Miso {
    fn button(&self, btn: Btn) -> Option<FrameBit> {
        if !btn.is_shared() && btn != Btn::Usb2Stop {
            return None;
        }
        let (byte, mask) = btn.bit()?;
        Some((byte + MISO_SHIFT, mask))
    }

    fn encode(&self, state: &PanelState) -> [u8; MISO_SIZE] {
        let mut f = miso_frame::from_idle(&IDLE);
        miso_frame::press(&mut f, state);
        cdj3k::write_fields(&mut f, state, MISO_SHIFT);
        miso_frame::clear(&mut f, state);
        if !state.power {
            miso_frame::set_bit(&mut f, POWER_ON_SHIFTED, false);
            miso_frame::set_bit(&mut f, POWER_ON, false);
        }
        miso_frame::stamp_crc(&mut f, 62, false);
        f
    }
}

/// The CDJ-3000X's lamps, read by the CDJ-3000 family's codec.
pub const MOSI: MosiMap = MosiMap {
    bit_shift: BIT_SHIFT,
    lamps: RgbLamps {
        slot_1: Some(48),
        slot_2: Some(51),
        // The CDJ-3000X has no ON AIR indicator; the selector ring takes the
        // offset it would map to.
        on_air: None,
        play: Some(18),
        cue: Some(21),
        // The ring is on the same 3-byte grid as the media slots: with both
        // slots idle, the frame holds `48:0a 08 07 | 51:0a 08 07 | 54:4a 57 39`.
        rotary: Some(54),
        hot_cue: [24, 27, 30, 33, 36, 39, 42, 45],
    },
    // The colour stays on the shifted bitfield byte; only the level moves
    // out to a byte of its own.
    jog: JogBrightness::Level { byte: 2 },
};

pub const SPEC: ModelSpec = ModelSpec {
    title: "CDJ-3000X",
    slug: "cdj3kx",
    aliases: &["3000x", "3kx"],
    // The panel is 800x1280 in portrait. The deck's apl_start.sh rotates it
    // with xrandr, and the emulator scans it out in landscape.
    main_lcd: (1280, 800),
    touch: TouchSpace::ScreenPixels,
    model_env: "CDJ3000X",
    emmc_index: 1,
    system_rev_env: Some("rev_system"),
    firmware_file_names: &["CDJ3000X"],
    miso: &Miso,
    mosi: &MOSI,
    // Each LED part has its own gamma and white point, fitted so that each
    // colour setting matches the hue the CDJ-3000X's screen shows.
    leds: LedProfiles {
        pad: CDJ3000X_SLOT,
        slot: CDJ3000X_SLOT,
        ring: LedProfile {
            dies: PRIMARY_DIES,
            gamma: 0.523,
            white: [0x4a, 0x57, 0x39],
            balance_whites_only: false,
        },
        on_air: CDJ3000X_SLOT,
        play_rim: LedProfile {
            dies: TEAL_GREEN_DIES,
            gamma: 0.805,
            white: [0xb9, 0x7f, 0x49],
            balance_whites_only: false,
        },
        cue_rim: CDJ3000X_SLOT,
    },
    jog_brake: JogBrake::Adjustable,
    controls: Controls {
        beat_loop_encoder: false,
        direction_lever: true,
        vinyl_speed_knob: true,
    },
};

/// The media slots' LEDs. The pads and the CUE rim use the same profile,
/// which is not measured for them.
const CDJ3000X_SLOT: LedProfile = LedProfile {
    dies: PRIMARY_DIES,
    gamma: 0.805,
    white: [0xcd, 0xa5, 0x8c],
    balance_whites_only: false,
};
