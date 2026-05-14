//! Cross-platform UI/runtime state shared between the native macOS NSMenu
//! implementation, the egui UI, and the runtime worker thread.
//!
//! Previously a soup of ~28 atomic globals + 3 ad-hoc `Mutex<...>`s. Folded
//! into a single `Mutex<AppState>` so:
//!
//!   * lock acquisition order is trivial (only one lock).
//!   * the menu can snapshot the whole state with one `lock()` instead of 20+
//!     individual atomic loads.
//!   * adding new fields no longer means picking a fresh atomic type.
//!
//! The lock is never held across I/O. Hot loops (menu sync, runtime poll)
//! snapshot what they need, drop the guard, then perform work. Audio thread
//! and DRM-stream threads do NOT touch this state.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, MutexGuard};

/// Maximum instance count (slots are 1..=MAX_INSTANCES).
pub const MAX_INSTANCES: u32 = 4;

/// Sentinel value for [`AppState::usb_phys_mounted_idx`] and
/// `usb_phys_toggle_idx` meaning "no physical disk".  `i32` because the
/// menu uses signed indices to make the "none" state representable
/// without an `Option`.
pub const NO_USB_MOUNTED: i32 = -1;

/// Latched on shutdown.  **Async-signal-safe** so the SIGTERM/SIGINT handler
/// can set it without taking the mutex (which would deadlock if a signal
/// arrives while another thread already holds `AppState`).  Every other
/// shutdown-related state lives in [`AppState`].
pub static APP_SHUTDOWN: AtomicBool = AtomicBool::new(false);

// ── Domain types ──────────────────────────────────────────────────────────────

/// A single discovered network interface.
#[derive(Clone, Debug, PartialEq)]
pub struct NetIf {
    /// e.g. "en0"
    pub name: String,
    /// IPv4 address, e.g. "192.168.1.42"
    pub addr: String,
    /// CIDR prefix length, e.g. 24
    pub prefix_len: u8,
}

impl NetIf {
    /// Menu label: "192.168.1.42/24 (en0)"
    pub fn label(&self) -> String {
        format!("{}/{} ({})", self.addr, self.prefix_len, self.name)
    }
}

/// A removable physical disk entry (mirrors cdj3k_emu_runtime::usb::PhysicalDisk).
#[derive(Clone, Debug, PartialEq)]
pub struct PhysicalDisk {
    pub bsd_name: String,
    pub label: String,
}

/// A CoreAudio output device exposed in the per-instance "Audio Output" picker.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioOutDevice {
    pub uid: String,
    pub name: String,
    pub is_default: bool,
    /// Current nominal sample rate in Hz, or 0 if unavailable.
    pub sample_rate_hz: u32,
}

// ── Aggregate state ───────────────────────────────────────────────────────────

/// All cross-thread UI / runtime state. Access via [`lock()`].
pub struct AppState {
    // ── Instance ────────────────────────────────────────────────────────────
    /// 1..=MAX_INSTANCES. Set once at startup from main.rs (`--instance N`).
    pub current_instance_id: u32,

    // ── View ────────────────────────────────────────────────────────────────
    pub jog_screen_popped: bool,
    pub main_screen_popped: bool,
    pub debug_screen_popped: bool,

    // ── Emulation lifecycle ─────────────────────────────────────────────────
    /// Fires once; consumer clears after acting.
    pub stop_requested: bool,
    /// Set to `true` while QEMU is running; updated by the runtime worker.
    pub qemu_running: bool,
    /// Fires once to open the firmware install wizard.
    pub firmware_wizard_requested: bool,
    /// Fires once after provisioning completes to (re)start QEMU.
    pub qemu_boot_requested: bool,
    /// Fires once to stop and immediately restart QEMU.
    pub restart_requested: bool,
    /// Set by menu actions that require a restart so the boot shade engages
    /// immediately. Cleared by the runtime worker once QEMU has respawned.
    pub shade_forced: bool,
    /// Set by the runtime on graceful shutdown; UI injects a `set_power(false)`
    /// MISO stimuli and clears.
    pub power_off_stimuli_requested: bool,
    pub service_mode: bool,

    // ── Audio toggles ───────────────────────────────────────────────────────
    /// Mirror of the per-instance `audio_enabled` setting. Toggled by the
    /// "Enable audio" menu item; `audio_toggle_requested` fires the runtime
    /// worker to persist + restart QEMU.
    pub audio_enabled: bool,
    pub audio_toggle_requested: bool,

    // ── Audio output device ─────────────────────────────────────────────────
    /// Selected CoreAudio device UID, or `None` for "system default output".
    /// Persisted in InstanceSettings. The runtime worker passes this through
    /// to QEMU's `-audiodev coreaudio` config on (re)spawn.
    pub audio_device_uid: Option<String>,
    /// Cached enumeration of host output devices; refreshed by the menu
    /// thread on a 5 s tick via [`refresh_audio_devices`].
    pub audio_devices: Vec<AudioOutDevice>,
    /// Bumped when [`audio_devices`] changes; menu compares vs a local copy
    /// to rebuild the radio list.
    pub audio_device_list_version: u32,
    /// One-shot: set when the user picks a different device. The runtime
    /// worker persists the new UID and restarts QEMU.
    pub audio_device_toggle_requested: bool,

    /// "Enable ALC (Experimental)" toggle. Mirrors the guest's
    /// `audio_sync_enabled` sysfs param.
    pub alc_enabled: bool,
    pub alc_toggle_requested: bool,

    /// "Trackpad Haptics" toggle. Gates the Force Touch detent clicks
    /// emitted as the jog wheel crosses detents.  No QEMU/guest side
    /// effect - read at the haptic-actuate call site only.
    pub haptic_enabled: bool,
    /// One-shot: set when the user toggles the menu item; the runtime worker
    /// consumes it and persists `haptic_enabled` to InstanceSettings.
    pub haptic_toggle_requested: bool,

    /// Latest audio pipeline depth, pushed by the guest cfg daemon every 3 s.
    /// Packed `(total << 32) | (guest << 16) | host`, all ms.  `u64::MAX` means
    /// "no data yet" and the menu shows `--` instead.
    pub latency_packed: u64,

    // ── Network ─────────────────────────────────────────────────────────────
    /// Index into [`net_ifaces`], or `u32::MAX` for "none".
    pub selected_interface: u32,
    /// Cached interface list; updated by [`refresh_net_interfaces`].
    pub net_ifaces: Vec<NetIf>,
    /// Bumped whenever [`net_ifaces`] changes; consumers compare against a
    /// local copy to rebuild.
    pub net_list_version: u32,
    /// Set by the runtime worker when network setup (vmnet / tap bridge)
    /// fails.  Consumed by the menu thread on next sync: shows an `rfd`
    /// error popup and clears.
    pub net_error_message: Option<String>,

    // ── Storage / virtual USB ───────────────────────────────────────────────
    /// Path of the user-selected virtual USB image.
    pub usb_virtual_img: Option<PathBuf>,
    /// True while the virtual USB image is mounted in the guest.
    pub usb_virtual_mounted: bool,
    /// Menu→worker one-shot requests.
    pub usb_virtual_mount_req: bool,
    pub usb_create_req: bool,
    /// Unified eject for both virtual and physical mounts.
    pub usb_eject_req: bool,

    // ── Storage / physical USB ──────────────────────────────────────────────
    pub usb_phys_disks: Vec<PhysicalDisk>,
    /// Index of the physical disk currently mounted in the guest, or
    /// [`NO_USB_MOUNTED`] when nothing is mounted.
    pub usb_phys_mounted_idx: i32,
    /// Disk index whose toggle was requested by the menu, or [`NO_USB_MOUNTED`].
    pub usb_phys_toggle_idx: i32,
    /// Bumped by the USB worker whenever the disk list changes.
    pub usb_phys_list_version: u32,
    /// Set by the runtime when attach_physical fails with PermissionDenied;
    /// consumed by the menu to show a one-shot alert.
    pub usb_phys_perm_denied: bool,
    /// Set by the alert "Retry" button; runtime worker re-fires the toggle.
    pub usb_phys_retry_req: bool,
}

impl AppState {
    pub const fn new() -> Self {
        Self {
            current_instance_id: 1,
            jog_screen_popped: false,
            main_screen_popped: false,
            debug_screen_popped: false,
            stop_requested: false,
            qemu_running: false,
            firmware_wizard_requested: false,
            qemu_boot_requested: false,
            restart_requested: false,
            shade_forced: false,
            power_off_stimuli_requested: false,
            service_mode: false,
            audio_enabled: false,
            audio_toggle_requested: false,
            audio_device_uid: None,
            audio_devices: Vec::new(),
            audio_device_list_version: 0,
            audio_device_toggle_requested: false,
            alc_enabled: false,
            alc_toggle_requested: false,
            haptic_enabled: true,
            haptic_toggle_requested: false,
            latency_packed: u64::MAX,
            selected_interface: u32::MAX,
            net_ifaces: Vec::new(),
            net_list_version: 0,
            net_error_message: None,
            usb_virtual_img: None,
            usb_virtual_mounted: false,
            usb_virtual_mount_req: false,
            usb_create_req: false,
            usb_eject_req: false,
            usb_phys_disks: Vec::new(),
            usb_phys_mounted_idx: NO_USB_MOUNTED,
            usb_phys_toggle_idx: NO_USB_MOUNTED,
            usb_phys_list_version: 0,
            usb_phys_perm_denied: false,
            usb_phys_retry_req: false,
        }
    }
}

static APP_STATE: Mutex<AppState> = Mutex::new(AppState::new());

/// Acquire the global state lock. Held only for the duration of struct field
/// accesses - never across I/O.
pub fn lock() -> MutexGuard<'static, AppState> {
    APP_STATE.lock().unwrap()
}

// ── Latency packing helpers ───────────────────────────────────────────────────

/// Pack a (total, guest, host) ms triple into the [`AppState::latency_packed`] encoding.
pub fn pack_latency(total_ms: u32, guest_ms: u32, host_ms: u32) -> u64 {
    ((total_ms as u64) << 32) | (((guest_ms as u64) & 0xFFFF) << 16) | ((host_ms as u64) & 0xFFFF)
}

/// Decode [`AppState::latency_packed`].  Returns `None` if no sample has been published yet.
pub fn unpack_latency(packed: u64) -> Option<(u32, u32, u32)> {
    if packed == u64::MAX {
        return None;
    }
    let total = (packed >> 32) as u32;
    let guest = ((packed >> 16) & 0xFFFF) as u32;
    let host = (packed & 0xFFFF) as u32;
    Some((total, guest, host))
}

// ── Network helpers ───────────────────────────────────────────────────────────

/// Returns true if the interface is something we can actually plug QEMU into.
/// Two valid backends, one allowlist prefix each:
///   en*  - real Ethernet / Wi-Fi; bridged via Apple's vmnet.framework.
///   tap* - user-mode TAP; bridged in userspace by `TapBridge::setup`.
/// Everything else (utun, awdl, gif, stf, ipsec, bridge, bond, vlan, lo, …)
/// is either Layer 3 or has no upstream link and can't be bridged.
fn is_bridgeable(name: &str) -> bool {
    name.starts_with("en") || name.starts_with("tap")
}

/// Enumerate all non-loopback IPv4 interfaces via `getifaddrs`.
pub fn enumerate_interfaces() -> Vec<NetIf> {
    let mut out = Vec::new();

    #[cfg(unix)]
    unsafe {
        let mut addrs: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut addrs) != 0 {
            return out;
        }

        let mut cur = addrs;
        while !cur.is_null() {
            let ifa = &*cur;
            cur = ifa.ifa_next;

            let sa = ifa.ifa_addr;
            if sa.is_null() {
                continue;
            }
            if (*sa).sa_family as i32 != libc::AF_INET {
                continue;
            }
            if (ifa.ifa_flags & libc::IFF_LOOPBACK as u32) != 0 {
                continue;
            }
            if (ifa.ifa_flags & libc::IFF_UP as u32) == 0 {
                continue;
            }

            let sin = &*(sa as *const libc::sockaddr_in);
            let ip = u32::from_be(sin.sin_addr.s_addr);
            let a = (ip >> 24) as u8;
            let b = (ip >> 16) as u8;
            let c = (ip >> 8) as u8;
            let d = ip as u8;

            let mask_sa = ifa.ifa_netmask;
            let prefix_len = if !mask_sa.is_null() && (*mask_sa).sa_family as i32 == libc::AF_INET {
                let mask_sin = &*(mask_sa as *const libc::sockaddr_in);
                u32::from_be(mask_sin.sin_addr.s_addr).count_ones() as u8
            } else {
                0
            };

            let name = std::ffi::CStr::from_ptr(ifa.ifa_name)
                .to_string_lossy()
                .into_owned();

            if !is_bridgeable(&name) {
                continue;
            }

            out.push(NetIf {
                name,
                addr: format!("{a}.{b}.{c}.{d}"),
                prefix_len,
            });
        }

        libc::freeifaddrs(addrs);
    }

    out
}

/// Re-enumerate interfaces and update the cached list.
/// Bumps `net_list_version` only when the list actually changed.
pub fn refresh_net_interfaces() {
    let fresh = enumerate_interfaces();
    let mut s = lock();
    let changed = s.net_ifaces.len() != fresh.len()
        || s.net_ifaces
            .iter()
            .zip(fresh.iter())
            .any(|(a, b)| a.name != b.name || a.addr != b.addr);
    if changed {
        s.net_ifaces = fresh;
        s.net_list_version = s.net_list_version.wrapping_add(1);
    }
}

// ── Audio device helpers ──────────────────────────────────────────────────────

/// Re-enumerate CoreAudio output devices and update the cached list.
/// Bumps `audio_device_list_version` only when the list actually changed.
/// No-op on non-macOS targets.
pub fn refresh_audio_devices() {
    #[cfg(target_os = "macos")]
    {
        let fresh: Vec<AudioOutDevice> = crate::audio_devices::enumerate_output_devices()
            .into_iter()
            .map(|d| AudioOutDevice {
                uid: d.uid,
                name: d.name,
                is_default: d.is_default,
                sample_rate_hz: d.sample_rate_hz,
            })
            .collect();
        let mut s = lock();
        let changed = s.audio_devices.len() != fresh.len()
            || s.audio_devices.iter().zip(fresh.iter()).any(|(a, b)| {
                a.uid != b.uid
                    || a.name != b.name
                    || a.is_default != b.is_default
                    || a.sample_rate_hz != b.sample_rate_hz
            });
        if changed {
            s.audio_devices = fresh;
            s.audio_device_list_version = s.audio_device_list_version.wrapping_add(1);
        }
    }
}
