// Like miso_frame, most LED tables are read by name from the UI crate.
// Cross-crate string-style references aren't visible to the dead-code lint
// → silenced at file scope rather than per-item.
#![allow(dead_code)]

pub const MOSI_SIZE: usize = 64;

/// Gamma used when converting raw LED PWM bytes to sRGB display values.
///
/// EP122 writes linear-light PWM values (duty cycle ∝ radiant flux).
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
pub type LedBit = (usize, u8);

/// Two-step LED location: byte offset + `medium`/`full` bitmasks.
///
/// EP122 encodes each step LED in two bits: setting only `medium` means
/// "dimly lit"; setting both `medium` and `full` means "fully lit". Decode
/// via [`led_step`] or [`MosiFrame::step_led`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct StepLedMask {
    pub byte: usize,
    pub medium: u8,
    pub full: u8,
}

impl StepLedMask {
    pub const fn new(byte: usize, medium: u8, full: u8) -> Self {
        Self { byte, medium, full }
    }
}

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

// Jog illumination brightness field (byte 3, bits 2:3)
pub const LED_JOG_BRT_BYTE: usize = 3;
pub const LED_JOG_BRT_OFF: u8 = 0x00;
pub const LED_JOG_BRT_1: u8 = 0x04;
pub const LED_JOG_BRT_2: u8 = 0x08;

/// Brightness level reported by a 2-bit step LED field.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum StepLed {
    Off,
    Medium,
    Full,
}

/// Decode a 2-step nav LED from the MOSI frame.
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

// HOT CUE pad RGB base bytes (A-H, 3 bytes each: R, G, B at base+0..+2)
pub const HOT_CUE_BASE: [usize; 8] = [12, 15, 18, 21, 24, 27, 30, 33];

// Indicator RGB base bytes
pub const SD_LED_BASE: usize = 36;
pub const USB_LED_BASE: usize = 39;
pub const ON_AIR_LED_BASE: usize = 42;

/// A 64-byte MOSI LED frame received from the guest via /dev/subucom_ctrl.
pub struct MosiFrame([u8; MOSI_SIZE]);

impl MosiFrame {
    pub fn from_bytes(frame: [u8; MOSI_SIZE]) -> Self {
        Self(frame)
    }

    /// Returns `true` if the LED bit `(byte, mask)` is non-zero.
    pub fn led_bit(&self, led: LedBit) -> bool {
        let (byte, mask) = led;
        self.0[byte] & mask != 0
    }

    /// Decode a 2-step LED field via [`led_step`].
    pub fn step_led(&self, led: StepLedMask) -> StepLed {
        led_step(&self.0, led)
    }

    /// Returns `(R, G, B)` for hot cue pad `pad` (0 = A … 7 = H).
    pub fn pad_rgb(&self, pad: usize) -> (u8, u8, u8) {
        let base = HOT_CUE_BASE[pad];
        (self.0[base], self.0[base + 1], self.0[base + 2])
    }

    /// Returns `(R, G, B)` for the SD LED (bytes 36-38).
    pub fn sd_rgb(&self) -> (u8, u8, u8) {
        (
            self.0[SD_LED_BASE],
            self.0[SD_LED_BASE + 1],
            self.0[SD_LED_BASE + 2],
        )
    }

    /// Returns `(R, G, B)` for the USB LIGHT LED (bytes 39-41).
    pub fn usb_rgb(&self) -> (u8, u8, u8) {
        (
            self.0[USB_LED_BASE],
            self.0[USB_LED_BASE + 1],
            self.0[USB_LED_BASE + 2],
        )
    }

    /// Returns `(R, G, B)` for the ON AIR LED (bytes 42-44).
    pub fn on_air_rgb(&self) -> (u8, u8, u8) {
        (
            self.0[ON_AIR_LED_BASE],
            self.0[ON_AIR_LED_BASE + 1],
            self.0[ON_AIR_LED_BASE + 2],
        )
    }

    pub fn as_bytes(&self) -> &[u8; MOSI_SIZE] {
        &self.0
    }
}
