//! Record types for a model's sub-CPU frame map.
//!
//! The sub-CPU speaks two 64-byte frames: MISO carries the panel's controls to
//! the deck, MOSI carries its lamps back. Every model puts the same fields in
//! them, in a different arrangement, so the codec in
//! [`miso_frame`](crate::miso_frame) / [`mosi_frame`](crate::mosi_frame) is
//! written once against the records here and a model is a table of numbers -
//! see [`cdj3k`](crate::cdj3k) and [`cdj3kx`](crate::cdj3kx).
//!
//! The CDJ-3000's frame is the origin of the shared coordinates: its bitfield
//! byte numbers and MISO field offsets are the ones the `LED_*` and `BTN_*`
//! constants carry, and a model's [`MosiMap::bit_shift`] / [`MisoMap::shift`]
//! move them into its own frame. RGB lamps do not follow that rule - a model
//! reassigns them - so [`RgbLamps`] names each one at its own offset.

use crate::button::Btn;

/// Single-bit field: `(byte_offset, bitmask)`.
pub type FrameBit = (usize, u8);

/// Two-step LED: byte offset + `medium`/`full` bitmasks.
///
/// Setting only `medium` means "dimly lit"; setting both means "fully lit".
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct StepLedMask {
    pub byte: usize,
    pub medium: u8,
    pub full: u8,
}

impl StepLedMask {
    pub const fn new(byte: usize, medium: u8, full: u8) -> Self {
        Self { byte, medium, full }
    }
}

/// How a model encodes the jog ring's brightness.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum JogBrightness {
    /// Two bits inside the ring's colour byte: `dim` alone is half
    /// brightness, `dim | bright` is full. `byte` is a shared bitfield byte,
    /// so [`MosiMap::bit_shift`] moves it with the rest of them.
    Bits { byte: usize, dim: u8, bright: u8 },
    /// A byte of its own, outside the shared block, holding the level as a
    /// plain value: 0 off, 1 dim, 2 full. `byte` is an offset in the player's
    /// own frame. Masking it with [`Self::Bits`] reads zero for 1 and 2.
    Level { byte: usize },
}

/// The RGB lamps of one model, each at its own MOSI offset. A model that has
/// no such lamp leaves it `None`.
///
/// Slot 1 and slot 2 are the media slots in panel order: SD then USB on the
/// CDJ-3000, USB 1 then USB 2 on the CDJ-3000X.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RgbLamps {
    pub slot_1: Option<usize>,
    pub slot_2: Option<usize>,
    pub on_air: Option<usize>,
    /// PLAY, on a model that drives it as a colour rather than the
    /// `LED_PLAY` bit.
    pub play: Option<usize>,
    /// CUE, under the [`Self::play`] rule with `LED_CUE` as the bit.
    pub cue: Option<usize>,
    /// The rotary selector ring.
    pub rotary: Option<usize>,
    /// HOT CUE pads A-H.
    pub hot_cue: [usize; 8],
}

/// Where a model keeps the lamps of the MOSI frame.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct MosiMap {
    /// Added to a shared bitfield byte (the byte of a `LED_*` constant) to
    /// reach this model's copy of it.
    pub bit_shift: usize,
    pub lamps: RgbLamps,
    pub jog: JogBrightness,
}

/// Where a model keeps the controls of the MISO frame.
#[derive(Copy, Clone, Debug)]
pub struct MisoMap {
    /// Added to a shared field or button byte to reach this model's copy of
    /// it. The whole CDJ-3000 frame moves together.
    pub shift: usize,
    /// Bytes 0..62 of the idle frame, before the CRC.
    pub idle: &'static [u8; 62],
    /// Buttons this player has beyond the CDJ-3000's set; every other
    /// [`Btn::EXTRA`](crate::button::Btn::EXTRA) resolves to nothing on it.
    pub extra: &'static [Btn],
    /// Bits in the player's own frame that the power flag drives besides its
    /// POWER ON button, for a guest that reads the flag somewhere else.
    pub power_mirrors: &'static [FrameBit],
}
