//! Host detection, and the per-host values the QEMU command line and the
//! menus take.
//!
//! This file holds the types and the logic every host shares; each host's
//! file answers for that host, and `unsupported.rs` for a host with none.

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod imp;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
#[path = "unsupported.rs"]
mod imp;

pub use imp::{AUDIO, KEY_PRIMARY, KEY_SHIFT, RNG_OBJECT};

/// A hypervisor QEMU can accelerate the guest with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accelerator {
    /// The `-accel` value.
    pub name: &'static str,
    /// Whether the GIC is the hypervisor's own (`kernel-irqchip=on`) rather
    /// than emulated in QEMU.
    pub in_kernel_gic: bool,
}

/// Why the guest runs under software emulation (TCG).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoftwareEmulation {
    /// `CDJ3K_EMU_TCG` is set.
    Forced,
    /// The host CPU is not arm64. Carries the host architecture.
    ForeignArch(&'static str),
    /// arm64 Linux with no `/dev/kvm`.
    NoKvm,
    /// `/dev/kvm` exists but this user cannot open it.
    KvmDenied,
    /// No accelerator is wired up for this OS.
    Unsupported,
}

/// The accelerators worth trying, fastest first, or why there are none.
///
/// Only the host's own hypervisor is named: QEMU exits on an accelerator it
/// was not built with.
pub fn accelerators() -> Result<Vec<Accelerator>, SoftwareEmulation> {
    if std::env::var_os("CDJ3K_EMU_TCG").is_some() {
        return Err(SoftwareEmulation::Forced);
    }
    // A hypervisor only runs a guest of the host's own architecture.
    if std::env::consts::ARCH != "aarch64" {
        return Err(SoftwareEmulation::ForeignArch(std::env::consts::ARCH));
    }
    imp::accelerators()
}

/// The fastest accelerator this host has, if any.
pub fn accelerator() -> Option<Accelerator> {
    accelerators().ok().and_then(|a| a.first().copied())
}

/// The reason the guest runs under TCG, or `None` when it is accelerated.
pub fn software_emulation() -> Option<SoftwareEmulation> {
    accelerators().err()
}

/// What QEMU's `-audiodev` needs on this host.
pub struct AudioBackend {
    /// The driver QEMU is asked to open.
    pub driver: &'static str,
    /// Options the driver needs beyond its name, or `""`.
    pub options: &'static str,
    /// Prefix of the output device option, spelled per driver: CoreAudio
    /// addresses a device by UID, PipeWire a sink by node name.
    pub device_selector: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host with a backend must have somewhere to put a device.
    #[test]
    fn an_audio_backend_that_exists_can_be_told_which_device() {
        if let Some(backend) = AUDIO {
            assert!(!backend.driver.is_empty());
            assert!(backend.device_selector.starts_with(','));
            assert!(backend.options.is_empty() || backend.options.starts_with(','));
        }
    }

    #[test]
    fn the_rng_object_is_an_object() {
        assert!(RNG_OBJECT.contains(",id=rng0"), "{RNG_OBJECT}");
    }
}
