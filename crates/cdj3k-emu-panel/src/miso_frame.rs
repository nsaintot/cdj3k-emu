//! The control frame the host sends the deck.
//!
//! The UI says what the panel is doing in a [`PanelState`], in no deck's
//! coordinates, and the player's [`MisoCodec`] (its `ModelSpec::miso`) writes
//! the frame its sub-CPU would send, every quirk of that deck included. Each
//! model keeps its encoder in its own spec file. Buttons travel as frame bits,
//! resolved once through [`MisoCodec::button`], because a script can also
//! name a raw bit.
//!
//! [`fields`] and [`ROTARY_IDLE`] are the CDJ-3000's frame, which the
//! CDJ-3000X's encoder writes again behind its version word.

use crate::button::Btn;
use crate::crc::crc16_x25;
use crate::frame::FrameBit;
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

/// Where the analog fields sit in the CDJ-3000's frame.
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
    /// Its speed in turns per second, forward positive: how fast it actually
    /// turns, the hand's motion while held.
    pub rps: f32,
    /// Whether a hand is on the platter.
    pub touched: bool,
}

/// What the panel is doing, in no deck's coordinates.
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
    /// A second encoder's counter, where the player has one (e.g. BEAT LOOP on
    /// the CDJ-1500X); ignored by a player that does not.
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
    /// Nothing pressed, every control at rest, powered.
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

/// Where `btn` sits in `model`'s frame, or `None` if that player has no such
/// button.
pub fn button(model: Model, btn: Btn) -> Option<FrameBit> {
    model.spec().miso.button(btn)
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

/// `model`'s frame for `state`.
pub fn encode(model: Model, state: &PanelState) -> [u8; MISO_SIZE] {
    model.spec().miso.encode(state)
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

// ── Pieces the encoders share ─────────────────────────────────────────────────

/// A frame starting from `idle`, the bytes before the CRC.
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

/// The CRC-16/X-25 of the bytes before `at`, stored at `at`.
pub(crate) fn stamp_crc(f: &mut [u8; MISO_SIZE], at: usize, big_endian: bool) {
    let crc = crc16_x25(&f[..at]);
    if big_endian {
        put_be(f, at, crc);
    } else {
        put_le(f, at, crc);
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

    /// The states the golden frames below were taken in.
    fn scenarios(model: Model) -> Vec<(&'static str, PanelState)> {
        let rest = PanelState::at_rest;
        let play = button(model, Btn::Play);
        let hot_c = button(model, Btn::HotC);
        vec![
            ("idle", rest()),
            (
                "buttons",
                PanelState {
                    pressed: play.into_iter().chain(hot_c).chain([(3, 0x10)]).collect(),
                    cleared: vec![(18, 0x40)],
                    ..rest()
                },
            ),
            (
                "analog",
                PanelState {
                    tempo: 0.73,
                    jog: JogState {
                        pos: 0x1234,
                        vel: 0x0456,
                        touch: 0x0c,
                        revs: 3.25,
                        rps: 2.5,
                        touched: false,
                    },
                    rotary: 0x7ff1,
                    beat_loop: 0x2345,
                    direction: Direction::Reverse,
                    vinyl: 200,
                    touch: Some((0x8123, 0x0456)),
                    ..rest()
                },
            ),
            (
                "tempo0",
                PanelState {
                    tempo: 0.0,
                    jog: JogState {
                        touch: 0x04,
                        ..rest().jog
                    },
                    ..rest()
                },
            ),
            (
                "tempo1",
                PanelState {
                    tempo: 1.0,
                    jog: JogState {
                        touch: 0x04,
                        ..rest().jog
                    },
                    direction: Direction::SlipReverse,
                    vinyl: 7,
                    ..rest()
                },
            ),
            (
                "poweroff",
                PanelState {
                    pressed: play.into_iter().collect(),
                    tempo: 0.25,
                    jog: JogState {
                        pos: 9,
                        vel: 8,
                        touch: 0x03,
                        revs: -0.5,
                        rps: -1.0,
                        touched: true,
                    },
                    rotary: 3,
                    vinyl: 1,
                    touch: Some((5, 6)),
                    power: false,
                    ..rest()
                },
            ),
        ]
    }

    /// Whole frames for the [`scenarios`], each player's encoder end to end.
    const GOLDEN: &[(&str, &str, &str)] = &[
        ("cdj3k", "idle", "0000010403000000000000008100ffff00000000007f507f00000000ffff00005101d5d6d6d5d5d6d6d5d5d6000000000000000000000000000000000000230b"),
        ("cdj3k", "buttons", "0000011403010000000400008100ffff00000000007f507f00000000ffff00005101d5d6d6d5d5d6d6d5d5d6000000000000000000000000000000000000107d"),
        ("cdj3k", "analog", "0000010401000000000000008100f17f23815604007f82bac800341256040c005101d5d6d6d5d5d6d6d5d5d6000000000000000000000000000000000000fe29"),
        ("cdj3k", "tempo0", "0000010403000000000000008100ffff00000000007f000000000000ffff04005101d5d6d6d5d5d6d6d5d5d60000000000000000000000000000000000002fa1"),
        ("cdj3k", "tempo1", "0000010402000000000000008100ffff00000000007fffff07000000ffff04005101d5d6d6d5d5d6d6d5d5d60000000000000000000000000000000000008f83"),
        ("cdj3k", "poweroff", "0000010403010000000000000100030005000600007fa83f01000900080003005101d5d6d6d5d5d6d6d5d5d6000000000000000000000000000000000000b5f7"),
        ("cdj3kx", "idle", "010000000000010403000000800000008000ffff00000000007f507f00000000ffff00005101d5d6d6d5d5d6d6d5d5d60000000000000000000000000000e88f"),
        ("cdj3kx", "buttons", "010000100000010403010000800400008000bfff00000000007f507f00000000ffff00005101d5d6d6d5d5d6d6d5d5d600000000000000000000000000005790"),
        ("cdj3kx", "analog", "010000000000010401000000800000008000f17f23815604007f82bac800341256040c005101d5d6d6d5d5d6d6d5d5d6000000000000000000000000000033b6"),
        ("cdj3kx", "tempo0", "010000000000010403000000800000008000ffff00000000007f000000000000ffff04005101d5d6d6d5d5d6d6d5d5d60000000000000000000000000000061a"),
        ("cdj3kx", "tempo1", "010000000000010402000000800000008000ffff00000000007fffff07000000ffff04005101d5d6d6d5d5d6d6d5d5d60000000000000000000000000000dc5a"),
        ("cdj3kx", "poweroff", "010000000000010403010000000000000000030005000600007fa83f01000900080003005101d5d6d6d5d5d6d6d5d5d60000000000000000000000000000f829"),
        ("cdj1500x", "idle", "010000001ff81ff8000000000000ffff00000000ffff000001d80000000080000000000000000000000000000000000000000000000000000000000000000000"),
        ("cdj1500x", "buttons", "010000101ff81ff8000000000000ffff40002000ffff00008fba0000000080000000000000000000000000000000000000000000000000000000000000000000"),
        ("cdj1500x", "analog", "010000002ead1ff8234500000924022c000000027ff100007b6e2381560480000000000000000000000000000000000000000000000000000000000000000000"),
        ("cdj1500x", "tempo0", "0100000000001ff8000000000000ffff00000000ffff0000646a0000000080000000000000000000000000000000000000000000000000000000000000000000"),
        ("cdj1500x", "tempo1", "010000003ff01ff8000000000000ffff00000000ffff000088bf0000000080000000000000000000000000000000000000000000000000000000000000000000"),
        ("cdj1500x", "poweroff", "010000000ffc1ff800000000fe98056d4000000100030000ca3a0500060000000000000000000000000000000000000000000000000000000000000000000000"),
    ];

    #[test]
    fn every_player_encodes_its_golden_frames() {
        for model in Model::ALL {
            for (name, state) in scenarios(model) {
                let (_, _, want) = GOLDEN
                    .iter()
                    .find(|(slug, n, _)| *slug == model.spec().slug && *n == name)
                    .unwrap_or_else(|| panic!("no golden for {model} {name}"));
                let got: String = encode(model, &state)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect();
                assert_eq!(&got, want, "{model} {name}");
            }
        }
    }

    /// A button resolves in the player's own frame.
    #[test]
    fn buttons_resolve_in_the_players_own_frame() {
        let (byte, mask) = Btn::Play.bit().unwrap();
        assert_eq!(button(Model::Cdj3k, Btn::Play), Some((byte, mask)));
        assert_eq!(
            button(Model::Cdj3kx, Btn::Play),
            Some((byte + cdj3kx::MISO_SHIFT, mask))
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
        let (byte, mask) = Btn::Usb2Stop.bit().unwrap();
        assert_eq!(
            button(Model::Cdj3kx, Btn::Usb2Stop),
            Some((byte + cdj3kx::MISO_SHIFT, mask))
        );
        assert_eq!(button(Model::Cdj3k, Btn::Usb2Stop), None);
    }

    /// The guest forwarder powers the deck off when the flag it reads drops:
    /// byte 12 on the RK3399 decks, where the CDJ-3000X also clears its own
    /// shifted copy; byte 30 on the CDJ-1500X, whose byte 12 is panel data.
    #[test]
    fn power_off_reaches_every_bit_the_guest_reads() {
        let off = PanelState {
            power: false,
            ..PanelState::at_rest()
        };
        let on = PanelState::at_rest();

        let f = encode(Model::Cdj3kx, &off);
        assert_eq!(f[12] & 0x80, 0);
        assert_eq!(f[16] & 0x80, 0);
        let f = encode(Model::Cdj3kx, &on);
        assert_eq!(f[12] & 0x80, 0x80);
        assert_eq!(f[16] & 0x80, 0x80);

        assert_eq!(encode(Model::Cdj3k, &off)[12] & 0x80, 0);
        assert_eq!(encode(Model::Cdj3k, &on)[12] & 0x80, 0x80);

        assert_eq!(encode(Model::Cdj1500x, &on)[30] & 0x80, 0x80);
        let f = encode(Model::Cdj1500x, &off);
        assert_eq!(f[30] & 0x80, 0);
        assert_eq!(f[12], 0);
    }

    /// Every RK3399 player answers to every shared button.
    #[test]
    fn the_rk3399_players_resolve_every_shared_button() {
        for model in [Model::Cdj3k, Model::Cdj3kx] {
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

    /// The CRC lands where each player's sub-CPU checks it.
    #[test]
    fn the_crc_lands_where_each_player_checks_it() {
        let rest = PanelState::at_rest();
        let f = encode(Model::Cdj3k, &rest);
        assert_eq!(&f[62..64], &crc16_x25(&f[..62]).to_le_bytes());
        let f = encode(Model::Cdj1500x, &rest);
        assert_eq!(&f[24..26], &crc16_x25(&f[..24]).to_be_bytes());
    }
}
