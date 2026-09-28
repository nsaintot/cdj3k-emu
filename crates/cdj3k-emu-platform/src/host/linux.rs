//! Linux: KVM, PipeWire.

use super::{Accelerator, AudioBackend, SoftwareEmulation};

/// KVM, whose GIC is always in-kernel. Readable, not merely present: a user
/// outside the `kvm` group has the node but cannot open it, and QEMU exits
/// rather than degrading.
pub(super) fn accelerators() -> Result<Vec<Accelerator>, SoftwareEmulation> {
    match std::fs::OpenOptions::new().read(true).open("/dev/kvm") {
        Ok(_) => Ok(vec![Accelerator {
            name: "kvm",
            in_kernel_gic: true,
        }]),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(SoftwareEmulation::NoKvm),
        Err(_) => Err(SoftwareEmulation::KvmDenied),
    }
}

pub const AUDIO: Option<AudioBackend> = Some(AudioBackend {
    driver: "pipewire",
    // The stream runs at the guest's own rate instead of audiodev's fixed
    // 44.1 kHz, so the player's 96 kHz is resampled once, by PipeWire.
    options: ",out.fixed-settings=off",
    device_selector: ",out.name=",
});

pub const RNG_OBJECT: &str = "rng-random,id=rng0,filename=/dev/urandom";

/// The primary and shift modifiers as a shortcut hint spells them.
pub const KEY_PRIMARY: &str = "Ctrl ";
pub const KEY_SHIFT: &str = "Shift ";
