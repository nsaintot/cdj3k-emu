//! The lamp frame the deck sends back, read through the player's
//! [`MosiCodec`].
//!
//! [`MosiFrame`] reads a frame by [`Lamp`], through the player's codec.

use crate::lamp::Lamp;

pub const MOSI_SIZE: usize = 64;

/// Gamma used when converting raw LED PWM bytes to sRGB display values.
///
/// The deck writes linear-light PWM values: the duty cycle is proportional to
/// the radiant flux. The display is sRGB, with a gamma of about 2.2, so a
/// 1/gamma power curve converts linear light to perceptual brightness and the
/// UI matches the hardware.
///
/// Raise it toward 2.5 if colours look washed out.
pub const LED_GAMMA: f32 = 2.2;

/// Peak output level (0-255) that the dominant LED channel is normalised to.
/// At 255 the channel is at full brightness; a lower value reduces the
/// intensity.
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

/// The drive of a fully lit hot cue pad. The deck dims a pad by lowering all
/// three channels, so a pad's level is its brightest channel divided by this
/// value.
pub const PAD_FULL_DRIVE: u8 = 0x7f;

/// Exponent applied to a pad's level. A value below 1 lifts dim pads: the dim
/// drive `0x0c` keeps about 45 % of its colour.
pub const PAD_LEVEL_CURVE: f32 = 0.34;

/// The grey a pad's colour fades to as its level drops, so a dim pad shows a
/// muted tint of its colour.
pub const PAD_DIM_GREY: f32 = 80.0;

/// How much of its colour a hot cue pad shows, from 0 to 1. A drive of
/// [`PAD_FULL_DRIVE`] gives 1; lower drives follow [`PAD_LEVEL_CURVE`].
/// Returns `None` when the pad is off.
#[inline]
pub fn pad_level(r: u8, g: u8, b: u8) -> Option<f32> {
    let peak = r.max(g).max(b);
    if peak == 0 {
        return None;
    }
    let level = (peak as f32 / PAD_FULL_DRIVE as f32).min(1.0);
    Some(level.powf(PAD_LEVEL_CURVE))
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
    /// The light output for a PWM duty is `duty^gamma`.
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
    /// Uses `p` for every part.
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
/// Dies with a bluish green whose red component lies outside sRGB, and the
/// azure blue.
pub const TEAL_GREEN_DIES: [[f32; 3]; 3] = [
    [1.0, 0.0, 0.0],
    [-0.0846, 1.0, 0.2176],
    [0.0446, 0.1975, 1.0],
];

/// A drive whose weakest channel is at least [`WHITE_BALANCE_FULL`] of its
/// strongest gets the full white balance of a `balance_whites_only` profile; a
/// drive at [`WHITE_BALANCE_FROM`] or below gets none.
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
    led_color_at(profile, r, g, b, 1.0)
}

/// [`led_color`] mixed with [`PAD_DIM_GREY`]. A `level` of 1 gives the full
/// colour, and lower levels move it toward the grey.
#[cfg(feature = "egui-color")]
#[inline]
pub fn led_color_at(
    profile: &LedProfile,
    r: u8,
    g: u8,
    b: u8,
    level: f32,
) -> Option<egui::Color32> {
    let c = led_linear(profile, r, g, b)?;
    let whiteness = c[0].min(c[1]).min(c[2]);
    let peak = LED_PEAK + (255.0 - LED_PEAK) * whiteness;
    let [r, g, b] = c.map(|v| {
        let full = (v.powf(1.0 / LED_GAMMA) * peak).min(255.0);
        (PAD_DIM_GREY + (full - PAD_DIM_GREY) * level).round() as u8
    });
    Some(egui::Color32::from_rgb(r, g, b))
}

/// Share of a channel's overshoot that spills into the other channels in
/// [`led_color_hot`].
pub const LED_SPILL: f32 = 0.5;

/// How an overshoot divides between the channels it spills into: the square
/// roots of the Rec. 709 luminance weights.
#[cfg(feature = "egui-color")]
const SPILL_WEIGHT: [f32; 3] = [0.4611, 0.8457, 0.2687];

/// The colour of the emitter itself: [`led_linear`] scaled by `exposure`. The
/// part of a channel above full scale spills into the other channels, weighted
/// by luminance.
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
    /// A plain value, such as a jog ring meter.
    Level(u8),
    /// Raw PWM per die, red, green, blue.
    Rgb(u8, u8, u8),
}

/// The drive of a lamp at [`StepLed::Medium`], which is half.
const MEDIUM_DRIVE: u8 = 0x80;

impl LampState {
    /// How hard the lamp is driven, 0..255: a colour's brightest channel, a
    /// step at half or full, and any other lamp at full when it is on.
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
    pub fn new(
        bytes: [u8; MOSI_SIZE],
        codec: &'static dyn MosiCodec,
        leds: &'static LedProfiles,
    ) -> Self {
        Self { bytes, codec, leds }
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

    /// `lamp` as a step. A lamp driven another way reads as full when lit.
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

    /// How much of its colour `part` shows for `(R, G, B)`, from 0 to 1. Only
    /// the pads show their drive level; the other parts show full colour.
    #[cfg(feature = "egui-color")]
    fn level(part: LedPart, r: u8, g: u8, b: u8) -> f32 {
        match part {
            LedPart::Pad => pad_level(r, g, b).unwrap_or(0.0),
            _ => 1.0,
        }
    }

    /// [`led_color`] through this player's `part`, at the part's level.
    #[cfg(feature = "egui-color")]
    pub fn led_color(&self, part: LedPart, r: u8, g: u8, b: u8) -> Option<egui::Color32> {
        let level = Self::level(part, r, g, b);
        led_color_at(self.led_profile(part), r, g, b, level)
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

    /// A dim pad shows a muted tint of a fully lit pad's colour, and an off
    /// pad has no colour. The other parts ignore the level.
    #[cfg(feature = "egui-color")]
    #[test]
    fn pads_show_their_drive_level() {
        let mosi = crate::Model::Cdj3k.mosi_frame([0; MOSI_SIZE]);
        let pad = |r, g, b| mosi.led_color(LedPart::Pad, r, g, b);
        let full = pad(0x7f, 0, 0).unwrap();
        let dim = pad(0x0c, 0, 0).unwrap();
        assert!(dim.r() < full.r(), "dim {dim:?} full {full:?}");
        assert!(dim.r() >= dim.g() + 0x30, "dim {dim:?} has lost its hue");
        assert_eq!(dim.g(), dim.b());
        assert!(dim.g() > full.g(), "dim {dim:?} is not desaturated");
        let white = pad(0x44, 0x78, 0x7f).unwrap();
        let dim_white = pad(0x06, 0x0c, 0x0c).unwrap();
        assert!(dim_white.r() < white.r());
        assert!(dim_white.r().abs_diff(dim_white.b()) < 8, "{dim_white:?}");
        assert!(pad(0, 0, 0).is_none());
        let slot = |r, g, b| mosi.led_color(LedPart::Slot, r, g, b);
        assert_eq!(slot(0x0c, 0, 0), slot(0x7f, 0, 0));
    }
}
