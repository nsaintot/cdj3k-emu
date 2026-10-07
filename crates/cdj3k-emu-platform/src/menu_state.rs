//! UI and runtime state shared between the menu, the egui UI and the runtime
//! worker thread, behind one `Mutex<AppState>`: one lock to order, and one
//! `lock()` to snapshot everything.
//!
//! The lock is never held across I/O. Hot loops (menu sync, runtime poll)
//! snapshot what they need, drop the guard, then perform work. Audio thread
//! and DRM-stream threads do NOT touch this state.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, MutexGuard};

use crate::audio::AudioOutDevice;

/// Maximum instance count (slots are 1..=MAX_INSTANCES).
pub const MAX_INSTANCES: u32 = 4;

/// Sentinel value for [`AppState::usb_phys_mounted_idx`] and
/// `usb_phys_toggle_idx` meaning "no physical disk".  `i32` because the
/// menu uses signed indices to make the "none" state representable
/// without an `Option`.
pub const NO_USB_MOUNTED: i32 = -1;

/// Sentinel value for [`AppState::selected_interface`] meaning "no network
/// interface picked" (QEMU runs with user-mode NAT, the menu's "None (NAT)"
/// entry).
pub const NET_SEL_NONE: u32 = u32::MAX;

/// Sentinel value for [`AppState::selected_interface`] meaning "vmnet host-only
/// network" (menu's "Host-only (link-local)" entry). All instances selecting this
/// open a `vmnet-host` interface under one shared network UUID, giving a
/// host-side `bridgeN` interface that vmnet.framework creates. Pure L2,
/// host-sniffable in Wireshark with no encapsulation. The daemon runs with
/// `--vmnet-network-identifier`, so the segment has no DHCP server: guests
/// self-assign 169.254/16 link-local via avahi-autoipd, like real gear.
pub const NET_SEL_VMNET_HOST: u32 = u32::MAX - 1;

/// Token written to `InstanceSettings::net_iface` to persist the vmnet-host
/// mode across launches. Not a valid BSD ifname, so it can never collide with
/// a real interface returned by `getifaddrs`.
pub const NET_IFACE_VMNET_HOST_TOKEN: &str = "__vmnet_host__";

/// Latched on shutdown.  **Async-signal-safe** so the SIGTERM/SIGINT handler
/// can set it without taking the mutex (which would deadlock if a signal
/// arrives while another thread already holds `AppState`).  Every other
/// shutdown-related state lives in [`AppState`].
pub static APP_SHUTDOWN: AtomicBool = AtomicBool::new(false);

// ── Domain types ──────────────────────────────────────────────────────────────

/// A removable physical disk entry (mirrors cdj3k_emu_runtime::disk::PhysicalDisk).
#[derive(Clone, Debug, PartialEq)]
pub struct PhysicalDisk {
    pub bsd_name: String,
    pub label: String,
}

// ── Aggregate state ───────────────────────────────────────────────────────────

/// All cross-thread UI / runtime state. Access via [`lock()`].
pub struct AppState {
    // ── Instance ────────────────────────────────────────────────────────────
    /// 1..=MAX_INSTANCES. Set once at startup from main.rs (`--instance N`).
    pub current_instance_id: u32,
    /// `--no-spawn`: the window runs without any runtime worker or QEMU
    /// (chassis/layout work). The boot shade stays down and nothing waits
    /// for a guest.
    pub ui_only: bool,
    /// The model of the panel on screen; `None` while the picker is up.
    pub model: Option<cdj3k_emu_panel::Model>,

    // ── View ────────────────────────────────────────────────────────────────
    pub jog_screen_popped: bool,
    pub main_screen_popped: bool,
    pub debug_screen_popped: bool,

    // ── Emulation lifecycle ─────────────────────────────────────────────────
    /// Fires once; consumer clears after acting.
    pub stop_requested: bool,
    /// Set to `true` while QEMU is running; updated by the runtime worker.
    pub qemu_running: bool,
    /// Fires once after provisioning completes to (re)start QEMU.
    pub qemu_boot_requested: bool,
    /// Fires once to stop and immediately restart QEMU.
    pub restart_requested: bool,
    /// Fires once to open the setup window - the one place a slot's emulation
    /// is chosen and its firmware installed.
    pub manage_emulation_requested: bool,
    /// Fires once to open the update window: a check for a newer release, or
    /// the restart an installed one is waiting for.
    pub update_check_requested: bool,
    /// An update is installed beside this build and waits for a restart; the
    /// menu offers that instead of a check.
    pub update_ready: bool,
    /// Fires once to retire the runtime worker (stop QEMU, exit its loop)
    /// without shutting the app down; a later launch spawns a fresh worker.
    pub worker_exit_requested: bool,
    /// Set by a worker that retired itself instead of restarting QEMU, a
    /// finished install waiting for the slot: the shell launches again,
    /// which swaps the install in.
    pub relaunch_requested: bool,
    /// Set by menu actions that require a restart so the boot shade engages
    /// immediately. Cleared by the runtime worker once QEMU has respawned.
    pub shade_forced: bool,
    /// Set by the runtime on graceful shutdown; UI injects a `set_power(false)`
    /// MISO stimuli and clears.
    pub power_off_stimuli_requested: bool,
    /// Draw the LCD larger than the deck's own, over the decoration around
    /// it. The panel reproduces the real form factor, which leaves the
    /// touchscreen small to read on a desktop display.
    pub screen_extended: bool,
    pub service_mode: bool,

    // ── Audio toggles ───────────────────────────────────────────────────────
    /// Mirror of the per-instance `audio_enabled` setting. Toggled by the
    /// "Enable audio" menu item; `audio_toggle_requested` fires the runtime
    /// worker to persist + restart QEMU.
    pub audio_enabled: bool,
    pub audio_toggle_requested: bool,

    // ── Audio output device ─────────────────────────────────────────────────
    /// Selected output device ([`AudioOutDevice::uid`]), or `None` for
    /// "system default output". Persisted in InstanceSettings. The runtime
    /// worker passes it to QEMU's `-audiodev` config on (re)spawn.
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

    /// "Enable Mods": whether the slot boots with its mods. Toggling it
    /// restarts a running emulation; the list stays editable either way.
    pub mods_enabled: bool,
    /// One-shot: the runtime worker persists `mods_enabled`.
    pub mods_toggle_requested: bool,

    /// Whether this build can publish PC Link endpoints. Set once at startup.
    pub pc_link_supported: bool,
    /// "PC Link (USB-B cable)" toggle.
    /// Models the rear-panel USB-B cable being plugged into the PC.
    pub pc_link_enabled: bool,
    /// One-shot: set when the user toggles the menu item; the runtime
    /// worker consumes it, persists, and applies start/stop.
    pub pc_link_toggle_requested: bool,

    /// Latest audio pipeline depth, pushed by the guest cfg daemon every 3 s.
    /// Packed `(total << 32) | (guest << 16) | host`, all ms.  `u64::MAX` means
    /// "no data yet" and the menu shows `--` instead.
    pub latency_packed: u64,

    // ── Network ─────────────────────────────────────────────────────────────
    /// Index into [`net_ifaces`], or one of [`NET_SEL_NONE`] / [`NET_SEL_MCAST`].
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
    /// Set by the runtime: the guest's cfgd has spoken on this boot, so a
    /// mount request reaches it.
    pub guest_cfg_live: bool,
    /// Set by the UI: the boot shade has lifted, so the player app is up to
    /// see a medium arrive.
    pub guest_booted: bool,
    pub usb_phys_disks: Vec<PhysicalDisk>,
    /// Index of the physical disk currently mounted in the guest, or
    /// [`NO_USB_MOUNTED`] when nothing is mounted.
    pub usb_phys_mounted_idx: i32,
    /// Disk index whose toggle was requested by the menu, or [`NO_USB_MOUNTED`].
    pub usb_phys_toggle_idx: i32,
    /// Bumped by the USB worker whenever the disk list changes.
    pub usb_phys_list_version: u32,
    /// Set by the runtime when attach_physical fails with PermissionDenied:
    /// the host's retry prompt, for a one-shot alert.
    pub usb_phys_perm_denied: Option<&'static str>,
    /// Set by the alert "Retry" button; runtime worker re-fires the toggle.
    pub usb_phys_retry_req: bool,
    /// Why the last physical attach failed, for a one-shot alert.
    pub usb_error_message: Option<String>,
    /// Set after the CoreMIDI driver plugin is replaced while `MIDIServer` is
    /// running: it keeps serving the old binary until the process exits.
    pub midi_driver_replaced: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub const fn new() -> Self {
        Self {
            current_instance_id: 1,
            ui_only: false,
            model: None,
            jog_screen_popped: false,
            main_screen_popped: false,
            debug_screen_popped: false,
            stop_requested: false,
            qemu_running: false,
            qemu_boot_requested: false,
            restart_requested: false,
            manage_emulation_requested: false,
            update_check_requested: false,
            update_ready: false,
            worker_exit_requested: false,
            relaunch_requested: false,
            shade_forced: false,
            power_off_stimuli_requested: false,
            screen_extended: false,
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
            mods_enabled: true,
            mods_toggle_requested: false,
            pc_link_supported: false,
            pc_link_enabled: false,
            pc_link_toggle_requested: false,
            latency_packed: u64::MAX,
            selected_interface: NET_SEL_NONE,
            net_ifaces: Vec::new(),
            net_list_version: 0,
            net_error_message: None,
            usb_virtual_img: None,
            usb_virtual_mounted: false,
            usb_virtual_mount_req: false,
            usb_create_req: false,
            usb_eject_req: false,
            guest_cfg_live: false,
            guest_booted: false,
            usb_phys_disks: Vec::new(),
            usb_error_message: None,
            usb_phys_mounted_idx: NO_USB_MOUNTED,
            usb_phys_toggle_idx: NO_USB_MOUNTED,
            usb_phys_list_version: 0,
            usb_phys_perm_denied: None,
            usb_phys_retry_req: false,
            midi_driver_replaced: false,
        }
    }
}

/// How the Instances menu names a slot: the deck installed in it and the
/// firmware release on it, e.g. `"CDJ-3000 3.20"`.
///
/// The slot settings live in `cdj3k-emu-storage`, which depends on this
/// crate, so the app registers a reader here rather than the menu reaching
/// for the files itself.
pub type SlotNoteFn = fn(u32) -> Option<String>;

static SLOT_NOTE_FN: Mutex<Option<SlotNoteFn>> = Mutex::new(None);

/// Register the reader the Instances menu asks for slot notes. Called once at
/// startup; the menu shows bare slot numbers until it is.
pub fn set_slot_note_fn(f: SlotNoteFn) {
    *SLOT_NOTE_FN.lock().unwrap() = Some(f);
}

/// Whether another process has slot `n` open, by the same locks the setup
/// window reads; registered by the app for the same reason as [`SlotNoteFn`].
pub type SlotBusyFn = fn(u32) -> bool;

static SLOT_BUSY_FN: Mutex<Option<SlotBusyFn>> = Mutex::new(None);

pub fn set_slot_busy_fn(f: SlotBusyFn) {
    *SLOT_BUSY_FN.lock().unwrap() = Some(f);
}

/// Whether slot `n` is open in another process; `false` until
/// [`set_slot_busy_fn`] has run.
pub fn slot_busy(n: u32) -> bool {
    let f = *SLOT_BUSY_FN.lock().unwrap();
    f.is_some_and(|busy| busy(n))
}

/// What slot `n` holds, or `None` for an empty slot - and for every slot
/// until [`set_slot_note_fn`] has run.
pub fn slot_note(n: u32) -> Option<String> {
    let f = *SLOT_NOTE_FN.lock().unwrap();
    f.and_then(|read| read(n))
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

/// The host decides which interfaces can carry a guest; see
/// [`crate::net`]. Re-exported here because this module is where the menu
/// reads the list from.
pub use crate::net::NetIf;

// ── Interface list refresh ────────────────────────────────────────────────────

/// Re-enumerate interfaces and update the cached list.
/// Bumps `net_list_version` only when the list actually changed.
pub fn refresh_net_interfaces() {
    let fresh = crate::net::enumerate_interfaces();
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

/// Re-enumerate the host's audio outputs and update the cached list.
///
/// Bumps `audio_device_list_version` only when the list actually changed, so
/// the menu rebuilds its radio group only when it has to. A host with no
/// enumeration leaves the list empty, and the guest follows the system
/// default.
pub fn refresh_audio_devices() {
    store_devices(crate::audio::enumerate_output_devices());
}

/// Replace the cached device list, bumping the version only on a real change.
///
/// The version drives a menu rebuild, and this is polled every few seconds —
/// so an unconditional bump would rebuild the radio group forever.
fn store_devices(fresh: Vec<AudioOutDevice>) {
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
