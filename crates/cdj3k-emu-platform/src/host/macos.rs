//! macOS: HVF, CoreAudio.

use super::{Accelerator, AudioBackend, SoftwareEmulation};

/// HVF with the in-kernel vGIC where the release has one (`hv_gic_create`,
/// macOS 15+), then HVF with the GIC emulated in QEMU. The in-kernel vGIC
/// also needs the hypervisor entitlement, so the second rung stays listed.
pub(super) fn accelerators() -> Result<Vec<Accelerator>, SoftwareEmulation> {
    let mut out = Vec::with_capacity(2);
    if macos_major_version() >= 15 {
        out.push(Accelerator {
            name: "hvf",
            in_kernel_gic: true,
        });
    }
    out.push(Accelerator {
        name: "hvf",
        in_kernel_gic: false,
    });
    Ok(out)
}

pub const AUDIO: Option<AudioBackend> = Some(AudioBackend {
    driver: "coreaudio",
    // The HAL clamps the buffer to about 13.78 ms on Apple silicon.
    options: ",out.buffer-length=5000",
    device_selector: ",out.device-uid=",
});

pub const RNG_OBJECT: &str = "rng-random,id=rng0,filename=/dev/urandom";

/// macOS major version (15 = Sequoia), or 0 when `uname` fails.
fn macos_major_version() -> u32 {
    static CACHED: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *CACHED.get_or_init(|| {
        // macOS major = Darwin major − 9, from macOS 11.
        match darwin_major() {
            major if major >= 20 => major - 9,
            _ => 0,
        }
    })
}

fn darwin_major() -> u32 {
    let mut uts: libc::utsname = unsafe { std::mem::zeroed() };
    // SAFETY: `uname` writes into a `utsname` this frame owns.
    if unsafe { libc::uname(&mut uts) } != 0 {
        return 0;
    }
    // SAFETY: `uname` NUL-terminates `release`.
    let release = unsafe { std::ffi::CStr::from_ptr(uts.release.as_ptr()) };
    release
        .to_string_lossy()
        .split('.')
        .next()
        .and_then(|t| t.parse().ok())
        .unwrap_or(0)
}

/// The primary and shift modifiers as a shortcut hint spells them.
pub const KEY_PRIMARY: &str = "⌘";
pub const KEY_SHIFT: &str = "⇧";

/// Extension of a new virtual USB image: a raw disk image.
pub const VIRTUAL_IMAGE_EXT: &str = "img";

#[cfg(test)]
mod tests {
    use super::*;

    /// A vGIC failure falls back to HVF, never straight to TCG.
    #[test]
    fn hvf_is_always_a_rung() {
        let rungs = accelerators().unwrap();
        assert!(rungs.iter().all(|a| a.name == "hvf"), "{rungs:?}");
        assert_eq!(rungs.last().map(|a| a.in_kernel_gic), Some(false));
    }
}
