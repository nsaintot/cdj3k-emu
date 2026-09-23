//! The lamp frame the deck sends back, and the CDJ-3000 coordinates every
//! model's [`MosiMap`] is expressed relative to.
//!
//! [`MosiFrame`] reads a lamp through the map, never at a literal offset: a
//! raw index is legal Rust and compiles against any model, which is how the
//! transport lamps once rendered on the hot cue pads and the USB glow read hot
//! cue F. Nothing here hands out the frame bytes.

pub use crate::frame::StepLedMask;
use crate::frame::{FrameBit, JogBrightness, MosiMap};
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

/// Single-bit LED location: `(byte_offset, bitmask)`.
pub type LedBit = FrameBit;

// Byte 2: sync / master LEDs
pub const LED_KEY_SYNC: LedBit = (2, 0x03);
pub const LED_BEAT_SYNC: LedBit = (2, 0x08);
pub const LED_MASTER: LedBit = (2, 0x20);

// Byte 3: mode LEDs + jog illumination
// SLIP and JILMR share bits (0x03); QUANTIZE and JILMW share bits (0x0c).
pub const LED_SLIP: StepLedMask = StepLedMask::new(3, 0x10, 0x30);
pub const LED_QUANTIZE: StepLedMask = StepLedMask::new(3, 0x40, 0xc0);
pub const LED_JOG_WHITE: LedBit = (3, 0x0c); // JILMW
pub const LED_JOG_RED: LedBit = (3, 0x03); // JILMR

/// Brightness level reported by a 2-bit step LED field.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum StepLed {
    Off,
    Medium,
    Full,
}

/// Decode a 2-step nav LED from a frame at an already-mapped offset.
pub fn led_step(frame: &[u8], led: StepLedMask) -> StepLed {
    let bits = frame[led.byte] & led.full;
    if bits == led.full {
        StepLed::Full
    } else if bits & led.medium != 0 {
        StepLed::Medium
    } else {
        StepLed::Off
    }
}

// Byte 4: navigation LEDs
pub const LED_SOURCE: StepLedMask = StepLedMask::new(4, 0x01, 0x03);
pub const LED_BROWSE: StepLedMask = StepLedMask::new(4, 0x04, 0x0c);
pub const LED_TAG_LIST: StepLedMask = StepLedMask::new(4, 0x10, 0x30);
pub const LED_PLAYLIST: StepLedMask = StepLedMask::new(4, 0x40, 0xc0);

// Byte 5: navigation LEDs (continued; base value 0xc0 always set - IC6003 dim)
pub const LED_SEARCH: StepLedMask = StepLedMask::new(5, 0x01, 0x03);
pub const LED_MENU: StepLedMask = StepLedMask::new(5, 0x04, 0x0c);

// Byte 7: transport + loop LEDs
pub const LED_PLAY: LedBit = (7, 0x01);
pub const LED_CUE: LedBit = (7, 0x02);
pub const LED_LOOP_IN: LedBit = (7, 0x08);
pub const LED_LOOP_OUT: LedBit = (7, 0x10);
pub const LED_RELOOP: LedBit = (7, 0x20);
pub const LED_BEAT_JUMP_4: LedBit = (7, 0x40);
pub const LED_BEAT_JUMP_8: LedBit = (7, 0x80);

// Byte 8: beat-jump direction / tempo / jog mode LEDs
pub const LED_BEAT_JUMP_NEXT: LedBit = (8, 0x01);
pub const LED_BEAT_JUMP_PREV: LedBit = (8, 0x02);
pub const LED_TEMPO_RESET: LedBit = (8, 0x08);
pub const LED_MASTER_TEMPO: LedBit = (8, 0x10);
pub const LED_JOG_MODE_CDJ: LedBit = (8, 0x40);
pub const LED_JOG_MODE_VINYL: LedBit = (8, 0x80);

// Byte 9: encoder / search / rev LEDs
pub const LED_ENCODER: LedBit = (9, 0x01);
pub const LED_TRACK_SEARCH: LedBit = (9, 0x04);
pub const LED_REV: LedBit = (9, 0x10);

/// A 64-byte MOSI LED frame received from the guest via /dev/subucom_ctrl,
/// read through one player's [`MosiMap`].
pub struct MosiFrame {
    bytes: [u8; MOSI_SIZE],
    map: &'static MosiMap,
    leds: &'static LedProfiles,
}

impl MosiFrame {
    pub fn new(bytes: [u8; MOSI_SIZE], model: Model) -> Self {
        Self {
            bytes,
            map: &model.spec().mosi,
            leds: &model.spec().leds,
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

    /// The frame byte holding shared bitfield byte `byte`.
    fn bit_byte(&self, byte: usize) -> usize {
        byte + self.map.bit_shift
    }

    fn rgb_at(&self, base: Option<usize>) -> Option<(u8, u8, u8)> {
        let base = base?;
        Some((self.bytes[base], self.bytes[base + 1], self.bytes[base + 2]))
    }

    /// Returns `true` if the LED bit `(byte, mask)` is non-zero.
    pub fn led_bit(&self, led: LedBit) -> bool {
        let (byte, mask) = led;
        self.bytes[self.bit_byte(byte)] & mask != 0
    }

    /// Jog ring brightness as a level: 0 off, 1 dim, 2 full.
    pub fn jog_level(&self) -> u8 {
        match self.map.jog {
            JogBrightness::Bits { byte, dim, bright } => {
                match self.bytes[self.bit_byte(byte)] & (dim | bright) {
                    0 => 0,
                    v if v == dim => 1,
                    _ => 2,
                }
            }
            JogBrightness::Level { byte } => self.bytes[byte].min(2),
        }
    }

    /// Decode a 2-step LED field via [`led_step`].
    pub fn step_led(&self, led: StepLedMask) -> StepLed {
        led_step(
            &self.bytes,
            StepLedMask {
                byte: self.bit_byte(led.byte),
                ..led
            },
        )
    }

    /// `(R, G, B)` of hot cue pad `pad` (0 = A … 7 = H).
    pub fn pad_rgb(&self, pad: usize) -> (u8, u8, u8) {
        self.rgb_at(Some(self.map.lamps.hot_cue[pad]))
            .unwrap_or((0, 0, 0))
    }

    /// `(R, G, B)` of media slot 1 - SD on the CDJ-3000, USB 1 on the
    /// CDJ-3000X - on a player that has one.
    pub fn slot_1_rgb(&self) -> Option<(u8, u8, u8)> {
        self.rgb_at(self.map.lamps.slot_1)
    }

    /// `(R, G, B)` of media slot 2 - USB on the CDJ-3000, USB 2 on the
    /// CDJ-3000X.
    pub fn slot_2_rgb(&self) -> Option<(u8, u8, u8)> {
        self.rgb_at(self.map.lamps.slot_2)
    }

    /// `(R, G, B)` of the ON AIR bar, on a player that has one.
    pub fn on_air_rgb(&self) -> Option<(u8, u8, u8)> {
        self.rgb_at(self.map.lamps.on_air)
    }

    /// `(R, G, B)` of the rotary selector ring, on a player that lights it.
    pub fn rotary_rgb(&self) -> Option<(u8, u8, u8)> {
        self.rgb_at(self.map.lamps.rotary)
    }

    /// `(R, G, B)` of the PLAY lamp, on a player that drives it as a colour.
    /// The CDJ-3000 drives PLAY from a bitfield bit ([`LED_PLAY`]) and
    /// returns `None` here.
    pub fn play_rgb(&self) -> Option<(u8, u8, u8)> {
        self.rgb_at(self.map.lamps.play)
    }

    /// `(R, G, B)` of the CUE lamp - the [`Self::play_rgb`] rules apply, with
    /// [`LED_CUE`] as the CDJ-3000's bit.
    pub fn cue_rgb(&self) -> Option<(u8, u8, u8)> {
        self.rgb_at(self.map.lamps.cue)
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

    /// Every lamp read has to go through the map: reading a CDJ-3000 offset
    /// straight out of a CDJ-3000X frame has produced three separate mis-renders
    /// (hot cue pads, the ON AIR bar, the jog ring).
    #[test]
    fn lamp_reads_follow_the_map() {
        // The ring's colour shifts with the bitfield (3 -> 9), but the CDJ-3000X keeps
        // its brightness on a byte of its own as a plain level, where the
        // CDJ-3000 packs it into the colour byte as two bits.
        let mut raw = [0u8; MOSI_SIZE];
        raw[LED_JOG_WHITE.0] = LED_JOG_WHITE.1; // CDJ-3000 byte 3: white, full
        raw[LED_JOG_RED.0 + cdj3kx::BIT_SHIFT] = LED_JOG_RED.1; // the CDJ-3000X's byte 9: red
        raw[2] = 2; // the CDJ-3000X's own brightness byte: full
        assert_eq!(frame(raw, Model::Cdj3k).jog_level(), 2);
        assert!(!frame(raw, Model::Cdj3k).led_bit(LED_JOG_RED));
        assert_eq!(frame(raw, Model::Cdj3kx).jog_level(), 2);
        assert!(frame(raw, Model::Cdj3kx).led_bit(LED_JOG_RED));
        // A level the CDJ-3000's bits cannot express still reads on the CDJ-3000X.
        raw[2] = 1;
        assert_eq!(frame(raw, Model::Cdj3kx).jog_level(), 1);

        let mut raw = [0u8; MOSI_SIZE];
        raw[cdj3k::SPEC.mosi.lamps.hot_cue[0]] = 0x11;
        raw[cdj3kx::SPEC.mosi.lamps.hot_cue[0]] = 0x22;
        assert_eq!(frame(raw, Model::Cdj3k).pad_rgb(0).0, 0x11);
        assert_eq!(frame(raw, Model::Cdj3kx).pad_rgb(0).0, 0x22);
    }

    /// PLAY and CUE own bytes 18-23 on the CDJ-3000X, which is where the CDJ-3000's
    /// hot cue C and D sit: a pad read that skips the map renders the
    /// transport lamps on the pads.
    #[test]
    fn the_cdj3000x_transport_lamps_are_not_hot_cue_pads() {
        let play = cdj3kx::SPEC.mosi.lamps.play.unwrap();
        let cue = cdj3kx::SPEC.mosi.lamps.cue.unwrap();
        let mut raw = [0u8; MOSI_SIZE];
        raw[play] = 0x31;
        raw[cue] = 0x42;
        let x = frame(raw, Model::Cdj3kx);
        assert_eq!(x.play_rgb().map(|rgb| rgb.0), Some(0x31));
        assert_eq!(x.cue_rgb().map(|rgb| rgb.0), Some(0x42));
        // The CDJ-3000 bases the CDJ-3000X's transport lamps overlap are pads C and D.
        assert_eq!(cdj3k::SPEC.mosi.lamps.hot_cue[2], play);
        assert_eq!(cdj3k::SPEC.mosi.lamps.hot_cue[3], cue);
        for pad in 0..8 {
            assert_eq!(x.pad_rgb(pad), (0, 0, 0), "pad {pad} caught a lamp");
        }
        // Only the CDJ-3000X splits them out.
        assert_eq!(frame(raw, Model::Cdj3k).play_rgb(), None);
        assert_eq!(frame(raw, Model::Cdj3k).cue_rgb(), None);
    }

    /// The CDJ-3000X's media slots and selector ring are one 3-byte grid, and the ring
    /// lands on the offset the CDJ-3000's ON AIR bar maps to: a player that
    /// has no ON AIR must not read one there.
    #[test]
    fn the_cdj3000x_slot_grid_ends_at_the_selector_ring() {
        let lamps = cdj3kx::SPEC.mosi.lamps;
        let slot_1 = lamps.slot_1.unwrap();
        assert_eq!(lamps.slot_2, Some(slot_1 + 3));
        assert_eq!(lamps.rotary, Some(slot_1 + 6));

        // Captured with both slots idle.
        let wire: [u8; 9] = [0x0a, 0x08, 0x07, 0x0a, 0x08, 0x07, 0x4a, 0x57, 0x39];
        let mut raw = [0u8; MOSI_SIZE];
        raw[slot_1..slot_1 + 9].copy_from_slice(&wire);
        let x = frame(raw, Model::Cdj3kx);
        assert_eq!(x.slot_1_rgb(), Some((0x0a, 0x08, 0x07)));
        assert_eq!(x.slot_2_rgb(), Some((0x0a, 0x08, 0x07)));
        assert_eq!(x.rotary_rgb(), Some((0x4a, 0x57, 0x39)));
        assert_eq!(x.on_air_rgb(), None);
        assert_eq!(cdj3k::SPEC.mosi.lamps.on_air, lamps.rotary.map(|r| r - 12));

        // The CDJ-3000 has the bar and not the ring.
        assert_eq!(frame(raw, Model::Cdj3k).rotary_rgb(), None);
        assert!(frame(raw, Model::Cdj3k).on_air_rgb().is_some());
    }

    /// No two lamps of one player may name the same byte: a lamp copied from
    /// another map without re-reading the deck lands on its neighbour.
    #[test]
    fn a_players_lamps_do_not_overlap() {
        for model in Model::ALL {
            let lamps = model.spec().mosi.lamps;
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
                assert!(a + 2 < MOSI_SIZE, "{model} lamp at {a} runs off the frame");
                for b in &bases[i + 1..] {
                    assert!(a.abs_diff(*b) >= 3, "{model} lamps at {a} and {b} overlap");
                }
            }
        }
    }
}
