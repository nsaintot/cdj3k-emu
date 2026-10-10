//! The direction switch, a 3-position rocker on the deck. Byte 4 of the MISO
//! frame holds its position as a 2-bit value:
//!
//! | value | position |
//! |---|---|
//! | `1` | REV (latched) |
//! | `2` | SLIP REV (momentary; it springs back to FWD on release) |
//! | `3` | FWD (the default) |

/// One of the three positions the direction rocker can encode.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum Direction {
    Reverse = 1,
    SlipReverse = 2,
    #[default]
    Forward = 3,
}

impl Direction {
    /// The byte the MISO frame's direction field holds.
    #[inline]
    pub fn as_byte(self) -> u8 {
        self as u8
    }
}
