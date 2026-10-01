//! Builds the menu model from shared state.
//!
//! This is the whole content side of the menu: what rows exist, what they say,
//! and which are ticked. It owns the polling cadences for the sources that do
//! not push (audio devices, slot notes) so no provider has to.

use std::time::{Duration, Instant};

use super::id::MenuId;
use super::model::{MenuIcon, MenuModel, MenuNode, NetKind, Notice, Predefined};
use crate::host::SoftwareEmulation;
use crate::menu_state;

/// How often to re-enumerate host audio output devices.
///
/// No backend gives us a portable "menu about to open" signal, so the list is
/// polled. Enumeration is a local query, and 5 s catches Bluetooth hot-plug
/// fast enough to feel live.
const AUDIO_DEVICE_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// How long a slot note is good for. Every other slot's window can rewrite
/// these files at any time, and the menu syncs every frame.
const SLOT_NOTE_TTL: Duration = Duration::from_secs(2);

/// Content source for the menu. One per window.
pub struct MenuService {
    audio_device_last_refresh: Instant,
    net_version_seen: u32,
    slot_notes: Vec<Option<String>>,
    slot_busy: Vec<bool>,
    slot_notes_read: Instant,
    /// Read once: neither the host nor the environment changes under a
    /// running window.
    software_emulation: Option<SoftwareEmulation>,
}

impl Default for MenuService {
    fn default() -> Self {
        Self::new()
    }
}

impl MenuService {
    pub fn new() -> Self {
        // Seed both lists so the very first model is populated rather than
        // empty-then-corrected on the second frame.
        menu_state::refresh_net_interfaces();
        menu_state::refresh_audio_devices();
        Self {
            audio_device_last_refresh: Instant::now(),
            net_version_seen: menu_state::lock().net_list_version,
            slot_notes: vec![None; menu_state::MAX_INSTANCES as usize],
            slot_busy: vec![false; menu_state::MAX_INSTANCES as usize],
            // Older than the TTL, so the first build reads them.
            slot_notes_read: Instant::now() - SLOT_NOTE_TTL,
            software_emulation: crate::host::software_emulation(),
        }
    }

    /// Refresh whatever is due, then describe the menu as it should now look.
    pub fn model(&mut self) -> MenuModel {
        self.refresh_due_sources();

        let snap = Snapshot::take();

        MenuModel {
            roots: vec![
                emulation(&snap, self.slot_notes[slot_index(&snap)].clone()),
                storage(&snap),
                network(&snap),
                audio(&snap),
                view(&snap),
                self.instances(&snap),
            ],
            notice: self.software_emulation.map(tcg_notice),
        }
    }

    fn refresh_due_sources(&mut self) {
        // Another thread bumps `net_list_version` to ask for a re-read.
        let live_net_version = menu_state::lock().net_list_version;
        if self.net_version_seen != live_net_version {
            menu_state::refresh_net_interfaces();
            self.net_version_seen = live_net_version;
        }

        if self.audio_device_last_refresh.elapsed() >= AUDIO_DEVICE_REFRESH_INTERVAL {
            menu_state::refresh_audio_devices();
            self.audio_device_last_refresh = Instant::now();
        }

        if self.slot_notes_read.elapsed() >= SLOT_NOTE_TTL {
            for (i, note) in self.slot_notes.iter_mut().enumerate() {
                *note = menu_state::slot_note(i as u32 + 1);
            }
            for (i, busy) in self.slot_busy.iter_mut().enumerate() {
                *busy = menu_state::slot_busy(i as u32 + 1);
            }
            self.slot_notes_read = Instant::now();
        }
    }

    fn instances(&self, snap: &Snapshot) -> MenuNode {
        let mut rows = Vec::with_capacity(menu_state::MAX_INSTANCES as usize);
        for i in 0..menu_state::MAX_INSTANCES {
            let n = i + 1;
            let is_self = n == snap.current_instance_id;
            let running = self.slot_busy[i as usize];

            let mut label = match self.slot_notes[i as usize].as_deref() {
                Some(note) => format!("Slot {n} — {note}"),
                None => format!("Slot {n}"),
            };
            if is_self {
                label.push_str(" (this window)");
            } else if running {
                label.push_str(" (running)");
            }

            rows.push(MenuNode::Check {
                id: MenuId::Instance(n),
                label,
                enabled: !is_self,
                checked: is_self || running,
                detail: None,
            });
        }
        MenuNode::submenu("Instances", rows).with_status(
            format!("Slot {}", snap.current_instance_id),
            MenuIcon::Instances,
        )
    }
}

/// The current slot as an index into the notes, clamped: a slot id is
/// one-based and a stale one must not panic the menu.
fn slot_index(snap: &Snapshot) -> usize {
    (snap.current_instance_id.max(1) as usize - 1).min(menu_state::MAX_INSTANCES as usize - 1)
}

/// The card behind the strip's TCG badge: why software emulation is in use,
/// and what it costs.
fn tcg_notice(why: SoftwareEmulation) -> Notice {
    let reason = match why {
        SoftwareEmulation::Forced => {
            "CDJ3K_EMU_TCG is set, so QEMU emulates the CPU in software even \
             though this host could accelerate it. Unset it to use the \
             hardware accelerator."
                .to_string()
        }
        SoftwareEmulation::ForeignArch(arch) => format!(
            "Host is {arch}, target is arm64 and virtualizes only same-arch guests. \
             All instructions are translated."
        ),
        SoftwareEmulation::NoKvm => "/dev/kvm is missing: the kernel has no KVM, or \
             virtualization is turned off in the firmware."
            .to_string(),
        SoftwareEmulation::KvmDenied => "/dev/kvm exists but this user cannot open it. Add \
             the user to the kvm group, then log in again."
            .to_string(),
        SoftwareEmulation::Unsupported => {
            "No hardware accelerator is wired up for this operating system.".to_string()
        }
    };
    Notice {
        title: "Software emulation (TCG)".into(),
        body: vec![
            reason,
            "Expect a slower/unstable emulation, audio pops and tempo drift."
                .into(),
        ],
    }
}

// ── Sections ──────────────────────────────────────────────────────────────────

/// First position, which is the app-name menu on macOS.
///
/// Choosing the deck a slot emulates and installing the firmware it runs are
/// steps of one window, so they are one entry.
fn emulation(snap: &Snapshot, deck: Option<String>) -> MenuNode {
    let mut rows = vec![
        MenuNode::item(MenuId::ManageEmulation, "Manage Emulation"),
        MenuNode::item_enabled(MenuId::Restart, "Restart Emulation", deck.is_some()),
        MenuNode::Separator,
        MenuNode::check(MenuId::ServiceMode, "Service Mode", snap.service_mode),
    ];
    // An actuator opened at startup: a Force Touch trackpad.
    if crate::haptic::available() {
        rows.push(MenuNode::check(MenuId::Haptic, "Jog Haptics", snap.haptic_enabled));
    }

    // The cable needs host-side MIDI and HID endpoints.
    if snap.pc_link_supported {
        // Models the rear-panel USB-B cable to a PC. The emulated cable starts
        // unplugged; toggling it on starts the in-guest bridge and brings up
        // the host-side MIDI and HID endpoints.
        rows.push(MenuNode::check(
            MenuId::PcLink,
            "PC Link (USB-B)",
            snap.pc_link_enabled,
        ));
    }
    rows.push(MenuNode::Separator);
    rows.push(MenuNode::Predefined(Predefined::Quit));

    // What this window *is*: the deck, and the firmware release on it.
    let (title, release) = match deck {
        Some(note) => match note.rsplit_once(' ') {
            Some((model, rel)) => (model.to_string(), Some(rel.to_string())),
            None => (note, None),
        },
        None => ("No firmware".to_string(), None),
    };
    let node = MenuNode::submenu("Emulation", rows).with_status(title, MenuIcon::Emulation);
    match release {
        Some(rel) => node.with_detail(rel),
        None => node,
    }
}

/// One Eject verb covers both virtual and physical mounts.
fn storage(snap: &Snapshot) -> MenuNode {
    let anything_mounted = snap.usb_virtual_mounted || snap.usb_phys_mounted_idx >= 0;

    let mut rows = vec![
        MenuNode::item_enabled(
            MenuId::EjectCurrentMedia,
            "Eject Current Media",
            anything_mounted,
        ),
        MenuNode::Section("Virtual (.img)".into()),
        MenuNode::item(MenuId::MountVirtualUsb, "Mount Image…"),
        MenuNode::item(MenuId::CreateVirtualUsb, "Create New…"),
        MenuNode::Section("Physical".into()),
    ];
    if snap.phys_disks.is_empty() {
        rows.push(MenuNode::Label("No removable disks".into()));
    }
    for (i, disk) in snap.phys_disks.iter().enumerate() {
        // Radio-style: clicking selects or switches. Eject is the top-level
        // verb. The device node is data, so it trails rather than joining the
        // label.
        rows.push(
            MenuNode::check(
                MenuId::PhysicalDisk(i),
                disk.label.clone(),
                snap.usb_phys_mounted_idx == i as i32,
            )
            .with_detail(disk.bsd_name.clone()),
        );
    }

    let mounted = anything_mounted;
    let status = if snap.usb_virtual_mounted {
        "image".to_string()
    } else if let Some(d) = snap
        .usb_phys_mounted_idx
        .try_into()
        .ok()
        .and_then(|i: usize| snap.phys_disks.get(i))
    {
        d.bsd_name.clone()
    } else {
        "none".to_string()
    };

    // Until the guest can take a medium, a mount would reach QEMU and never
    // the player.
    MenuNode::submenu("Storage", rows)
        .with_status(status, MenuIcon::Storage { mounted })
        .with_enabled(snap.guest_ready)
}

fn network(snap: &Snapshot) -> MenuNode {
    let ifaces = &snap.net_ifaces;
    let sel = snap.selected_interface;

    let mut rows = vec![MenuNode::check(
        MenuId::NetNat,
        "Default (NAT)",
        sel == menu_state::NET_SEL_NONE,
    )];
    if crate::net::HOST_ONLY {
        rows.push(MenuNode::check(
            MenuId::NetVmnetHost,
            "Host-only (link-local)",
            sel == menu_state::NET_SEL_VMNET_HOST,
        ));
    }
    if !ifaces.is_empty() {
        rows.push(MenuNode::Separator);
    }
    for (i, iface) in ifaces.iter().enumerate() {
        let i = i as u32;
        rows.push(MenuNode::check(
            MenuId::NetInterface(i),
            iface.label(),
            sel == i,
        ));
    }

    let (status, kind) = match sel {
        menu_state::NET_SEL_NONE => ("NAT".to_string(), NetKind::Nat),
        menu_state::NET_SEL_VMNET_HOST => ("link-local".to_string(), NetKind::LinkLocal),
        n => (
            ifaces
                .get(n as usize)
                .map(|i| i.label())
                .unwrap_or_else(|| "bridged".into()),
            NetKind::Bridged,
        ),
    };
    MenuNode::submenu("Network", rows).with_status(status, MenuIcon::Network(kind))
}

fn audio(snap: &Snapshot) -> MenuNode {
    // Driven by the cfg daemon's 3 s push.
    let (value, detail) = match menu_state::unpack_latency(snap.latency_packed) {
        Some((total, guest, host)) => (
            format!("{total} ms"),
            Some(format!("guest {guest} · host {host}")),
        ),
        None => ("-- ms".to_string(), None),
    };
    let status = value.clone();

    MenuNode::submenu(
        "Audio",
        vec![
            MenuNode::Readout {
                title: "Pipeline latency".into(),
                value,
                detail,
            },
            MenuNode::Separator,
            MenuNode::check(MenuId::Audio, "Enable Audio", snap.audio_enabled),
            MenuNode::check(MenuId::Alc, "Enable ALC (Experimental)", snap.alc_enabled),
            MenuNode::Separator,
            MenuNode::submenu("Output Device", audio_devices(snap)),
        ],
    )
    .with_status(
        status,
        MenuIcon::Audio {
            on: snap.audio_enabled,
        },
    )
}

fn audio_devices(snap: &Snapshot) -> Vec<MenuNode> {
    let selected = snap.audio_device_uid.as_deref();
    let devices = &snap.audio_devices;

    // A persisted UID can point at a device that is gone — Bluetooth off, USB
    // interface unplugged. Show the default as ticked rather than leaving the
    // radio group with nothing ticked at all. The persisted UID is left alone,
    // so reconnecting the device restores the binding.
    let selection_present = selected.is_none_or(|uid| devices.iter().any(|d| d.uid == uid));

    let mut rows = vec![MenuNode::check(
        MenuId::AudioDeviceDefault,
        "Default System Output",
        selected.is_none() || !selection_present,
    )];
    if !devices.is_empty() {
        rows.push(MenuNode::Separator);
    }
    for dev in devices {
        // Suffix the nominal sample rate so a mismatch against the guest's
        // 96 kHz is visible without opening the host's audio settings.
        let mut label = dev.name.clone();
        let rate = format_rate(dev.sample_rate_hz);
        if !rate.is_empty() {
            label.push_str(&format!(" ({rate})"));
        }
        if dev.is_default {
            label.push_str(" (system default)");
        }
        rows.push(MenuNode::check(
            MenuId::AudioDevice(dev.uid.clone()),
            label,
            selected == Some(dev.uid.as_str()),
        ));
    }
    rows
}

fn view(snap: &Snapshot) -> MenuNode {
    MenuNode::submenu(
        "View",
        vec![
            MenuNode::check(
                MenuId::ScreenExtended,
                "Extend Screen",
                snap.screen_extended,
            ),
            MenuNode::check(
                MenuId::JogScreen,
                "External Jog Screen",
                snap.jog_screen_popped,
            ),
            MenuNode::check(
                MenuId::MainScreen,
                "External Main Screen",
                snap.main_screen_popped,
            ),
            MenuNode::check(MenuId::DebugScreen, "Debug Panel", snap.debug_screen_popped),
        ],
    )
    .with_icon(MenuIcon::View)
}

/// "44.1 kHz", "48 kHz", "96 kHz" — empty when the device reports no usable
/// nominal rate. Rounds to one decimal so 44.1 renders, and collapses a
/// trailing ".0" so 48000 prints as "48 kHz".
fn format_rate(hz: u32) -> String {
    if hz == 0 {
        return String::new();
    }
    let k = hz as f32 / 1000.0;
    if (k - k.round()).abs() < 0.05 {
        format!("{} kHz", k.round() as u32)
    } else {
        format!("{k:.1} kHz")
    }
}

// ── State snapshot ────────────────────────────────────────────────────────────

/// Everything the model needs, read under one lock acquisition.
///
/// The three lists are cloned rather than borrowed: the lock must not be held
/// while the model is built, because a provider may be reached from the same
/// thread and `menu_state::lock()` is not reentrant.
struct Snapshot {
    net_ifaces: Vec<menu_state::NetIf>,
    phys_disks: Vec<menu_state::PhysicalDisk>,
    audio_devices: Vec<crate::audio::AudioOutDevice>,
    audio_device_uid: Option<String>,
    screen_extended: bool,
    jog_screen_popped: bool,
    main_screen_popped: bool,
    debug_screen_popped: bool,
    service_mode: bool,
    audio_enabled: bool,
    alc_enabled: bool,
    haptic_enabled: bool,
    pc_link_supported: bool,
    pc_link_enabled: bool,
    latency_packed: u64,
    usb_virtual_mounted: bool,
    usb_phys_mounted_idx: i32,
    guest_ready: bool,
    selected_interface: u32,
    current_instance_id: u32,
}

impl Snapshot {
    fn take() -> Self {
        let s = menu_state::lock();
        Self {
            net_ifaces: s.net_ifaces.clone(),
            phys_disks: s.usb_phys_disks.clone(),
            audio_devices: s.audio_devices.clone(),
            audio_device_uid: s.audio_device_uid.clone(),
            screen_extended: s.screen_extended,
            jog_screen_popped: s.jog_screen_popped,
            main_screen_popped: s.main_screen_popped,
            debug_screen_popped: s.debug_screen_popped,
            service_mode: s.service_mode,
            audio_enabled: s.audio_enabled,
            alc_enabled: s.alc_enabled,
            haptic_enabled: s.haptic_enabled,
            pc_link_supported: s.pc_link_supported,
            pc_link_enabled: s.pc_link_enabled,
            latency_packed: s.latency_packed,
            usb_virtual_mounted: s.usb_virtual_mounted,
            usb_phys_mounted_idx: s.usb_phys_mounted_idx,
            guest_ready: s.guest_cfg_live && s.guest_booted,
            selected_interface: s.selected_interface,
            current_instance_id: s.current_instance_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_render_the_way_a_device_sheet_reads() {
        assert_eq!(format_rate(0), "");
        assert_eq!(format_rate(48_000), "48 kHz");
        assert_eq!(format_rate(96_000), "96 kHz");
        assert_eq!(format_rate(192_000), "192 kHz");
        assert_eq!(format_rate(44_100), "44.1 kHz");
    }
}
