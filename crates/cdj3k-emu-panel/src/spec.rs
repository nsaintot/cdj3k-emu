//! Everything the rest of the emulator needs to know about one player.
//!
//! A model is a [`ModelSpec`] constant - see [`cdj3k::SPEC`](crate::cdj3k::SPEC),
//! [`cdj3kx::SPEC`](crate::cdj3kx::SPEC) and
//! [`cdj1500x::SPEC`](crate::cdj1500x::SPEC). Adding a player is writing one
//! more of them, with its MISO encoder, and one more [`Model`](crate::Model)
//! variant; no consumer matches on the model.

use crate::miso_frame::MisoCodec;
use crate::mosi_frame::{LedProfiles, MosiCodec};

/// Span of a touch coordinate in the CDJ-3000's sub-CPU frame.
pub const TOUCH_FRAME_RANGE: f32 = 1000.0;

/// Set in the X word of a [`TouchSpace::ScreenPixels`] contact. Pixel 0 is a
/// coordinate there, so `(0, 0)` cannot also mean "no touch"; the guest shim
/// takes a contact from this bit and masks it off the coordinate.
pub const TOUCH_DOWN: u16 = 0x8000;

/// How the jog platter's brake is set. The platter decelerates linearly.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum JogBrake {
    /// By the JOG ADJUST knob.
    Adjustable,
    /// Fixed: the seconds the platter takes to stop from one turn a second.
    Fixed { stop_secs: f32 },
}

/// The coordinates a model's app expects touch in.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TouchSpace {
    /// Sub-CPU frame units, `0..range`, with the origin at the right edge:
    /// the app reads touch straight out of the MISO frame.
    SubCpu { range: u16 },
    /// Screen pixels: the app reads a Goodix input device, so the guest shim
    /// copies these into the evdev records it synthesises and the app hands
    /// them to `XWarpPointer` unscaled. The X word carries [`TOUCH_DOWN`].
    ScreenPixels,
}

/// One player.
#[derive(Copy, Clone, Debug)]
pub struct ModelSpec {
    /// Product name as printed on the chassis.
    pub title: &'static str,
    /// Stable (slug) identifier for directory names and settings.
    pub slug: &'static str,
    /// User spellings [`Model::parse`](crate::Model::parse) accepts besides
    /// the number and the slug, lowercase and without an `ep`/`cdj` prefix.
    pub aliases: &'static [&'static str],
    /// Main LCD framebuffer `(width, height)` in pixels: the mode the guest's
    /// X server runs at, so the virtio-gpu scanout and the host's LCD texture.
    pub main_lcd: (u32, u32),
    pub touch: TouchSpace,
    /// The `model` U-Boot variable. `genkey_pr` hashes it with the
    /// `/proc/cpuinfo` serial to derive the cabinet.img passphrase, so it has
    /// to match the deck it claims to be.
    pub model_env: &'static str,
    /// The `N` of the `/dev/mmcblkN` the deck's eMMC is: its scripts,
    /// `fw_env.config` and udev rules name it. `1` on the RK3399 decks,
    /// `0` where the udev rules take `mmcblk1` for the SD card (e.g. the
    /// CDJ-1500X).
    pub emmc_index: u32,
    /// The U-Boot variable the deck's updater records `images/system.rev`
    /// under: `rev_kernel` on the CDJ-3000, `rev_system` on the CDJ-3000X.
    /// `None` where the environment carries no revision.
    pub system_rev_env: Option<&'static str>,
    /// What Pioneer calls this deck's `.UPD`, version suffix aside:
    /// `CDJ3Kv322.UPD`, `CDJ3000Xv140.UPD`. The installer has nothing else to
    /// go on - a `.UPD` is a bare LUKS container and its header names no
    /// product - so it compares the file's name against these.
    pub firmware_file_names: &'static [&'static str],
    /// How the player's sub-CPU frames the panel's controls.
    pub miso: &'static dyn MisoCodec,
    /// How the player's sub-CPU frames the panel's lamps.
    pub mosi: &'static dyn MosiCodec,
    /// How each of the player's LED parts shows a colour.
    pub leds: LedProfiles,
    /// How the jog platter's brake is set.
    pub jog_brake: JogBrake,
}

impl ModelSpec {
    /// The frame's touch words for a contact at a point given as fractions
    /// of the display (0..1 from its top-left corner), in the units this
    /// model's app expects.
    pub fn touch_units(&self, fx: f32, fy: f32) -> (u16, u16) {
        let fx = fx.clamp(0.0, 1.0);
        let fy = fy.clamp(0.0, 1.0);
        match self.touch {
            TouchSpace::SubCpu { range } => {
                let range = range as f32;
                ((range - fx * range) as u16, (fy * range) as u16)
            }
            TouchSpace::ScreenPixels => {
                let (w, h) = self.main_lcd;
                let x = (fx * (w - 1) as f32) as u16;
                (x | TOUCH_DOWN, (fy * (h - 1) as f32) as u16)
            }
        }
    }
}
