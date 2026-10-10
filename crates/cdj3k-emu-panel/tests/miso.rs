//! Every player's MISO encoding, through [`Model`].

use cdj3k_emu_panel::miso_frame::{JogState, PanelState, MISO_SIZE, ROTARY_IDLE};
use cdj3k_emu_panel::{cdj3k, cdj3kx, crc16_x25, Btn, Direction, Model};

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
    let play = model.button(Btn::Play);
    let hot_c = model.button(Btn::HotC);
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
            let got: String = model
                .encode(&state)
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
    assert_eq!(Model::Cdj3k.button(Btn::Play), Some((byte, mask)));
    assert_eq!(
        Model::Cdj3kx.button(Btn::Play),
        Some((byte + cdj3kx::MISO_SHIFT, mask))
    );
    // Case and the raw form are accepted the same way on either player.
    assert_eq!(
        Model::Cdj3kx.button_by_name("play"),
        Model::Cdj3kx.button(Btn::Play)
    );
    assert_eq!(Model::Cdj3kx.button_by_name("12:0x01"), Some((12, 0x01)));
    assert_eq!(Model::Cdj3k.button_by_name("64:0x01"), None);
    assert_eq!(Model::Cdj3k.button_by_name("NOT_A_BUTTON"), None);
}

/// The CDJ-3000X uses the CDJ-3000's SD cover state bit, at the shifted
/// offset, for USB 2 STOP, a button the CDJ-3000 does not have.
#[test]
fn a_player_can_add_a_button() {
    let (byte, mask) = Btn::Usb2Stop.bit().unwrap();
    assert_eq!(
        Model::Cdj3kx.button(Btn::Usb2Stop),
        Some((byte + cdj3kx::MISO_SHIFT, mask))
    );
    assert_eq!(Model::Cdj3k.button(Btn::Usb2Stop), None);
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

    let f = Model::Cdj3kx.encode(&off);
    assert_eq!(f[12] & 0x80, 0);
    assert_eq!(f[16] & 0x80, 0);
    let f = Model::Cdj3kx.encode(&on);
    assert_eq!(f[12] & 0x80, 0x80);
    assert_eq!(f[16] & 0x80, 0x80);

    assert_eq!(Model::Cdj3k.encode(&off)[12] & 0x80, 0);
    assert_eq!(Model::Cdj3k.encode(&on)[12] & 0x80, 0x80);

    assert_eq!(Model::Cdj1500x.encode(&on)[30] & 0x80, 0x80);
    let f = Model::Cdj1500x.encode(&off);
    assert_eq!(f[30] & 0x80, 0);
    assert_eq!(f[12], 0);
}

/// Every RK3399 player answers to every shared button.
#[test]
fn the_rk3399_players_resolve_every_shared_button() {
    for model in [Model::Cdj3k, Model::Cdj3kx] {
        for &btn in Btn::SHARED {
            let bit = model
                .button(btn)
                .unwrap_or_else(|| panic!("{model} has no {}", btn.name()));
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
    let f = Model::Cdj3k.encode(&rest);
    assert_eq!(&f[62..64], &crc16_x25(&f[..62]).to_le_bytes());
    let f = Model::Cdj1500x.encode(&rest);
    assert_eq!(&f[24..26], &crc16_x25(&f[..24]).to_be_bytes());
}
