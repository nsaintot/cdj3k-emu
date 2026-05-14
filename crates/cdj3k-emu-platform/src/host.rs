//! Host OS feature detection. One-shot, cached for the life of the process.

use std::sync::OnceLock;

/// macOS major version (15 = Sequoia, 14 = Sonoma, 13 = Ventura, …).
/// Returns 0 if the version can't be determined (non-macOS or `uname` fails).
///
/// Mapping: macOS major = Darwin major − 9 (for macOS 11+).
pub fn macos_major_version() -> u32 {
    static CACHED: OnceLock<u32> = OnceLock::new();
    *CACHED.get_or_init(|| {
        #[cfg(not(target_os = "macos"))]
        {
            return 0;
        }
        #[cfg(target_os = "macos")]
        {
            let mut uts: libc::utsname = unsafe { std::mem::zeroed() };
            if unsafe { libc::uname(&mut uts) } != 0 {
                return 0;
            }
            let release = unsafe { std::ffi::CStr::from_ptr(uts.release.as_ptr()) };
            let darwin_major: u32 = release
                .to_string_lossy()
                .split('.')
                .next()
                .and_then(|t| t.parse().ok())
                .unwrap_or(0);
            if darwin_major >= 20 {
                darwin_major - 9
            } else {
                0
            }
        }
    })
}

/// True when QEMU/HVF can use the in-kernel ARM vGIC
/// (`hv_gic_create`, macOS 15+).
///
/// On older macOS releases the host hypervisor has no GIC primitive, so QEMU
/// must emulate the GIC in userspace - slower per-IRQ cost but functional.
pub fn has_hvf_in_kernel_gic() -> bool {
    macos_major_version() >= 15
}
