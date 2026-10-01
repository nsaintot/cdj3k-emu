//! Windows: WHPX on arm64, WASAPI.

use windows::core::{s, w, BOOL};
use windows::Win32::Foundation::FreeLibrary;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::PCWSTR;
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};

use super::{whpx, Accelerator, AudioBackend, SoftwareEmulation};

/// `WHvCapabilityCodeHypervisorPresent`.
const CAPABILITY_HYPERVISOR_PRESENT: i32 = 0;

type GetCapability = unsafe extern "system" fn(i32, *mut BOOL, u32, *mut u32) -> i32;

/// WHPX, whose arm64 GIC is always in-hypervisor. Called on arm64 hosts only.
pub(super) fn accelerators() -> Result<Vec<Accelerator>, SoftwareEmulation> {
    let (build, ubr) = windows_build();
    if !whpx::build_supports_arm64_whpx(build, ubr) {
        return Err(SoftwareEmulation::WindowsTooOld { build, ubr });
    }
    if !hypervisor_platform_present() {
        return Err(SoftwareEmulation::NoWhpx);
    }
    Ok(vec![Accelerator {
        name: "whpx",
        in_kernel_gic: true,
    }])
}

/// `WHvGetCapability(HypervisorPresent)`. `WinHvPlatform.dll` ships with the
/// Windows Hypervisor Platform feature, so it is loaded at run time.
fn hypervisor_platform_present() -> bool {
    // SAFETY: the library is unloaded after its one call; the signature is
    // WHvGetCapability's, and the out-parameters are locals.
    unsafe {
        let Ok(lib) = LoadLibraryW(w!("WinHvPlatform.dll")) else {
            return false;
        };
        let present = GetProcAddress(lib, s!("WHvGetCapability")).is_some_and(|f| {
            let f: GetCapability = std::mem::transmute(f);
            let mut value = BOOL(0);
            let mut written = 0u32;
            let hr = f(
                CAPABILITY_HYPERVISOR_PRESENT,
                &mut value,
                std::mem::size_of::<BOOL>() as u32,
                &mut written,
            );
            hr >= 0 && value.as_bool()
        });
        let _ = FreeLibrary(lib);
        present
    }
}

const CURRENT_VERSION: PCWSTR = w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");

/// The OS build number and its update revision, or zeros where unreadable.
///
/// From the registry: `GetVersionExW` reports 6.2.9200 to an executable
/// without a compatibility manifest.
fn windows_build() -> (u32, u32) {
    let build = reg_string(w!("CurrentBuildNumber"))
        .and_then(|b| b.trim().parse().ok())
        .unwrap_or(0);
    (build, reg_dword(w!("UBR")).unwrap_or(0))
}

fn reg_dword(name: PCWSTR) -> Option<u32> {
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: a DWORD value read into a DWORD this frame owns.
    unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            CURRENT_VERSION,
            name,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    }
    .is_ok()
    .then_some(value)
}

fn reg_string(name: PCWSTR) -> Option<String> {
    let mut buf = [0u16; 64];
    let mut size = std::mem::size_of_val(&buf) as u32;
    // SAFETY: at most `size` bytes are written into `buf`, NUL included.
    unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            CURRENT_VERSION,
            name,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    }
    .ok()
    .ok()?;
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// `-audiodev wasapi,id=audio0,out.dev=<endpoint id>`: the endpoint is
/// addressed by its `IMMDevice::GetId` string.
pub const AUDIO: Option<AudioBackend> = Some(AudioBackend {
    driver: "wasapi",
    options: "",
    device_selector: ",out.dev=",
});

/// `rng-random` is not built on Windows.
pub const RNG_OBJECT: &str = "rng-builtin,id=rng0";

/// The primary and shift modifiers as a shortcut hint spells them.
pub const KEY_PRIMARY: &str = "Ctrl ";
pub const KEY_SHIFT: &str = "Shift ";

/// Extension of a new virtual USB image: a VHDX, which Windows can attach and
/// format.
pub const VIRTUAL_IMAGE_EXT: &str = "vhdx";
