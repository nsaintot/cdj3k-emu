//! The control frame the host sends the deck, and the CDJ-3000 coordinates
//! every model's [`MisoMap`] is expressed relative to.
//!
//! A player's whole frame moves together, so the one [`Btn`] set shifted by
//! [`MisoMap::shift`] serves all of them; a player that gained a control
//! claims it in [`MisoMap::extra`].

use crate::button::Btn;
use crate::crc::crc16_x25;
use crate::frame::{FrameBit, MisoMap};
use crate::model::Model;
use crate::Direction;

pub const MISO_SIZE: usize = 64;

/// Rotary encoder counter at rest (b14-b15, LE).
///
/// The deck differentiates this counter, so the host has to send exactly what
/// the guest's sub-CPU module serves while nothing turns - any mismatch is a
/// step the deck reads as a detent. The value itself is arbitrary; what matters
/// is that `subucom_virt.c`, the idle payloads and the app's neutral agree, and
/// the module is compiled into the initramfs so it is the one they follow.
pub const ROTARY_IDLE: u16 = 0xffff;

/// Where the analog fields sit in the shared frame.
pub mod fields {
    /// 3-position rocker switch (REV / SLIP_REV / FWD).
    pub const DIRECTION: usize = 4;
    /// Rotary encoder counter, 16-bit LE.
    pub const ROTARY: usize = 14;
    /// LCD touch, X then Y, 16-bit LE each. `(0, 0)` means no touch; a
    /// model that takes touch in screen pixels flags a contact with
    /// [`TOUCH_DOWN`](crate::TOUCH_DOWN) instead.
    pub const TOUCH: usize = 16;
    /// Tempo slider, 16-bit LE over the full range, dead zone ~0x7f50-0x7fd0.
    pub const TEMPO: usize = 22;
    /// Vinyl speed rotary, 0x00-0xff.
    pub const VINYL: usize = 24;
    /// Jog wheel: position (16-bit LE), velocity (16-bit LE, INVERSE -
    /// 0xffff stopped, 0x0000 full speed), then the touch state byte
    /// (0x00 none, 0x03 press, 0x04 idle/baseline, 0x0c turning).
    pub const JOG: usize = 26;
}

/// Where `btn` sits in `model`'s frame, or `None` if that player has no such
/// button.
pub fn button(model: Model, btn: Btn) -> Option<FrameBit> {
    let map = &model.spec().miso;
    if !btn.is_shared() && !map.extra.contains(&btn) {
        return None;
    }
    let (byte, mask) = btn.bit();
    Some((byte + map.shift, mask))
}

/// [`button`] from a written name: a [`Btn::name`] (case-insensitive) or a raw
/// frame bit as `<byte>:<mask>` (`"12:0x01"`, decimal or `0x`-prefixed). A raw
/// bit is an address in the player's own frame and is taken as written.
pub fn button_by_name(model: Model, name: &str) -> Option<FrameBit> {
    match raw_bit(name) {
        Some(raw) => Some(raw),
        None => button(model, Btn::from_name(name)?),
    }
}

fn raw_bit(name: &str) -> Option<FrameBit> {
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

/// A 64-byte MISO control frame, written through one player's [`MisoMap`].
pub struct MisoFrame {
    bytes: [u8; MISO_SIZE],
    map: &'static MisoMap,
}

impl MisoFrame {
    /// `model`'s idle frame: nothing pressed, every analog control at rest.
    pub fn idle(model: Model) -> Self {
        let map = &model.spec().miso;
        let mut bytes = [0u8; MISO_SIZE];
        bytes[..62].copy_from_slice(map.idle);
        Self { bytes, map }
    }

    /// The frame offset of shared field byte `byte`.
    fn at(&self, byte: usize) -> usize {
        byte + self.map.shift
    }

    /// Press or release a bit already resolved against this player's frame -
    /// from [`button`], so the shift is applied once, at lookup.
    pub fn set_btn(&mut self, btn: FrameBit, pressed: bool) {
        let (byte, mask) = btn;
        if pressed {
            self.bytes[byte] |= mask;
        } else {
            self.bytes[byte] &= !mask;
        }
    }

    /// Set the touch words ([`fields::TOUCH`]); `(0, 0)` means no touch.
    pub fn set_touch(&mut self, x: u16, y: u16) {
        let o = self.at(fields::TOUCH);
        self.bytes[o..o + 2].copy_from_slice(&x.to_le_bytes());
        self.bytes[o + 2..o + 4].copy_from_slice(&y.to_le_bytes());
    }

    /// Rotary encoder counter.
    pub fn set_rotary(&mut self, counter: u16) {
        let o = self.at(fields::ROTARY);
        self.bytes[o..o + 2].copy_from_slice(&counter.to_le_bytes());
    }

    /// Jog wheel position, velocity and touch state - see [`fields::JOG`].
    pub fn set_jog(&mut self, pos: u16, vel: u16, touch: u8) {
        let o = self.at(fields::JOG);
        self.bytes[o..o + 2].copy_from_slice(&pos.to_le_bytes());
        self.bytes[o + 2..o + 4].copy_from_slice(&vel.to_le_bytes());
        self.bytes[o + 4] = touch;
    }

    pub fn set_direction(&mut self, direction: Direction) {
        let o = self.at(fields::DIRECTION);
        self.bytes[o] = direction.as_byte();
    }

    /// Tempo slider - see [`fields::TEMPO`].
    pub fn set_tempo(&mut self, raw: u16) {
        let o = self.at(fields::TEMPO);
        self.bytes[o..o + 2].copy_from_slice(&raw.to_le_bytes());
    }

    /// Vinyl speed rotary.
    pub fn set_vinyl(&mut self, v: u8) {
        self.bytes[self.at(fields::VINYL)] = v;
    }

    pub fn set_power(&mut self, on: bool) {
        let (byte, mask) = Btn::PowerOn.bit();
        self.set_btn((self.at(byte), mask), on);
        for &mirror in self.map.power_mirrors {
            self.set_btn(mirror, on);
        }
    }

    pub fn finalize(mut self) -> [u8; MISO_SIZE] {
        let crc = crc16_x25(&self.bytes[..62]);
        self.bytes[62..64].copy_from_slice(&crc.to_le_bytes());
        self.bytes
    }

    pub fn as_bytes(&self) -> &[u8; MISO_SIZE] {
        &self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cdj3k, cdj3kx};

    #[test]
    fn the_cdj3000x_idle_frame_is_the_cdj3000s_at_plus_4() {
        let f = cdj3kx::IDLE;
        assert_eq!(&f[0..4], &[0x01, 0x00, 0x00, 0x00]);
        assert_eq!(&f[6..9], &[0x01, 0x04, 0x03]);
        assert_eq!(f[16], 0x80);
        assert_eq!(&f[18..20], &ROTARY_IDLE.to_le_bytes());
        assert_eq!(&f[36..48], &cdj3k::IDLE[32..44]);
    }

    #[test]
    fn analog_fields_land_shifted() {
        let mut f = MisoFrame::idle(Model::Cdj3kx);
        f.set_direction(Direction::Reverse);
        f.set_touch(0x1234, 0x5678);
        f.set_tempo(0x7f90);
        assert_eq!(f.as_bytes()[8], Direction::Reverse.as_byte());
        assert_eq!(&f.as_bytes()[20..24], &[0x34, 0x12, 0x78, 0x56]);
        assert_eq!(&f.as_bytes()[26..28], &[0x90, 0x7f]);
    }

    /// A button resolves in the player's own frame, so a slate that names a
    /// control cannot land on the wrong one whichever player it draws.
    #[test]
    fn buttons_resolve_in_the_players_own_frame() {
        assert_eq!(button(Model::Cdj3k, Btn::Play), Some(Btn::Play.bit()));
        assert_eq!(
            button(Model::Cdj3kx, Btn::Play),
            Some((Btn::Play.bit().0 + cdj3kx::MISO_SHIFT, Btn::Play.bit().1))
        );
        // Case and the raw form are accepted the same way on either player.
        assert_eq!(
            button_by_name(Model::Cdj3kx, "play"),
            button(Model::Cdj3kx, Btn::Play)
        );
        assert_eq!(button_by_name(Model::Cdj3kx, "12:0x01"), Some((12, 0x01)));
        assert_eq!(button_by_name(Model::Cdj3k, "64:0x01"), None);
        assert_eq!(button_by_name(Model::Cdj3k, "NOT_A_BUTTON"), None);
    }

    /// The CDJ-3000X turned the CDJ-3000's SD-cover state bit into USB 2 STOP: a
    /// button the CDJ-3000 does not have, at the shifted offset.
    #[test]
    fn a_player_can_add_a_button() {
        assert_eq!(
            button(Model::Cdj3kx, Btn::Usb2Stop),
            Some((
                Btn::Usb2Stop.bit().0 + cdj3kx::MISO_SHIFT,
                Btn::Usb2Stop.bit().1
            ))
        );
        assert_eq!(button(Model::Cdj3k, Btn::Usb2Stop), None);
    }

    /// The CDJ-3000X's guest forwarder checks power at the CDJ-3000 offset, which its
    /// own shift moved: the flag has to reach both.
    #[test]
    fn power_reaches_every_bit_the_guest_reads() {
        let mut f = MisoFrame::idle(Model::Cdj3kx);
        f.set_power(false);
        assert_eq!(f.as_bytes()[12] & 0x80, 0);
        assert_eq!(f.as_bytes()[16] & 0x80, 0);
        f.set_power(true);
        assert_eq!(f.as_bytes()[12] & 0x80, 0x80);
        assert_eq!(f.as_bytes()[16] & 0x80, 0x80);

        let mut f = MisoFrame::idle(Model::Cdj3k);
        f.set_power(false);
        assert_eq!(f.as_bytes()[12] & 0x80, 0);
    }

    /// Every player answers to every shared button, so a slate written
    /// against one is not silently missing controls on another.
    #[test]
    fn every_player_resolves_every_shared_button() {
        for model in Model::ALL {
            for &btn in Btn::SHARED {
                let bit =
                    button(model, btn).unwrap_or_else(|| panic!("{model} has no {}", btn.name()));
                assert!(
                    bit.0 < MISO_SIZE - 2,
                    "{model} {} runs off the frame",
                    btn.name()
                );
            }
        }
    }
}
