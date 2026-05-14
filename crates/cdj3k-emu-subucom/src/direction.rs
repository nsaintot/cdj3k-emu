//! Direction switch — the 3-position rocker on the deck (byte 4 of the
//! MISO frame is a 2-bit encoded value, **not** a bitmask).
//!
//! Wire encoding (byte 4):
//!   `1` → REV       (latched)
//!   `2` → SLIP REV  (momentary; springs back to FWD on release)
//!   `3` → FWD       (default)
//!
//! Modeled here as an enum to stop the rest of the codebase from passing
//! around bare `i8`s and comparing to magic literals.

/// One of the three positions the direction rocker can encode.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Direction {
    Reverse = 1,
    SlipReverse = 2,
    Forward = 3,
}

impl Direction {
    /// Wire-encoded byte for the MISO frame's direction slot.
    #[inline]
    pub fn as_byte(self) -> u8 {
        self as u8
    }
}

impl Default for Direction {
    fn default() -> Self {
        Direction::Forward
    }
}
