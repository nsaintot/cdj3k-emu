//! The control frame the host sends the deck.
//!
//! The UI says what the panel is doing in a [`PanelState`], in no deck's
//! coordinates, and the player's [`MisoCodec`] (its `ModelSpec::miso`) writes
//! the frame its sub-CPU would send, every quirk of that deck included. Each
//! model keeps its encoder in its own spec file. Buttons travel as frame bits,
//! resolved once through [`MisoCodec::button`], because a script can also
//! name a raw bit.
//!
//! [`fields`] and [`ROTARY_IDLE`] describe the CDJ-3000's frame. The
//! CDJ-3000X's encoder writes the same fields after its version word.

use crate::button::Btn;
use crate::crc::crc16_x25;
use crate::frame::FrameBit;
use crate::Direction;

pub const MISO_SIZE: usize = 64;

/// The rotary encoder's counter at rest, a little-endian word at bytes 14-15.
///
/// The deck differentiates this counter, so the host has to send exactly what
/// the guest's sub-CPU module serves while nothing turns; the deck reads any
/// difference as a detent. The value is arbitrary, but `subucom_virt.c`, the
/// idle payloads and the app's neutral value must agree, and the module is
/// compiled into the initramfs so that the guest uses this copy.
pub const ROTARY_IDLE: u16 = 0xffff;

/// Where the analog fields sit in the CDJ-3000's frame.
pub mod fields {
    /// 3-position rocker switch (REV, SLIP_REV or FWD).
    pub const DIRECTION: usize = 4;
    /// Rotary encoder counter, 16-bit LE.
    pub const ROTARY: usize = 14;
    /// LCD touch, X then Y, 16-bit LE each. `(0, 0)` means no touch; a
    /// model that takes touch in screen pixels flags a contact with
    /// [`TOUCH_DOWN`](crate::TOUCH_DOWN) instead.
    pub const TOUCH: usize = 16;
    /// Tempo slider, 16-bit LE over the full range, with a dead zone of about
    /// 0x7f50-0x7fd0.
    pub const TEMPO: usize = 22;
    /// Vinyl speed rotary, 0x00-0xff.
    pub const VINYL: usize = 24;
    /// Jog wheel: position (16-bit LE), velocity (16-bit LE and inverted:
    /// 0xffff is stopped, 0x0000 is full speed), then the touch state byte
    /// (0x00 none, 0x03 press, 0x04 idle/baseline, 0x0c turning).
    pub const JOG: usize = 26;
}

/// The jog wheel as the panel's physics count it: position, velocity and
/// touch state, in the CDJ-3000 sub-CPU's units (see [`fields::JOG`]).
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct JogState {
    /// The CDJ-3000's position counter, velocity word and touch byte, as the
    /// UI's platter model encodes them.
    pub pos: u16,
    pub vel: u16,
    pub touch: u8,
    /// The platter's travel in turns since start, forward positive.
    pub revs: f64,
    /// The platter's speed in turns per second, forward positive. While a
    /// hand holds the platter, this is the hand's motion.
    pub rps: f32,
    /// Whether a hand is on the platter.
    pub touched: bool,
}

/// What the panel is doing. The button bits are already in the player's
/// frame; the player's encoder places the analog fields.
#[derive(Clone, Debug)]
pub struct PanelState {
    /// Frame bits held down, already resolved in the player's frame.
    pub pressed: Vec<FrameBit>,
    /// Frame bits forced low after everything else is written.
    pub cleared: Vec<FrameBit>,
    /// Tempo fader travel, `0.0..=1.0`, the centre detent at `0.5`.
    pub tempo: f32,
    pub jog: JogState,
    /// The browse rotary's counter (see [`ROTARY_IDLE`]).
    pub rotary: u16,
    /// A second encoder's counter, such as BEAT LOOP on the CDJ-1500X. A
    /// player without a second encoder ignores it.
    pub beat_loop: u16,
    pub direction: Direction,
    /// VINYL SPEED ADJUST, `0x00..=0xff`.
    pub vinyl: u8,
    /// LCD touch in the model's touch units, `None` for no contact.
    pub touch: Option<(u16, u16)>,
    /// The emulator's power flag. The idle frame carries it set; `false`
    /// clears it after everything else, which the guest reads as power off.
    pub power: bool,
}

impl PanelState {
    /// A state with nothing pressed, every control at rest and the power on.
    pub fn at_rest() -> Self {
        Self {
            pressed: Vec::new(),
            cleared: Vec::new(),
            tempo: 0.5,
            jog: JogState {
                pos: 0,
                vel: 0xffff,
                touch: 0x00,
                revs: 0.0,
                rps: 0.0,
                touched: false,
            },
            rotary: ROTARY_IDLE,
            beat_loop: 0,
            direction: Direction::Forward,
            vinyl: 0,
            touch: None,
            power: true,
        }
    }
}

/// How one player's sub-CPU frames the panel.
pub trait MisoCodec: Sync + std::fmt::Debug {
    /// Where `btn` sits in this player's frame, or `None` if it has no such
    /// button.
    fn button(&self, btn: Btn) -> Option<FrameBit>;

    /// The frame for `state`, CRC included.
    fn encode(&self, state: &PanelState) -> [u8; MISO_SIZE];
}

/// A raw frame bit written as `<byte>:<mask>` (`"12:0x01"`, decimal or
/// `0x`-prefixed), or `None` if `name` is not one or falls off the frame.
pub fn raw_bit(name: &str) -> Option<FrameBit> {
    let (byte, mask) = name.split_once(':')?;
    let parse = |s: &str| -> Option<u64> {
        match s.strip_prefix("0x") {
            Some(h) => u64::from_str_radix(h, 16).ok(),
            None => s.parse().ok(),
        }
    };
    let byte = parse(byte)? as usize;
    let mask = u8::try_from(parse(mask)?).ok()?;
    (byte < MISO_SIZE - 2).then_some((byte, mask))
}

// ── Pieces the encoders share ─────────────────────────────────────────────────

/// A frame whose bytes before the CRC are copied from `idle`.
pub(crate) fn from_idle(idle: &[u8; 62]) -> [u8; MISO_SIZE] {
    let mut f = [0u8; MISO_SIZE];
    f[..62].copy_from_slice(idle);
    f
}

pub(crate) fn set_bit(f: &mut [u8; MISO_SIZE], (byte, mask): FrameBit, on: bool) {
    if on {
        f[byte] |= mask;
    } else {
        f[byte] &= !mask;
    }
}

/// Set every bit `state` holds down.
pub(crate) fn press(f: &mut [u8; MISO_SIZE], state: &PanelState) {
    for &bit in &state.pressed {
        set_bit(f, bit, true);
    }
}

/// Force low every bit `state` clears.
pub(crate) fn clear(f: &mut [u8; MISO_SIZE], state: &PanelState) {
    for &bit in &state.cleared {
        set_bit(f, bit, false);
    }
}

/// Write `v` at `at`, 16-bit little-endian.
pub(crate) fn put_le(f: &mut [u8; MISO_SIZE], at: usize, v: u16) {
    f[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

/// Write `v` at `at`, 16-bit big-endian.
pub(crate) fn put_be(f: &mut [u8; MISO_SIZE], at: usize, v: u16) {
    f[at..at + 2].copy_from_slice(&v.to_be_bytes());
}

/// Stores the CRC-16/X-25 of the bytes before `at` at offset `at`.
pub(crate) fn stamp_crc(f: &mut [u8; MISO_SIZE], at: usize, big_endian: bool) {
    let crc = crc16_x25(&f[..at]);
    if big_endian {
        put_be(f, at, crc);
    } else {
        put_le(f, at, crc);
    }
}
