//! Everything the rest of the emulator needs to know about one player.
//!
//! A model is a [`ModelSpec`] constant, such as
//! [`cdj3k::SPEC`](crate::cdj3k::SPEC), [`cdj3kx::SPEC`](crate::cdj3kx::SPEC)
//! or [`cdj1500x::SPEC`](crate::cdj1500x::SPEC). To add a player, write one
//! more spec with its MISO encoder and add a [`Model`](crate::Model) variant;
//! no consumer matches on the model.

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
    /// A fixed brake: the seconds the platter takes to stop from one turn a
    /// second.
    Fixed { stop_secs: f32 },
}

/// The panel controls a model has besides its buttons, the jog platter, the
/// rotary selector and the tempo fader.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Controls {
    /// The BEAT LOOP encoder.
    pub beat_loop_encoder: bool,
    /// The direction lever: forward, reverse and slip reverse.
    pub direction_lever: bool,
    /// The VINYL SPEED ADJUST knob.
    pub vinyl_speed_knob: bool,
}

/// The coordinates a model's app expects touch in.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TouchSpace {
    /// Sub-CPU frame units, `0..range`, with the origin at the right edge.
    /// The app reads touch straight from the MISO frame.
    SubCpu { range: u16 },
    /// Screen pixels. The app reads a Goodix input device, so the guest shim
    /// copies these into the evdev records it synthesises and the app hands
    /// them to `XWarpPointer` unscaled. The X word carries [`TOUCH_DOWN`].
    ScreenPixels,
}

/// One player.
#[derive(Copy, Clone, Debug)]
pub struct ModelSpec {
    /// Product name as printed on the chassis.
    pub title: &'static str,
    /// The stable identifier (slug) for directory names and settings.
    pub slug: &'static str,
    /// User spellings that [`Model::parse`](crate::Model::parse) accepts
    /// besides the slug, such as `3000` or `3k`. They are lowercase and have
    /// no `cdj` prefix, because `parse` strips it.
    pub aliases: &'static [&'static str],
    /// Main LCD framebuffer `(width, height)` in pixels. The guest's X server
    /// runs at this mode, so it is also the size of the virtio-gpu scanout and
    /// of the host's LCD texture.
    pub main_lcd: (u32, u32),
    pub touch: TouchSpace,
    /// The `model` U-Boot variable. `genkey_pr` hashes it with the
    /// `/proc/cpuinfo` serial to derive the cabinet.img passphrase, so it has
    /// to match the deck it claims to be.
    pub model_env: &'static str,
    /// The `N` in `/dev/mmcblkN` for the deck's eMMC, which its scripts,
    /// `fw_env.config` and udev rules use. It is `1` on the RK3399 decks and
    /// `0` where the udev rules give `mmcblk1` to the SD card, as on the
    /// CDJ-1500X.
    pub emmc_index: u32,
    /// The U-Boot variable the deck's updater records `images/system.rev`
    /// under: `rev_kernel` on the CDJ-3000, `rev_system` on the CDJ-3000X.
    /// `None` where the environment carries no revision.
    pub system_rev_env: Option<&'static str>,
    /// What Pioneer calls this deck's `.UPD`, without the version suffix, as
    /// in `CDJ3Kv322.UPD` or `CDJ3000Xv140.UPD`. The installer compares the
    /// file's name against these, because a `.UPD` is a bare LUKS container
    /// whose header names no product.
    pub firmware_file_names: &'static [&'static str],
    /// How the player's sub-CPU frames the panel's controls.
    pub miso: &'static dyn MisoCodec,
    /// How the player's sub-CPU frames the panel's lamps.
    pub mosi: &'static dyn MosiCodec,
    /// How each of the player's LED parts shows a colour.
    pub leds: LedProfiles,
    /// How the jog platter's brake is set.
    pub jog_brake: JogBrake,
    /// The optional controls the model has.
    pub controls: Controls,
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
