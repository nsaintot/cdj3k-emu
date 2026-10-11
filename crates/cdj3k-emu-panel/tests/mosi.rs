//! Every player's MOSI reading, through [`Model`].

use cdj3k_emu_panel::mosi_frame::{
    led_drive_factor, led_linear, LedProfile, MosiFrame, StepLed, LED_GAMMA, MOSI_SIZE,
};
use cdj3k_emu_panel::{cdj3k, cdj3kx, Lamp, Model};

fn frame(raw: [u8; MOSI_SIZE], model: Model) -> MosiFrame {
    model.mosi_frame(raw)
}

fn encoded(profile: &LedProfile, r: u8, g: u8, b: u8) -> [u8; 3] {
    led_linear(profile, r, g, b)
        .unwrap()
        .map(|v| (v.powf(1.0 / LED_GAMMA) * 255.0).round() as u8)
}

fn neutral([r, g, b]: [u8; 3]) -> bool {
    r.min(g).min(b) >= 0xf6
}

/// Red and green keep the plain gamma mapping; pure blue is shifted to an
/// azure, `3e 7a ff`.
#[test]
fn the_cdj3000_blue_is_desaturated_and_warm_colours_are_not() {
    let pad = &cdj3k::SPEC.leds.pad;
    assert_eq!(encoded(pad, 0xff, 0, 0), [255, 0, 0]);
    assert_eq!(encoded(pad, 0, 0xff, 0), [0, 255, 0]);
    assert_eq!(encoded(pad, 0xff, 0x2c, 0)[2], 0);
    assert_eq!(encoded(pad, 0, 0, 0xff), [0x3e, 0x7a, 0xff]);
    assert!(led_linear(pad, 0, 0, 0).is_none());
}

/// Each deck's white is neutral on its part, and fully driven.
#[test]
fn each_decks_white_is_neutral() {
    let k = &cdj3k::SPEC.leds;
    let x = &cdj3kx::SPEC.leds;
    for (profile, raw) in [
        (&k.pad, [0x44, 0x78, 0x7f]),
        (&k.pad, [0x88, 0xf0, 0xff]),
        (&x.slot, [0xcd, 0xa5, 0x8c]),
        (&x.ring, [0x4a, 0x57, 0x39]),
        (&x.play_rim, [0xb9, 0x7f, 0x49]),
    ] {
        let out = encoded(profile, raw[0], raw[1], raw[2]);
        assert!(neutral(out), "{raw:x?} -> {out:x?}");
    }
    assert_eq!(led_drive_factor(0x44, 0x78, 0x7f), Some(1.0));
    let dim = led_drive_factor(0x0a, 0x0f, 0x0f).unwrap();
    assert!((0.5..0.8).contains(&dim), "dim white drive {dim}");
}

/// The CDJ-3000X's aqua renders cyan, not green, on its slot and ring.
#[test]
fn the_cdj3000x_aqua_is_not_green() {
    let x = &cdj3kx::SPEC.leds;
    for (profile, raw) in [(&x.slot, [0x00, 0xff, 0xa7]), (&x.ring, [0x00, 0xff, 0x3f])] {
        let [_, g, b] = encoded(profile, raw[0], raw[1], raw[2]);
        assert!(b as f32 >= 0.75 * g as f32, "{raw:x?} -> g {g:x} b {b:x}");
    }
    assert_eq!(encoded(&x.slot, 0, 0xff, 0), [0, 255, 0]);
}

/// The hot cue pads, the ON AIR bar and the jog ring sit at other offsets
/// in a CDJ-3000X frame; the codec reads each where its player puts it.
#[test]
fn lamp_reads_follow_the_codec() {
    // The ring's colour moves with the bitfield, from byte 3 to byte 9. The
    // CDJ-3000X keeps its brightness as a plain level in a byte of its own;
    // the CDJ-3000 packs it into the colour byte as two bits.
    let mut raw = [0u8; MOSI_SIZE];
    raw[3] = 0x0c; // CDJ-3000 byte 3: white, full
    raw[3 + cdj3kx::BIT_SHIFT] = 0x03; // the CDJ-3000X's byte 9: red
    raw[2] = 2; // the CDJ-3000X's own brightness byte: full
    assert_eq!(frame(raw, Model::Cdj3k).step(Lamp::JogRing), StepLed::Full);
    assert!(!frame(raw, Model::Cdj3k).lit(Lamp::JogRed));
    assert_eq!(frame(raw, Model::Cdj3kx).step(Lamp::JogRing), StepLed::Full);
    assert!(frame(raw, Model::Cdj3kx).lit(Lamp::JogRed));
    // A level the CDJ-3000's bits cannot express still reads on the CDJ-3000X.
    raw[2] = 1;
    assert_eq!(
        frame(raw, Model::Cdj3kx).step(Lamp::JogRing),
        StepLed::Medium
    );

    let mut raw = [0u8; MOSI_SIZE];
    raw[cdj3k::MOSI.lamps.hot_cue[0]] = 0x11;
    raw[cdj3kx::MOSI.lamps.hot_cue[0]] = 0x22;
    assert_eq!(
        frame(raw, Model::Cdj3k).rgb(Lamp::HotA).map(|c| c.0),
        Some(0x11)
    );
    assert_eq!(
        frame(raw, Model::Cdj3kx).rgb(Lamp::HotA).map(|c| c.0),
        Some(0x22)
    );
}

/// PLAY and CUE own bytes 18-23 on the CDJ-3000X, where the CDJ-3000's
/// hot cue C and D sit; the codec keeps the transport lamps off the pads.
#[test]
fn the_cdj3000x_transport_lamps_are_not_hot_cue_pads() {
    let play = cdj3kx::MOSI.lamps.play.unwrap();
    let cue = cdj3kx::MOSI.lamps.cue.unwrap();
    let mut raw = [0u8; MOSI_SIZE];
    raw[play] = 0x31;
    raw[cue] = 0x42;
    let x = frame(raw, Model::Cdj3kx);
    assert_eq!(x.rgb(Lamp::Play).map(|rgb| rgb.0), Some(0x31));
    assert_eq!(x.rgb(Lamp::Cue).map(|rgb| rgb.0), Some(0x42));
    // On the CDJ-3000, the bases that the CDJ-3000X's transport lamps overlap
    // are pads C and D.
    assert_eq!(cdj3k::MOSI.lamps.hot_cue[2], play);
    assert_eq!(cdj3k::MOSI.lamps.hot_cue[3], cue);
    for pad in Lamp::HOT_CUES {
        assert_eq!(x.rgb(pad), Some((0, 0, 0)), "{pad:?} caught a lamp");
    }
    // The CDJ-3000 drives them as bits.
    assert_eq!(frame(raw, Model::Cdj3k).rgb(Lamp::Play), None);
    assert_eq!(frame(raw, Model::Cdj3k).rgb(Lamp::Cue), None);
}

/// The CDJ-3000X's media slots and selector ring are one 3-byte grid, and the ring
/// lands on the offset the CDJ-3000's ON AIR bar maps to, which the
/// CDJ-3000X reads as the ring.
#[test]
fn the_cdj3000x_slot_grid_ends_at_the_selector_ring() {
    let lamps = cdj3kx::MOSI.lamps;
    let slot_1 = lamps.slot_1.unwrap();
    assert_eq!(lamps.slot_2, Some(slot_1 + 3));
    assert_eq!(lamps.rotary, Some(slot_1 + 6));

    // Both slots are idle in this frame.
    let wire: [u8; 9] = [0x0a, 0x08, 0x07, 0x0a, 0x08, 0x07, 0x4a, 0x57, 0x39];
    let mut raw = [0u8; MOSI_SIZE];
    raw[slot_1..slot_1 + 9].copy_from_slice(&wire);
    let x = frame(raw, Model::Cdj3kx);
    assert_eq!(x.rgb(Lamp::Slot1), Some((0x0a, 0x08, 0x07)));
    assert_eq!(x.rgb(Lamp::Slot2), Some((0x0a, 0x08, 0x07)));
    assert_eq!(x.rgb(Lamp::Rotary), Some((0x4a, 0x57, 0x39)));
    assert_eq!(x.lamp(Lamp::OnAir), None);
    assert_eq!(cdj3k::MOSI.lamps.on_air, lamps.rotary.map(|r| r - 12));

    // The CDJ-3000 has the bar and not the ring.
    assert_eq!(frame(raw, Model::Cdj3k).lamp(Lamp::Rotary), None);
    assert!(frame(raw, Model::Cdj3k).rgb(Lamp::OnAir).is_some());
}

/// A lamp a player lacks reads dark, whatever the frame holds.
#[test]
fn a_lamp_a_player_lacks_is_never_lit() {
    let raw = [0xffu8; MOSI_SIZE];
    for model in Model::ALL {
        let f = frame(raw, model);
        for &lamp in Lamp::ALL {
            if f.lamp(lamp).is_none() {
                assert!(!f.lit(lamp), "{model} lights {lamp:?}");
                assert_eq!(f.step(lamp), StepLed::Off);
                assert_eq!(f.rgb(lamp), None);
            }
        }
    }
}

/// Each lamp of a map names a byte of its own.
#[test]
fn a_maps_lamps_do_not_overlap() {
    for (name, map) in [("CDJ-3000", cdj3k::MOSI), ("CDJ-3000X", cdj3kx::MOSI)] {
        let lamps = map.lamps;
        let mut bases: Vec<usize> = lamps.hot_cue.to_vec();
        bases.extend(
            [
                lamps.slot_1,
                lamps.slot_2,
                lamps.on_air,
                lamps.play,
                lamps.cue,
                lamps.rotary,
            ]
            .into_iter()
            .flatten(),
        );
        for (i, a) in bases.iter().enumerate() {
            assert!(a + 2 < MOSI_SIZE, "{name} lamp at {a} runs off the frame");
            for b in &bases[i + 1..] {
                assert!(a.abs_diff(*b) >= 3, "{name} lamps at {a} and {b} overlap");
            }
        }
    }
}
