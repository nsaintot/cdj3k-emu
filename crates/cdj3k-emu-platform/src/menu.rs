//! Cross-platform native menu using `muda`.
//!
//! Call [`setup_menu`] once on the first frame (main thread), then call
//! [`sync_menu`] every frame to poll events and update checkmarks.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use crate::menu_state::{self};

/// How often the menu thread re-enumerates CoreAudio output devices.
/// muda doesn't expose "menu about to open" cross-platform; a 5 s tick is
/// cheap (HAL queries are local IPC to `coreaudiod`) and catches Bluetooth
/// device hot-plug fast enough to feel live.
const AUDIO_DEVICE_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

// ── Thread-local menu state ───────────────────────────────────────────────────

struct MenuState {
    // Root menu - must be kept alive: muda stores a raw *const MenuChild pointer in each
    // NSMenuItem's ObjC ivars; dropping Menu drops the Rc chain and frees that memory,
    // leaving dangling pointers that crash on the next menu click.
    _menu: Menu,

    // Emulation
    service_item: CheckMenuItem,
    haptic_item: CheckMenuItem,

    // Audio
    latency_item: MenuItem,
    audio_item: CheckMenuItem,
    alc_item: CheckMenuItem,

    // Audio output device submenu (rebuilt when version changes)
    audio_device_submenu: Submenu,
    audio_device_version_seen: u32,
    /// Last time we re-enumerated CoreAudio devices. The menu thread polls
    /// every [`AUDIO_DEVICE_REFRESH_INTERVAL`] to catch hot-plug.
    audio_device_last_refresh: Instant,

    // View
    jog_item: CheckMenuItem,
    main_item: CheckMenuItem,
    debug_item: CheckMenuItem,

    // Storage - eject_item is dynamically toggled; the others are ownership-
    // only to keep the muda Rc chain alive (click events still dispatch via
    // their registered ids).
    eject_item: MenuItem,
    _create_item: MenuItem,
    _mount_item: MenuItem,

    // Storage - physical submenu (rebuilt when version changes)
    phys_submenu: Submenu,
    phys_version_seen: u32,

    // Network submenu (rebuilt when version changes)
    net_submenu: Submenu,
    net_version_seen: u32,

    // Instances
    inst_items: Vec<CheckMenuItem>,
}

mod launch;
use launch::{launch_instance, show_fda_alert, show_net_error_alert};

thread_local! {
    static MENU_STATE: RefCell<Option<MenuState>> = RefCell::new(None);
}

// ── Setup ─────────────────────────────────────────────────────────────────────

/// Build the menu structure and install it. Call once on the first frame.
pub fn setup_menu() {
    let menu = Menu::new();

    // ── Emulation submenu (first position = app-name menu on macOS) ──────────
    let fw_item = MenuItem::with_id("install_firmware", "Install Firmware…", true, None);
    let restart_item = MenuItem::with_id("restart", "Restart Emulation", true, None);
    let service_item = CheckMenuItem::with_id("service_mode", "Service Mode", true, false, None);
    // Initial checked state is the runtime default (true); sync_menu() will
    // re-apply the persisted value once the UI has loaded InstanceSettings.
    let haptic_item = CheckMenuItem::with_id("haptic", "Jog Haptics", true, true, None);

    let emu_submenu = Submenu::with_items(
        "Emulation",
        true,
        &[
            &fw_item,
            &PredefinedMenuItem::separator(),
            &restart_item,
            &service_item,
            &haptic_item,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ],
    )
    .expect("build Emulation submenu");

    // ── Storage submenu ──────────────────────────────────────────────────────
    // Single Eject verb works for both virtual and physical mounts.
    let eject_item = MenuItem::with_id("eject_current_media", "Eject Current Media", false, None);
    let virt_header = MenuItem::new("Virtual (.img file)", false, None);
    let mount_item = MenuItem::with_id("mount_virtual_usb", "Mount Image…", true, None);
    let create_item = MenuItem::with_id("create_virtual_usb", "Create New…", true, None);
    let phys_header = MenuItem::new("Physical", false, None);
    let phys_submenu = Submenu::new("Devices", true);

    let storage_submenu = Submenu::with_items(
        "Storage",
        true,
        &[
            &eject_item,
            &PredefinedMenuItem::separator(),
            &virt_header,
            &mount_item,
            &create_item,
            &PredefinedMenuItem::separator(),
            &phys_header,
            &phys_submenu,
        ],
    )
    .expect("build Storage submenu");

    // ── Network submenu ──────────────────────────────────────────────────────
    // Populated immediately and rebuilt every time NET_LIST_VERSION changes.
    let net_submenu = Submenu::new("Network", true);
    menu_state::refresh_net_interfaces();
    build_net_submenu(&net_submenu);

    // ── Audio submenu ────────────────────────────────────────────────────────
    let latency_item = MenuItem::new("Current Latency: -- ms", false, None);
    let audio_item = CheckMenuItem::with_id("audio", "Enable Audio", true, true, None);
    let alc_item = CheckMenuItem::with_id("alc", "Enable ALC (Experimental)", true, false, None);

    // Output Device sub-submenu (populated immediately and rebuilt every time
    // `audio_device_list_version` changes).  Initially seeded synchronously
    // so the menu has data on the very first open.
    let audio_device_submenu = Submenu::new("Output Device", true);
    menu_state::refresh_audio_devices();
    build_audio_device_submenu(&audio_device_submenu);

    let audio_submenu = Submenu::with_items(
        "Audio",
        true,
        &[
            &latency_item,
            &PredefinedMenuItem::separator(),
            &audio_item,
            &alc_item,
            &PredefinedMenuItem::separator(),
            &audio_device_submenu,
        ],
    )
    .expect("build Audio submenu");

    // ── View submenu ─────────────────────────────────────────────────────────
    let jog_item = CheckMenuItem::with_id("jog_screen", "External Jog Screen", true, false, None);
    let main_item =
        CheckMenuItem::with_id("main_screen", "External Main Screen", true, false, None);
    let debug_item = CheckMenuItem::with_id("debug_screen", "Debug Panel", true, false, None);

    let view_submenu = Submenu::with_items("View", true, &[&jog_item, &main_item, &debug_item])
        .expect("build View submenu");

    // ── Instances submenu ────────────────────────────────────────────────────
    let inst_submenu = Submenu::new("Instances", true);
    let mut inst_items = Vec::with_capacity(menu_state::MAX_INSTANCES as usize);
    for n in 1..=menu_state::MAX_INSTANCES {
        let item = CheckMenuItem::with_id(
            format!("instance_{n}"),
            format!("Slot {n}"),
            true,
            false,
            None,
        );
        inst_submenu.append(&item).expect("append instance item");
        inst_items.push(item);
    }

    // ── Assemble root menu ───────────────────────────────────────────────────
    menu.append(&emu_submenu).expect("append Emulation");
    menu.append(&storage_submenu).expect("append Storage");
    menu.append(&net_submenu).expect("append Network");
    menu.append(&audio_submenu).expect("append Audio");
    menu.append(&view_submenu).expect("append View");
    menu.append(&inst_submenu).expect("append Instances");

    // ── Install ──────────────────────────────────────────────────────────────
    #[cfg(target_os = "macos")]
    menu.init_for_nsapp();

    // On non-macOS platforms the menu bar is shown on each window. Since we
    // run a single eframe window and muda requires a window handle, we skip
    // automatic installation on those platforms; the menu events are still
    // delivered via MenuEvent::receiver().

    let (initial_net_ver, initial_audio_dev_ver) = {
        let s = menu_state::lock();
        (s.net_list_version, s.audio_device_list_version)
    };

    MENU_STATE.with(|cell| {
        *cell.borrow_mut() = Some(MenuState {
            _menu: menu,
            service_item,
            haptic_item,
            latency_item,
            audio_item,
            alc_item,
            audio_device_submenu,
            audio_device_version_seen: initial_audio_dev_ver,
            audio_device_last_refresh: Instant::now(),
            jog_item,
            main_item,
            debug_item,
            eject_item,
            _create_item: create_item,
            _mount_item: mount_item,
            phys_submenu,
            phys_version_seen: u32::MAX,
            net_submenu,
            net_version_seen: initial_net_ver,
            inst_items,
        });
    });
}

// ── Sync (called every frame) ─────────────────────────────────────────────────

/// Poll menu events and sync checkmarks. Call every frame on the main thread.
pub fn sync_menu() {
    // ── FDA alert ────────────────────────────────────────────────────────────
    if std::mem::take(&mut menu_state::lock().usb_phys_perm_denied) {
        show_fda_alert();
    }

    // ── Network setup failure alert ──────────────────────────────────────────
    // Drop the lock guard before showing the dialog so other threads aren't
    // blocked while the modal is up.
    let net_err = menu_state::lock().net_error_message.take();
    if let Some(msg) = net_err {
        show_net_error_alert(&msg);
    }

    // ── Poll events ──────────────────────────────────────────────────────────
    let mut pending_create = false;
    let mut pending_mount = false;

    while let Ok(event) = MenuEvent::receiver().try_recv() {
        let id = event.id.0.as_str();
        handle_event(id, &mut pending_create, &mut pending_mount);
    }

    // ── File pickers (must run after event poll, same frame) ─────────────────
    if pending_create {
        if let Some(path) = rfd::FileDialog::new().set_file_name("usb.img").save_file() {
            let mut s = menu_state::lock();
            s.usb_virtual_img = Some(path);
            s.usb_create_req = true;
        }
    }
    if pending_mount {
        if let Some(path) = rfd::FileDialog::new().pick_file() {
            let mut s = menu_state::lock();
            s.usb_virtual_img = Some(path);
            s.usb_virtual_mount_req = true;
        }
    }

    // Snapshot every field the menu redraw needs in a single lock acquisition.
    let snap = {
        let s = menu_state::lock();
        MenuSnap {
            jog_screen_popped: s.jog_screen_popped,
            main_screen_popped: s.main_screen_popped,
            debug_screen_popped: s.debug_screen_popped,
            service_mode: s.service_mode,
            audio_enabled: s.audio_enabled,
            alc_enabled: s.alc_enabled,
            haptic_enabled: s.haptic_enabled,
            latency_packed: s.latency_packed,
            usb_virtual_mounted: s.usb_virtual_mounted,
            usb_phys_list_version: s.usb_phys_list_version,
            usb_phys_mounted_idx: s.usb_phys_mounted_idx,
            net_list_version: s.net_list_version,
            current_instance_id: s.current_instance_id,
        }
    };

    // ── Checkmark sync ───────────────────────────────────────────────────────
    MENU_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(state) = borrow.as_mut() else { return };

        // View toggles.
        state.jog_item.set_checked(snap.jog_screen_popped);
        state.main_item.set_checked(snap.main_screen_popped);
        state.debug_item.set_checked(snap.debug_screen_popped);

        // Emulation.
        state.service_item.set_checked(snap.service_mode);
        state.haptic_item.set_checked(snap.haptic_enabled);
        state.audio_item.set_checked(snap.audio_enabled);
        state.alc_item.set_checked(snap.alc_enabled);

        // Latency label - driven by the cfg daemon's 3 s push.
        let label = match menu_state::unpack_latency(snap.latency_packed) {
            Some((total, guest, host)) => {
                format!("Latency: {} ms (guest {}, host {})", total, guest, host)
            }
            None => "Current Latency: -- ms".to_string(),
        };
        state.latency_item.set_text(&label);

        // Storage - unified eject is enabled iff something is currently mounted
        // (either a virtual image or a physical disk).
        let anything_mounted = snap.usb_virtual_mounted || snap.usb_phys_mounted_idx >= 0;
        state.eject_item.set_enabled(anything_mounted);

        // Storage - physical disk list; rebuild when version changes.
        if state.phys_version_seen != snap.usb_phys_list_version {
            rebuild_phys_submenu(&state.phys_submenu);
            state.phys_version_seen = snap.usb_phys_list_version;
        }

        // Physical checkmarks (labels are set on rebuild).
        let items = state.phys_submenu.items();
        for (i, item_kind) in items.iter().enumerate() {
            if let Some(check) = item_kind.as_check_menuitem() {
                check.set_checked(snap.usb_phys_mounted_idx == i as i32);
            }
        }

        // Network - rebuild if list changed.
        if state.net_version_seen != snap.net_list_version {
            menu_state::refresh_net_interfaces();
            clear_submenu(&state.net_submenu);
            build_net_submenu(&state.net_submenu);
            state.net_version_seen = snap.net_list_version;
        }

        // Network radio checkmarks.
        sync_net_checkmarks(&state.net_submenu);

        // Audio devices - periodic re-enumeration (catches hot-plug); then
        // rebuild the radio list iff the version actually changed.
        if state.audio_device_last_refresh.elapsed() >= AUDIO_DEVICE_REFRESH_INTERVAL {
            menu_state::refresh_audio_devices();
            state.audio_device_last_refresh = Instant::now();
        }
        let live_audio_dev_ver = menu_state::lock().audio_device_list_version;
        if state.audio_device_version_seen != live_audio_dev_ver {
            clear_submenu(&state.audio_device_submenu);
            build_audio_device_submenu(&state.audio_device_submenu);
            state.audio_device_version_seen = live_audio_dev_ver;
        }
        sync_audio_device_checkmarks(&state.audio_device_submenu);

        // Instances.
        let cur = snap.current_instance_id;
        for (i, item) in state.inst_items.iter().enumerate() {
            let n = (i as u32) + 1;
            let sock_dir = crate::runtime_paths::instance_dir(n);
            let running = sock_dir.exists();
            let is_self = n == cur;
            item.set_checked(is_self || running);
            item.set_enabled(!is_self);
            let label = if is_self {
                format!("Slot {n} (this window)")
            } else if running {
                format!("Slot {n} — running")
            } else {
                format!("Slot {n}")
            };
            item.set_text(&label);
        }
    });
}

// ── Frame snapshot ────────────────────────────────────────────────────────────

/// Single-acquisition snapshot of every state field touched during one menu
/// sync. Avoids 13 separate `lock()` calls per frame.
struct MenuSnap {
    jog_screen_popped: bool,
    main_screen_popped: bool,
    debug_screen_popped: bool,
    service_mode: bool,
    audio_enabled: bool,
    alc_enabled: bool,
    haptic_enabled: bool,
    latency_packed: u64,
    usb_virtual_mounted: bool,
    usb_phys_list_version: u32,
    usb_phys_mounted_idx: i32,
    net_list_version: u32,
    current_instance_id: u32,
}

// ── Event dispatch ────────────────────────────────────────────────────────────

fn handle_event(id: &str, pending_create: &mut bool, pending_mount: &mut bool) {
    let mut s = menu_state::lock();
    match id {
        "install_firmware" => {
            s.firmware_wizard_requested = true;
        }
        "restart" => {
            s.shade_forced = true;
            s.restart_requested = true;
        }
        "service_mode" => {
            s.service_mode = !s.service_mode;
            s.shade_forced = true;
            s.restart_requested = true;
        }
        "audio" => {
            s.audio_enabled = !s.audio_enabled;
            s.shade_forced = true;
            s.audio_toggle_requested = true;
        }
        "alc" => {
            s.alc_enabled = !s.alc_enabled;
            // Runtime worker consumes this and pushes
            // `set audio_sync_enabled <0|1>` to the guest cfg daemon.
            s.alc_toggle_requested = true;
        }
        "haptic" => {
            // Pure host-side toggle.  Set the flag here (the jog physics layer
            // reads it at each detent-crossing) and arm `haptic_toggle_requested`
            // so the runtime worker persists the new value to InstanceSettings.
            // No QEMU restart, no guest push.
            s.haptic_enabled = !s.haptic_enabled;
            s.haptic_toggle_requested = true;
        }
        "jog_screen" => s.jog_screen_popped = !s.jog_screen_popped,
        "main_screen" => s.main_screen_popped = !s.main_screen_popped,
        "debug_screen" => s.debug_screen_popped = !s.debug_screen_popped,
        "net_none" => {
            if s.selected_interface != menu_state::NET_SEL_NONE {
                s.shade_forced = true;
            }
            s.selected_interface = menu_state::NET_SEL_NONE;
        }
        "net_vmnet_host" => {
            if s.selected_interface != menu_state::NET_SEL_VMNET_HOST {
                s.shade_forced = true;
            }
            s.selected_interface = menu_state::NET_SEL_VMNET_HOST;
        }
        "create_virtual_usb" => {
            *pending_create = true;
        }
        "mount_virtual_usb" => {
            *pending_mount = true;
        }
        "eject_current_media" => {
            s.usb_eject_req = true;
        }
        "audio_dev_default" => {
            if s.audio_device_uid.is_some() {
                s.audio_device_uid = None;
                s.audio_device_toggle_requested = true;
                s.shade_forced = true;
            }
        }
        other => {
            if let Some(rest) = other.strip_prefix("audio_dev_uid_") {
                // Decode the hex-encoded UID so identifier-unsafe chars in
                // CoreAudio UIDs (':', spaces, etc.) survive the menu-id
                // round-trip.
                if let Some(uid) = decode_hex_uid(rest) {
                    if s.audio_device_uid.as_deref() != Some(uid.as_str()) {
                        s.audio_device_uid = Some(uid);
                        s.audio_device_toggle_requested = true;
                        s.shade_forced = true;
                    }
                }
            } else if let Some(rest) = other.strip_prefix("net_if_") {
                if let Ok(n) = rest.parse::<u32>() {
                    if s.selected_interface != n {
                        s.shade_forced = true;
                    }
                    s.selected_interface = n;
                }
            } else if let Some(rest) = other.strip_prefix("phys_select_") {
                if let Ok(n) = rest.parse::<i32>() {
                    // Click-to-switch only. Re-selecting the currently mounted
                    // disk is a no-op (the runtime_worker filters this), and
                    // ejecting is exclusively done via "Eject Current Media".
                    if s.usb_phys_mounted_idx != n {
                        s.usb_phys_toggle_idx = n;
                    }
                }
            } else if let Some(rest) = other.strip_prefix("instance_") {
                if let Ok(n) = rest.parse::<u32>() {
                    // launch_instance reads CURRENT_INSTANCE_ID via lock; drop guard first.
                    drop(s);
                    launch_instance(n);
                    return;
                }
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn build_net_submenu(submenu: &Submenu) {
    let none_item = CheckMenuItem::with_id("net_none", "Default (NAT)", true, true, None);
    let host_item =
        CheckMenuItem::with_id("net_vmnet_host", "Host-only (vmnet)", true, false, None);
    submenu.append(&none_item).ok();
    submenu.append(&host_item).ok();

    // Clone the iface list out under the lock so we don't hold it across muda calls.
    let ifaces: Vec<menu_state::NetIf> = menu_state::lock().net_ifaces.clone();

    if !ifaces.is_empty() {
        submenu.append(&PredefinedMenuItem::separator()).ok();
    }
    for (i, iface) in ifaces.iter().enumerate() {
        let item = CheckMenuItem::with_id(format!("net_if_{i}"), iface.label(), true, false, None);
        submenu.append(&item).ok();
    }
}

fn sync_net_checkmarks(submenu: &Submenu) {
    let sel_idx = menu_state::lock().selected_interface;
    for item_kind in &submenu.items() {
        let Some(check) = item_kind.as_check_menuitem() else {
            continue;
        };
        let id = check.id().0.as_str();
        let is_selected = if id == "net_none" {
            sel_idx == menu_state::NET_SEL_NONE
        } else if id == "net_vmnet_host" {
            sel_idx == menu_state::NET_SEL_VMNET_HOST
        } else if let Some(rest) = id.strip_prefix("net_if_") {
            rest.parse::<u32>().map(|n| n == sel_idx).unwrap_or(false)
        } else {
            false
        };
        check.set_checked(is_selected);
    }
}

fn clear_submenu(submenu: &Submenu) {
    // Remove all items from the submenu by removing index 0 repeatedly.
    while submenu.remove_at(0).is_some() {}
}

// ── Audio device submenu ─────────────────────────────────────────────────────

fn build_audio_device_submenu(submenu: &Submenu) {
    // "Default System Output" first - selecting it clears the override and
    // QEMU falls back to whatever the user has set in Sound Preferences.
    let default_item = CheckMenuItem::with_id(
        "audio_dev_default",
        "Default System Output",
        true,
        true,
        None,
    );
    submenu.append(&default_item).ok();

    // Clone the device list out under the lock so we don't hold it while
    // muda walks the NSMenu graph.
    let devs: Vec<menu_state::AudioOutDevice> = menu_state::lock().audio_devices.clone();
    if !devs.is_empty() {
        submenu.append(&PredefinedMenuItem::separator()).ok();
    }
    for dev in devs {
        // Suffix the device's current nominal sample rate so the user can
        // see at a glance whether it matches the guest's 96 kHz expectation
        // without opening Audio MIDI Setup. `(system default)` is also
        // appended for the device the default-output selector points to.
        let mut suffix = String::new();
        let rate_label = format_rate(dev.sample_rate_hz);
        if !rate_label.is_empty() {
            suffix.push_str(" (");
            suffix.push_str(&rate_label);
            suffix.push(')');
        }
        if dev.is_default {
            suffix.push_str(" (system default)");
        }
        let label = format!("{}{}", dev.name, suffix);
        let id = format!("audio_dev_uid_{}", encode_hex_uid(&dev.uid));
        let item = CheckMenuItem::with_id(id, label, true, false, None);
        submenu.append(&item).ok();
    }
}

/// "44.1 kHz", "48 kHz", "96 kHz", "192 kHz" - empty string when the device
/// has no usable nominal rate. Rounds non-integer kHz to one decimal so 44.1
/// renders correctly; collapses trailing ".0" so 48000 prints as "48 kHz".
fn format_rate(hz: u32) -> String {
    if hz == 0 {
        return String::new();
    }
    let k = hz as f32 / 1000.0;
    if (k - k.round()).abs() < 0.05 {
        format!("{} kHz", k.round() as u32)
    } else {
        format!("{:.1} kHz", k)
    }
}

fn sync_audio_device_checkmarks(submenu: &Submenu) {
    // Snapshot the selected UID and the live device list under one lock.
    let (selected, present_uids): (Option<String>, Vec<String>) = {
        let s = menu_state::lock();
        (
            s.audio_device_uid.clone(),
            s.audio_devices.iter().map(|d| d.uid.clone()).collect(),
        )
    };
    // If the persisted UID points at a device that is no longer present
    // (Bluetooth off, USB interface unplugged, etc.) treat the selection
    // as "system default" for display — otherwise the radio group would
    // show nothing checked at all. The persisted UID itself is left intact
    // so reconnecting the device restores the binding.
    let selection_present = selected
        .as_deref()
        .map(|uid| present_uids.iter().any(|u| u == uid))
        .unwrap_or(true);
    let default_checked = selected.is_none() || !selection_present;
    for item_kind in &submenu.items() {
        let Some(check) = item_kind.as_check_menuitem() else {
            continue;
        };
        let id = check.id().0.as_str();
        let is_selected = if id == "audio_dev_default" {
            default_checked
        } else if let Some(rest) = id.strip_prefix("audio_dev_uid_") {
            decode_hex_uid(rest)
                .map(|uid| selected.as_deref() == Some(uid.as_str()))
                .unwrap_or(false)
        } else {
            false
        };
        check.set_checked(is_selected);
    }
}

/// muda menu ids are arbitrary ASCII strings, but to be safe against colons,
/// spaces and the rare UID containing characters that AppKit might choke on
/// when matched through NSMenuItem identifier paths, we hex-encode the UID
/// for the round-trip and decode it on click.
fn encode_hex_uid(uid: &str) -> String {
    let mut out = String::with_capacity(uid.len() * 2);
    for b in uid.as_bytes() {
        use std::fmt::Write;
        let _ = write!(out, "{:02x}", b);
    }
    out
}

fn decode_hex_uid(s: &str) -> Option<String> {
    if s.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(s.len() / 2);
    for chunk in s.as_bytes().chunks(2) {
        let hi = (chunk[0] as char).to_digit(16)?;
        let lo = (chunk[1] as char).to_digit(16)?;
        bytes.push(((hi << 4) | lo) as u8);
    }
    String::from_utf8(bytes).ok()
}

fn rebuild_phys_submenu(submenu: &Submenu) {
    clear_submenu(submenu);

    // Snapshot under lock; muda calls happen outside.
    let disks: Vec<menu_state::PhysicalDisk> = menu_state::lock().usb_phys_disks.clone();
    if disks.is_empty() {
        let none_item = MenuItem::new("No removable disks", false, None);
        submenu.append(&none_item).ok();
        return;
    }
    for (i, disk) in disks.iter().enumerate() {
        // Radio-style: clicking selects/switches; eject is the top-level item.
        let item =
            CheckMenuItem::with_id(format!("phys_select_{i}"), &disk.label, true, false, None);
        submenu.append(&item).ok();
    }
}
