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

#[inline]
fn led_expanded_rgb(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let expand = |v: u8| (v as f32 / 255.0).powf(1.0 / LED_GAMMA) * 255.0;
    (expand(r), expand(g), expand(b))
}

/// Linear-light peak (0..1) from raw PWM after per-channel gamma expand,
/// before [`LED_PEAK`] scaling. `None` when the LED is off (all channels 0).
///
/// [`led_color`] normalises the dominant channel to [`LED_PEAK`], so hue
/// reads correctly at any drive level but overall intensity is lost.
/// Multiply glow alpha or use [`egui::Color32::gamma_multiply`] with this
/// factor so dim hardware PWM appears dimmer on screen. Callers that just
/// need an alpha multiplier can `.unwrap_or(0.0)`.
#[inline]
pub fn led_drive_factor(r: u8, g: u8, b: u8) -> Option<f32> {
    if r | g | b == 0 {
        return None;
    }
    let (rf, gf, bf) = led_expanded_rgb(r, g, b);
    Some((rf.max(gf).max(bf) / 255.0).clamp(0.0, 1.0))
}

/// Convert a raw LED `(R, G, B)` triple to an egui `Color32`.
/// Applies gamma expansion then normalises so the dominant channel reaches
/// [`LED_PEAK`], preserving hue and saturation regardless of the raw PWM level.
/// Returns `None` when all channels are zero (LED off / unassigned).
#[cfg(feature = "egui-color")]
#[inline]
pub fn led_color(r: u8, g: u8, b: u8) -> Option<egui::Color32> {
    if r | g | b == 0 {
        return None;
    }
    let (rf, gf, bf) = led_expanded_rgb(r, g, b);
    let scale = LED_PEAK / rf.max(gf).max(bf);
    Some(egui::Color32::from_rgb(
        (rf * scale).min(255.0) as u8,
        (gf * scale).min(255.0) as u8,
        (bf * scale).min(255.0) as u8,
    ))
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
}

impl MosiFrame {
    pub fn new(bytes: [u8; MOSI_SIZE], model: Model) -> Self {
        Self {
            bytes,
            map: &model.spec().mosi,
        }
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
