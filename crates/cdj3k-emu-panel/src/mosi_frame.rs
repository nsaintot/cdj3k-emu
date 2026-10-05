//! The lamp frame the deck sends back, read through the player's
//! [`MosiCodec`].
//!
//! [`MosiFrame`] reads a frame by [`Lamp`], through the player's codec.

use crate::lamp::Lamp;
use crate::model::Model;

pub const MOSI_SIZE: usize = 64;

/// Gamma used when converting raw LED PWM bytes to sRGB display values.
///
/// The deck writes linear-light PWM values (duty cycle ∝ radiant flux).
/// Our display is sRGB (gamma ≈ 2.2).  Applying a 1/gamma power curve converts
/// linear light to perceptual brightness so the UI matches the hardware visually.
///
/// Raise toward 2.5 if colors still look washed out.
pub const LED_GAMMA: f32 = 2.2;

/// Peak output level (0–255) the dominant LED channel is normalised to.
/// 255 = fully saturated / maximum brightness; we lower it to reduce intensity.
pub const LED_PEAK: f32 = 220.0;

/// Exponent that lifts [`led_drive_factor`]: a dim LED reads brighter than its
/// duty cycle.
pub const LED_DIM_LIFT: f32 = 0.4;

/// How bright a lamp is driven, 0..1: the dies' summed duty, gamma-encoded
/// and lifted by [`LED_DIM_LIFT`]. [`led_color`] is always full brightness;
/// callers dim it by this. `None` when the LED is off.
#[inline]
pub fn led_drive_factor(r: u8, g: u8, b: u8) -> Option<f32> {
    if r | g | b == 0 {
        return None;
    }
    let duty = (r as f32 + g as f32 + b as f32) / 255.0;
    Some(duty.min(1.0).powf(LED_DIM_LIFT / LED_GAMMA))
}

/// Which LED part a lamp is built from. A model's [`LedProfiles`] gives each
/// its [`LedProfile`].
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum LedPart {
    /// Hot cue pads.
    Pad,
    /// Media slot indicators.
    Slot,
    /// The rotary selector ring.
    Ring,
    OnAir,
    PlayRim,
    CueRim,
}

/// How one LED part turns PWM into the colour it shows through the panel.
#[derive(Copy, Clone, Debug)]
pub struct LedProfile {
    /// The red, green and blue dies at full drive, in linear sRGB.
    pub dies: [[f32; 3]; 3],
    /// Light out per unit of PWM duty is `duty^gamma`.
    pub gamma: f32,
    /// The drive the deck uses for white. Per-die gains make it neutral.
    pub white: [u8; 3],
    /// Apply those gains only to drives that light all three dies, leaving
    /// saturated colours on the bare dies.
    pub balance_whites_only: bool,
}

/// A model's LED parts.
#[derive(Copy, Clone, Debug)]
pub struct LedProfiles {
    pub pad: LedProfile,
    pub slot: LedProfile,
    pub ring: LedProfile,
    pub on_air: LedProfile,
    pub play_rim: LedProfile,
    pub cue_rim: LedProfile,
}

impl LedProfiles {
    /// Every part the same.
    pub const fn uniform(p: LedProfile) -> Self {
        Self {
            pad: p,
            slot: p,
            ring: p,
            on_air: p,
            play_rim: p,
            cue_rim: p,
        }
    }

    pub fn get(&self, part: LedPart) -> &LedProfile {
        match part {
            LedPart::Pad => &self.pad,
            LedPart::Slot => &self.slot,
            LedPart::Ring => &self.ring,
            LedPart::OnAir => &self.on_air,
            LedPart::PlayRim => &self.play_rim,
            LedPart::CueRim => &self.cue_rim,
        }
    }
}

/// Dies that show as the sRGB primaries.
pub const PRIMARY_DIES: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
/// The blue die as `#3E7AFF` in linear light: an azure washed out by the
/// diffuser.
pub const AZURE_BLUE_DIES: [[f32; 3]; 3] =
    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0446, 0.1975, 1.0]];
/// A bluish green whose red lies outside sRGB, with the azure blue.
pub const TEAL_GREEN_DIES: [[f32; 3]; 3] = [
    [1.0, 0.0, 0.0],
    [-0.0846, 1.0, 0.2176],
    [0.0446, 0.1975, 1.0],
];

/// A drive whose weakest channel is at least this share of its strongest
/// takes the full white balance of a `balance_whites_only` profile; one at
/// [`WHITE_BALANCE_FROM`] or under takes none.
const WHITE_BALANCE_FROM: f32 = 0.15;
const WHITE_BALANCE_FULL: f32 = 0.45;

impl LedProfile {
    fn light(&self, duty: f32) -> f32 {
        duty.powf(self.gamma)
    }

    /// Per-die gains that make [`Self::white`] come out neutral.
    fn white_gains(&self) -> [f32; 3] {
        let d = self.dies;
        let w = self.white.map(|v| self.light(v as f32 / 255.0));
        // Solve sum_i d[i][ch] * w[i] * k[i] = 1 for every channel.
        let m = [0, 1, 2].map(|ch| [0, 1, 2].map(|i| d[i][ch] * w[i]));
        let det = |m: [[f32; 3]; 3]| {
            m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
                - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
                + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
        };
        let full = det(m);
        let k = [0, 1, 2].map(|i| {
            let mut mi = m;
            for row in mi.iter_mut() {
                row[i] = 1.0;
            }
            det(mi) / full
        });
        let top = k[0].max(k[1]).max(k[2]);
        k.map(|v| v / top)
    }
}

/// Colour a lamp shows for raw PWM `(R, G, B)`, in linear light with its
/// brightest channel at 1. `None` when the LED is off.
///
/// Each die contributes its column of [`LedProfile::dies`] in proportion to
/// the light its duty gives, scaled by the gains that make the part's white
/// neutral. A mix outside sRGB is pulled toward the grey of the same
/// luminance until no channel is negative.
pub fn led_linear(profile: &LedProfile, r: u8, g: u8, b: u8) -> Option<[f32; 3]> {
    if r | g | b == 0 {
        return None;
    }
    let pwm = [r, g, b].map(|v| v as f32 / 255.0);
    let whiteness = if profile.balance_whites_only {
        let strongest = pwm[0].max(pwm[1]).max(pwm[2]);
        let weakest = pwm[0].min(pwm[1]).min(pwm[2]);
        let t = ((weakest / strongest - WHITE_BALANCE_FROM)
            / (WHITE_BALANCE_FULL - WHITE_BALANCE_FROM))
            .clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    } else {
        1.0
    };
    let gains = profile.white_gains().map(|k| 1.0 + (k - 1.0) * whiteness);
    let mut c = [0.0f32; 3];
    for ((die, duty), gain) in profile.dies.iter().zip(pwm).zip(gains) {
        let light = profile.light(duty) * gain;
        for (ch, emitted) in c.iter_mut().zip(die) {
            *ch += emitted * light;
        }
    }
    let luma = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let min = c[0].min(c[1]).min(c[2]);
    if min < 0.0 && luma > 0.0 {
        let t = luma / (luma - min);
        c = c.map(|v| luma + t * (v - luma));
    }
    let peak = c[0].max(c[1]).max(c[2]);
    if peak <= 0.0 {
        return None;
    }
    Some(c.map(|v| (v / peak).max(0.0)))
}

/// Convert a raw LED `(R, G, B)` triple to an egui `Color32`: the lamp's
/// colour from [`led_linear`], gamma-encoded with the brightest channel at
/// [`LED_PEAK`] for a saturated colour and at 255 for white. `None` when the
/// LED is off.
#[cfg(feature = "egui-color")]
#[inline]
pub fn led_color(profile: &LedProfile, r: u8, g: u8, b: u8) -> Option<egui::Color32> {
    let c = led_linear(profile, r, g, b)?;
    let whiteness = c[0].min(c[1]).min(c[2]);
    let peak = LED_PEAK + (255.0 - LED_PEAK) * whiteness;
    let [r, g, b] = c.map(|v| (v.powf(1.0 / LED_GAMMA) * peak).round().min(255.0) as u8);
    Some(egui::Color32::from_rgb(r, g, b))
}

/// Share of a channel's overshoot that spills into the other channels in
/// [`led_color_hot`].
pub const LED_SPILL: f32 = 0.5;

/// How an overshoot divides between the channels it spills into: the square
/// roots of the Rec. 709 luminance weights.
#[cfg(feature = "egui-color")]
const SPILL_WEIGHT: [f32; 3] = [0.4611, 0.8457, 0.2687];

/// The emitter itself: [`led_linear`] scaled by `exposure`, with what passes
/// full scale spilling into the other channels by luminance. Brightest
/// channel at 255.
#[cfg(feature = "egui-color")]
pub fn led_color_hot(
    profile: &LedProfile,
    r: u8,
    g: u8,
    b: u8,
    exposure: f32,
) -> Option<egui::Color32> {
    let c = led_linear(profile, r, g, b)?.map(|v| v * exposure);
    let mut out = c.map(|v| v.min(1.0));
    for (i, v) in c.iter().enumerate() {
        let excess = (v - 1.0).max(0.0) * LED_SPILL;
        let others: f32 = (0..3).filter(|&j| j != i).map(|j| SPILL_WEIGHT[j]).sum();
        for j in (0..3).filter(|&j| j != i) {
            out[j] += excess * SPILL_WEIGHT[j] / others;
        }
    }
    let [r, g, b] = out.map(|v| (v.min(1.0).powf(1.0 / LED_GAMMA) * 255.0).round() as u8);
    Some(egui::Color32::from_rgb(r, g, b))
}

/// Brightness level of a step LED.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum StepLed {
    Off,
    Medium,
    Full,
}

impl StepLed {
    /// The step for a level counted 0 off, 1 medium, 2 and over full.
    pub fn from_level(level: u8) -> Self {
        match level {
            0 => StepLed::Off,
            1 => StepLed::Medium,
            _ => StepLed::Full,
        }
    }

    /// The step as a level: 0 off, 1 medium, 2 full.
    pub fn level(self) -> u8 {
        match self {
            StepLed::Off => 0,
            StepLed::Medium => 1,
            StepLed::Full => 2,
        }
    }
}

/// What a frame says about one lamp, in the form the player drives it.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum LampState {
    Bit(bool),
    Step(StepLed),
    /// A plain value, e.g. a jog ring meter.
    Level(u8),
    /// Raw PWM per die, red, green, blue.
    Rgb(u8, u8, u8),
}

/// Drive of a lamp at [`StepLed::Medium`]: half.
const MEDIUM_DRIVE: u8 = 0x80;

impl LampState {
    /// How hard the lamp is driven, 0..255: a colour's brightest channel, a
    /// step at half or full, anything else on at full.
    pub fn drive(self) -> u8 {
        match self {
            LampState::Bit(on) => {
                if on {
                    0xff
                } else {
                    0
                }
            }
            LampState::Step(StepLed::Off) => 0,
            LampState::Step(StepLed::Medium) => MEDIUM_DRIVE,
            LampState::Step(StepLed::Full) => 0xff,
            LampState::Level(v) => {
                if v == 0 {
                    0
                } else {
                    0xff
                }
            }
            LampState::Rgb(r, g, b) => r.max(g).max(b),
        }
    }
}

/// How a player's sub-CPU frame carries its lamps: the MOSI counterpart of
/// [`MisoCodec`](crate::miso_frame::MisoCodec).
pub trait MosiCodec: Sync + std::fmt::Debug {
    /// What `frame` says about `lamp`, or `None` on a player without it.
    fn lamp(&self, frame: &[u8; MOSI_SIZE], lamp: Lamp) -> Option<LampState>;
}

/// A MOSI LED frame received from the guest via /dev/subucom_ctrl, read
/// through one player's [`MosiCodec`].
pub struct MosiFrame {
    bytes: [u8; MOSI_SIZE],
    codec: &'static dyn MosiCodec,
    leds: &'static LedProfiles,
}

impl MosiFrame {
    pub fn new(bytes: [u8; MOSI_SIZE], model: Model) -> Self {
        Self {
            bytes,
            codec: model.spec().mosi,
            leds: &model.spec().leds,
        }
    }

    /// What the frame says about `lamp`, or `None` on a player without it.
    pub fn lamp(&self, lamp: Lamp) -> Option<LampState> {
        self.codec.lamp(&self.bytes, lamp)
    }

    /// How hard `lamp` is driven, 0..255; see [`LampState::drive`].
    pub fn drive(&self, lamp: Lamp) -> u8 {
        self.lamp(lamp).map_or(0, LampState::drive)
    }

    /// Whether `lamp` is lit at all.
    pub fn lit(&self, lamp: Lamp) -> bool {
        self.drive(lamp) > 0
    }

    /// `lamp` as a step: a lamp driven another way reads full when lit.
    pub fn step(&self, lamp: Lamp) -> StepLed {
        match self.lamp(lamp) {
            Some(LampState::Step(s)) => s,
            Some(state) if state.drive() > 0 => StepLed::Full,
            _ => StepLed::Off,
        }
    }

    /// `(R, G, B)` of a lamp the player drives as a colour; `None` for one it
    /// drives as a bit or a step, or does not have.
    pub fn rgb(&self, lamp: Lamp) -> Option<(u8, u8, u8)> {
        match self.lamp(lamp)? {
            LampState::Rgb(r, g, b) => Some((r, g, b)),
            _ => None,
        }
    }

    /// How this player's `part` shows a colour.
    pub fn led_profile(&self, part: LedPart) -> &'static LedProfile {
        self.leds.get(part)
    }

    /// [`led_color`] through this player's `part`.
    #[cfg(feature = "egui-color")]
    pub fn led_color(&self, part: LedPart, r: u8, g: u8, b: u8) -> Option<egui::Color32> {
        led_color(self.led_profile(part), r, g, b)
    }

    /// [`led_color_hot`] through this player's `part`.
    #[cfg(feature = "egui-color")]
    pub fn led_color_hot(
        &self,
        part: LedPart,
        r: u8,
        g: u8,
        b: u8,
        exposure: f32,
    ) -> Option<egui::Color32> {
        led_color_hot(self.led_profile(part), r, g, b, exposure)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cdj3k, cdj3kx};

    fn frame(raw: [u8; MOSI_SIZE], model: Model) -> MosiFrame {
        MosiFrame::new(raw, model)
    }

    fn encoded(profile: &LedProfile, r: u8, g: u8, b: u8) -> [u8; 3] {
        led_linear(profile, r, g, b)
            .unwrap()
            .map(|v| (v.powf(1.0 / LED_GAMMA) * 255.0).round() as u8)
    }

    fn neutral([r, g, b]: [u8; 3]) -> bool {
        r.min(g).min(b) >= 0xf6
    }

    /// Red and green keep the plain gamma mapping; blue is the azure.
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
        // The ring's colour shifts with the bitfield (3 -> 9), but the CDJ-3000X keeps
        // its brightness on a byte of its own as a plain level, where the
        // CDJ-3000 packs it into the colour byte as two bits.
        let mut raw = [0u8; MOSI_SIZE];
        raw[cdj3k::LED_JOG_WHITE.0] = cdj3k::LED_JOG_WHITE.1; // CDJ-3000 byte 3: white, full
        raw[cdj3k::LED_JOG_RED.0 + cdj3kx::BIT_SHIFT] = cdj3k::LED_JOG_RED.1; // the CDJ-3000X's byte 9: red
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
        // The CDJ-3000 bases the CDJ-3000X's transport lamps overlap are pads C and D.
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

        // Captured with both slots idle.
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

    /// [`Lamp::ALL`] names every variant once.
    #[test]
    fn lamp_all_is_complete() {
        for (i, a) in Lamp::ALL.iter().enumerate() {
            assert!(!Lamp::ALL[i + 1..].contains(a), "{a:?} is listed twice");
            // A new variant fails to compile here until it is matched, listed
            // in ALL and counted below.
            match a {
                Lamp::KeySync
                | Lamp::BeatSync
                | Lamp::Master
                | Lamp::Slip
                | Lamp::Quantize
                | Lamp::JogRing
                | Lamp::JogRed
                | Lamp::JogMeter
                | Lamp::Source
                | Lamp::Browse
                | Lamp::TagList
                | Lamp::Playlist
                | Lamp::Search
                | Lamp::Menu
                | Lamp::Play
                | Lamp::Cue
                | Lamp::LoopIn
                | Lamp::LoopOut
                | Lamp::Reloop
                | Lamp::BeatJump4
                | Lamp::BeatJump8
                | Lamp::BeatJumpNext
                | Lamp::BeatJumpPrev
                | Lamp::TempoReset
                | Lamp::MasterTempo
                | Lamp::JogModeCdj
                | Lamp::JogModeVinyl
                | Lamp::Encoder
                | Lamp::TrackSearch
                | Lamp::Rev
                | Lamp::HotA
                | Lamp::HotB
                | Lamp::HotC
                | Lamp::HotD
                | Lamp::HotE
                | Lamp::HotF
                | Lamp::HotG
                | Lamp::HotH
                | Lamp::Slot1
                | Lamp::Slot2
                | Lamp::OnAir
                | Lamp::Rotary
                | Lamp::Standby
                | Lamp::Eject => {}
            }
        }
        assert_eq!(Lamp::ALL.len(), 44);
    }
}
