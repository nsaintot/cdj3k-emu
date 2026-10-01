//! A host with no adapter: no accelerator, no audio backend.

use super::{Accelerator, AudioBackend, SoftwareEmulation};

pub(super) fn accelerators() -> Result<Vec<Accelerator>, SoftwareEmulation> {
    Err(SoftwareEmulation::Unsupported)
}

/// No `-audiodev` at all, which QEMU runs with: the guest's `snd-dummy` still
/// enumerates a card.
pub const AUDIO: Option<AudioBackend> = None;

/// `rng-builtin` is the entropy backend QEMU builds on every host;
/// `rng-random` reads a host file and is not built on Windows.
pub const RNG_OBJECT: &str = "rng-builtin,id=rng0";

/// The primary and shift modifiers as a shortcut hint spells them.
pub const KEY_PRIMARY: &str = "Ctrl ";
pub const KEY_SHIFT: &str = "Shift ";

/// Extension of a new virtual USB image: a raw disk image.
pub const VIRTUAL_IMAGE_EXT: &str = "img";
