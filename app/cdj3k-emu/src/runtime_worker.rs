//! Background thread that polls `menu_state` and drives QemuInstance / UsbManager.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use cdj3k_emu_platform::menu_state::{APP_SHUTDOWN, NO_USB_MOUNTED};

use cdj3k_emu_platform::menu_state;
use cdj3k_emu_runtime::{
    register_worker_thread, CfgClient, DiskProvider, MacOsDiskProvider, QemuConfig, QemuInstance,
    SocketVmnet, TapBridge, UsbManager,
};

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const DISK_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const NET_REFRESH_INTERVAL: Duration = Duration::from_secs(3);

/// Pre-built network backends, set up before the very first QEMU spawn so
/// the initial process already has the right `-netdev`. The worker holds
/// them for its lifetime - Drop tears down the bridge / vmnet daemon.
pub struct PrebuiltNet {
    pub tap_bridge: Option<TapBridge>,
    pub vmnet: Option<SocketVmnet>,
    /// Initial value to seed `prev_net_idx` with so the worker doesn't
    /// trigger a restart on the first poll iteration.
    pub initial_net_idx: u32,
}

pub fn spawn(instance: Option<QemuInstance>, config: QemuConfig, prebuilt_net: PrebuiltNet) {
    let handle = std::thread::Builder::new()
        .name("cdj3k-emu-runtime-worker".into())
        .spawn(move || run(instance, config, prebuilt_net))
        .expect("failed to spawn runtime worker");
    register_worker_thread(handle);
}

fn run(mut instance: Option<QemuInstance>, mut config: QemuConfig, prebuilt_net: PrebuiltNet) {
    menu_state::lock().qemu_running = instance.is_some();
    let provider = MacOsDiskProvider;
    let cfg_client = CfgClient::new(&config.sock_dir());
    let mut usb = UsbManager::new(config.usb_placeholder_path(), cfg_client.clone());
    let mut phys_disks: Vec<cdj3k_emu_runtime::PhysicalDisk> = Vec::new();
    let mut last_disk_refresh = Instant::now() - DISK_REFRESH_INTERVAL;

    // Network backends: own them here so Drop fires on app exit (worker breaks
    // out of the loop, function returns, locals drop) and on iface change
    // (`= None;` drops the previous Some).  TapBridge::Drop tears down the
    // host TAP; SocketVmnet::Drop unlinks the socket file, which is the
    // signal to the root-side watchdog (spawned alongside socket_vmnet) to
    // reap the daemon - the user-level process can't kill it directly.
    #[allow(unused_assignments)]
    let mut _active_tap_bridge: Option<TapBridge> = prebuilt_net.tap_bridge;
    #[allow(unused_assignments)]
    let mut _active_vmnet: Option<SocketVmnet> = prebuilt_net.vmnet;
    let mut prev_net_idx: u32 = prebuilt_net.initial_net_idx;
    let mut last_phys_toggle: i32 = -1;
    let mut last_net_refresh = Instant::now() - NET_REFRESH_INTERVAL;
    // Latched per QEMU lifetime - cleared on restart so a fresh kernel module
    // load gets the user's persisted ALC value pushed once cfg.sock is up.
    let mut alc_pushed_for_boot = false;
    // Storage to re-mount after the next QEMU boot (deferred until cfg.sock is ready).
    let mut pending_remount: Option<PendingRemount> = None;
    // Populate the interface cache immediately so the menu has data on first open.
    menu_state::refresh_net_interfaces();

    // ── Restore persisted per-instance settings (best effort) ────────────────
    // Virtual USB: main.rs seeded usb_virtual_img if the file existed; queue a
    // deferred mount that runs once QEMU is up.
    if let Some(path) = menu_state::lock().usb_virtual_img.clone() {
        if path.exists() {
            pending_remount = Some(PendingRemount::Virtual { path });
        }
    }
    let saved_phys_bsd =
        cdj3k_emu_storage::InstanceSettings::load_or_init(config.instance_id).usb_physical_bsd;
    let mut phys_restore_attempted = saved_phys_bsd.is_none();

    loop {
        // ── App-exit gate: graceful shutdown ─────────────────────────────────
        if APP_SHUTDOWN.load(Ordering::Relaxed) {
            if let Some(mut inst) = instance.take() {
                inst.stop();
            }
            menu_state::lock().qemu_running = false;
            break;
        }

        // ── Auto-restart after guest-initiated shutdown ───────────────────────
        let guest_exited = instance.as_ref().map(|i| !i.is_running()).unwrap_or(false);
        if guest_exited {
            if let Some(ref mut inst) = instance {
                inst.stop();
            }
            instance = None;
            menu_state::lock().qemu_running = false;

            if APP_SHUTDOWN.load(Ordering::Relaxed) {
                break;
            }

            eprintln!("cdj3k-emu: QEMU exited - restarting");
            let (prev_phys, prev_virt) = {
                let s = menu_state::lock();
                (s.usb_phys_mounted_idx, s.usb_virtual_mounted)
            };
            reset_usb(&mut usb, &config, &cfg_client);
            pending_remount = PendingRemount::capture(prev_phys, prev_virt, &phys_disks);
            apply_menu_to_config(&mut config);
            #[cfg(target_os = "macos")]
            match QemuInstance::spawn(config.clone()) {
                Ok(new_inst) => {
                    instance = Some(new_inst);
                    alc_pushed_for_boot = false;
                    let mut s = menu_state::lock();
                    s.qemu_running = true;
                    s.shade_forced = false;
                }
                Err(e) => eprintln!("cdj3k-emu: auto-restart failed: {e:?}"),
            }
        }

        // ── Per-iteration request dispatch ───────────────────────────────────
        // Drain every one-shot flag in a single lock acquisition.
        let req = {
            let mut s = menu_state::lock();
            Requests {
                boot: std::mem::take(&mut s.qemu_boot_requested) && instance.is_none(),
                stop: std::mem::take(&mut s.stop_requested),
                audio_toggle: std::mem::take(&mut s.audio_toggle_requested),
                audio_device_toggle: std::mem::take(&mut s.audio_device_toggle_requested),
                alc_toggle: std::mem::take(&mut s.alc_toggle_requested),
                restart: std::mem::take(&mut s.restart_requested),
                usb_create: std::mem::take(&mut s.usb_create_req),
                usb_virt_mount: std::mem::take(&mut s.usb_virtual_mount_req),
                usb_eject: std::mem::take(&mut s.usb_eject_req),
                usb_phys_toggle: std::mem::replace(&mut s.usb_phys_toggle_idx, NO_USB_MOUNTED),
                usb_phys_retry: std::mem::take(&mut s.usb_phys_retry_req),
                audio_enabled: s.audio_enabled,
                audio_device_uid: s.audio_device_uid.clone(),
                alc_enabled: s.alc_enabled,
                haptic_toggle: std::mem::take(&mut s.haptic_toggle_requested),
                haptic_enabled: s.haptic_enabled,
                selected_iface: s.selected_interface,
                usb_virtual_img: s.usb_virtual_img.clone(),
            }
        };

        // ── Post-provisioning boot ────────────────────────────────────────────
        if req.boot {
            apply_menu_to_config(&mut config);
            #[cfg(target_os = "macos")]
            match QemuInstance::spawn(config.clone()) {
                Ok(inst) => {
                    eprintln!("cdj3k-emu: QEMU started after provisioning");
                    instance = Some(inst);
                    alc_pushed_for_boot = false;
                    let mut s = menu_state::lock();
                    s.qemu_running = true;
                    s.shade_forced = false;
                }
                Err(e) => eprintln!("cdj3k-emu: post-provision QEMU start failed: {e:?}"),
            }
        }

        // ── Stop ─────────────────────────────────────────────────────────────
        if req.stop {
            if let Some(ref mut inst) = instance {
                inst.stop();
            }
            instance = None;
            menu_state::lock().qemu_running = false;
        }

        // ── Audio toggle ─────────────────────────────────────────────────────
        // Persist the new value then re-arm RESTART so QEMU comes back up with
        // the new -audio config.
        let mut restart_pending = req.restart;
        if req.audio_toggle {
            let mut settings =
                cdj3k_emu_storage::InstanceSettings::load_or_init(config.instance_id);
            settings.audio_enabled = req.audio_enabled;
            if let Err(e) = settings.save(config.instance_id) {
                eprintln!("cdj3k-emu: persisting audio_enabled failed: {e}");
            }
            restart_pending = true;
        }

        // ── Audio output device selection ────────────────────────────────────
        // Same shape as audio_toggle: persist and trigger a restart so the
        // new `-audiodev coreaudio,out.dev=<UID>` arg takes effect.
        if req.audio_device_toggle {
            persist_inst(config.instance_id, |s| {
                s.audio_device_uid = req.audio_device_uid.clone();
            });
            restart_pending = true;
        }

        // ── ALC toggle ───────────────────────────────────────────────────────
        // Push `set audio_sync_enabled <0|1>` to the guest cfg daemon. No
        // restart needed - sysfs takes effect immediately and the LD_PRELOAD
        // shims pick up the change on their next 1 s sysfs refresh tick.
        // Also persist so the setting survives across launches.
        if req.alc_toggle {
            let v = if req.alc_enabled { "1" } else { "0" };
            if let Err(e) = cfg_client.set_param("audio_sync_enabled", v) {
                eprintln!("cdj3k-emu: set audio_sync_enabled failed: {e}");
            }
            persist_inst(config.instance_id, |s| s.alc_enabled = req.alc_enabled);
        }

        // ── Trackpad haptics toggle ──────────────────────────────────────────
        // Host-side only - no QEMU restart, no guest push.  The jog physics
        // layer reads `menu_state.haptic_enabled` at each detent crossing.
        if req.haptic_toggle {
            persist_inst(config.instance_id, |s| {
                s.haptic_enabled = req.haptic_enabled
            });
        }

        // ── First-time ALC push for this QEMU boot ───────────────────────────
        // The kernel module's `audio_sync_enabled` defaults to 0 on every
        // module load.  Push the user's persisted value once per QEMU
        // lifetime, as soon as cfg.sock is responsive.  We detect "responsive"
        // by waiting for the first latency push - which only arrives after
        // the cfg daemon has handshook the port.
        if !alc_pushed_for_boot && cfg_client.latency().is_some() {
            let v = if req.alc_enabled { "1" } else { "0" };
            if let Err(e) = cfg_client.set_param("audio_sync_enabled", v) {
                eprintln!("cdj3k-emu: initial set audio_sync_enabled failed: {e}");
            } else {
                alc_pushed_for_boot = true;
            }
        }

        // ── Latency mirror ───────────────────────────────────────────────────
        if let Some(lat) = cfg_client.latency() {
            menu_state::lock().latency_packed =
                menu_state::pack_latency(lat.total_ms, lat.guest_ms, lat.host_ms);
        }

        // ── Restart ──────────────────────────────────────────────────────────
        if restart_pending {
            if let Some(ref mut inst) = instance {
                inst.stop();
            }
            instance = None;
            let (prev_phys, prev_virt) = {
                let mut s = menu_state::lock();
                s.qemu_running = false;
                (s.usb_phys_mounted_idx, s.usb_virtual_mounted)
            };
            reset_usb(&mut usb, &config, &cfg_client);
            pending_remount = PendingRemount::capture(prev_phys, prev_virt, &phys_disks);
            apply_menu_to_config(&mut config);
            #[cfg(target_os = "macos")]
            match QemuInstance::spawn(config.clone()) {
                Ok(inst) => {
                    eprintln!("cdj3k-emu: QEMU restarted");
                    instance = Some(inst);
                    alc_pushed_for_boot = false;
                    let mut s = menu_state::lock();
                    s.qemu_running = true;
                    s.shade_forced = false;
                }
                Err(e) => eprintln!("cdj3k-emu: restart failed: {e:?}"),
            }
        }

        // ── Network interface selection ───────────────────────────────────────
        if req.selected_iface != prev_net_idx {
            prev_net_idx = req.selected_iface;
            _active_tap_bridge = None;
            _active_vmnet = None;
            config.net_socket_vmnet = None;
            config.net_tap_iface = None;
            config.net_tap_fd = None;

            if req.selected_iface == menu_state::NET_SEL_VMNET_HOST {
                match SocketVmnet::start_host() {
                    Ok(sv) => {
                        eprintln!(
                            "cdj3k-emu: vmnet host-only up  socket={}",
                            sv.socket_path().display()
                        );
                        config.net_socket_vmnet = Some(sv.socket_path().to_path_buf());
                        _active_vmnet = Some(sv);
                    }
                    Err(e) => {
                        let msg = format!("vmnet host-only start failed: {e}");
                        eprintln!("cdj3k-emu: {msg}");
                        let mut s = menu_state::lock();
                        s.selected_interface = menu_state::NET_SEL_NONE;
                        s.net_error_message = Some(msg);
                        prev_net_idx = menu_state::NET_SEL_NONE;
                    }
                }
            } else if req.selected_iface != menu_state::NET_SEL_NONE {
                let iface_name = menu_state::lock()
                    .net_ifaces
                    .get(req.selected_iface as usize)
                    .map(|i| i.name.clone());
                if let Some(name) = iface_name {
                    if name.starts_with("tap") {
                        match TapBridge::setup(&name, config.instance_id) {
                            Ok(tb) => {
                                config.net_tap_iface = Some(tb.qemu_tap.clone());
                                config.net_tap_fd = Some(tb.qemu_tap_fd);
                                _active_tap_bridge = Some(tb);
                            }
                            Err(e) => {
                                let msg = format!("tapbridge setup failed: {e}");
                                eprintln!("cdj3k-emu: {msg}");
                                let mut s = menu_state::lock();
                                s.selected_interface = menu_state::NET_SEL_NONE;
                                s.net_error_message = Some(msg);
                                prev_net_idx = menu_state::NET_SEL_NONE;
                            }
                        }
                    } else {
                        match SocketVmnet::start_bridged(&name) {
                            Ok(sv) => {
                                eprintln!(
                                    "cdj3k-emu: vmnet up on {}  socket={}",
                                    name,
                                    sv.socket_path().display()
                                );
                                config.net_socket_vmnet = Some(sv.socket_path().to_path_buf());
                                _active_vmnet = Some(sv);
                            }
                            Err(e) => {
                                let msg = format!("vmnet start failed: {e}");
                                eprintln!("cdj3k-emu: {msg}");
                                let mut s = menu_state::lock();
                                s.selected_interface = menu_state::NET_SEL_NONE;
                                s.net_error_message = Some(msg);
                                prev_net_idx = menu_state::NET_SEL_NONE;
                            }
                        }
                    }
                }
            }

            apply_menu_to_config(&mut config);
            #[cfg(target_os = "macos")]
            if let Some(ref mut inst) = instance {
                match inst.restart(config.clone()) {
                    Ok(()) => {
                        reset_usb(&mut usb, &config, &cfg_client);
                        alc_pushed_for_boot = false;
                        menu_state::lock().shade_forced = false;
                    }
                    Err(e) => eprintln!("cdj3k-emu: restart after net change failed: {e:?}"),
                }
            } else {
                menu_state::lock().shade_forced = false;
            }

            // Persist the resulting iface name (or None on auto-revert / "none"),
            // or the vmnet-host token when host-only mode is selected.
            let final_name = {
                let s = menu_state::lock();
                match s.selected_interface {
                    menu_state::NET_SEL_NONE => None,
                    menu_state::NET_SEL_VMNET_HOST => {
                        Some(menu_state::NET_IFACE_VMNET_HOST_TOKEN.to_string())
                    }
                    idx => s.net_ifaces.get(idx as usize).map(|i| i.name.clone()),
                }
            };
            persist_inst(config.instance_id, |s| s.net_iface = final_name);
        }

        // ── Virtual USB create ────────────────────────────────────────────────
        // attach_virtual / attach_physical auto-eject whatever was mounted, so
        // we don't need a separate detach step on a virtual↔physical switch.
        if req.usb_create {
            if let (Some(path), Some(ref mut inst)) =
                (req.usb_virtual_img.clone(), instance.as_mut())
            {
                const DEFAULT_SIZE: u64 = 32_000_000_000;
                match usb.create_and_attach_virtual(inst.qmp(), &provider, &path, DEFAULT_SIZE) {
                    Ok(()) => mark_virtual_mounted(config.instance_id, &path),
                    Err(e) => eprintln!("cdj3k-emu: USB create failed: {e}"),
                }
            }
        }

        // ── Virtual USB mount ─────────────────────────────────────────────────
        if req.usb_virt_mount {
            if let (Some(path), Some(ref mut inst)) =
                (req.usb_virtual_img.clone(), instance.as_mut())
            {
                match usb.attach_virtual(inst.qmp(), &provider, &path) {
                    Ok(()) => mark_virtual_mounted(config.instance_id, &path),
                    Err(e) => eprintln!("cdj3k-emu: USB mount failed: {e}"),
                }
            }
        }

        // ── Unified eject (works for both virtual and physical) ──────────────
        if req.usb_eject {
            if let Some(ref mut inst) = instance {
                match usb.detach(inst.qmp(), &provider) {
                    Ok(()) => {
                        let mut s = menu_state::lock();
                        s.usb_virtual_mounted = false;
                        s.usb_phys_mounted_idx = NO_USB_MOUNTED;
                        drop(s);
                        persist_inst(config.instance_id, |s| {
                            s.usb_virtual_path = None;
                            s.usb_physical_bsd = None;
                        });
                    }
                    Err(e) => eprintln!("cdj3k-emu: eject failed: {e}"),
                }
            }
        }

        // ── Physical USB switch ──────────────────────────────────────────────
        // Either a fresh menu pick (usb_phys_toggle >= 0) or a retry of the
        // last one (usb_phys_retry).  Menu-side already filters re-selecting
        // the currently mounted disk; ejecting is via `usb_eject`.
        let toggle_idx = if req.usb_phys_toggle >= 0 {
            last_phys_toggle = req.usb_phys_toggle;
            req.usb_phys_toggle
        } else if req.usb_phys_retry {
            last_phys_toggle
        } else {
            -1
        };
        if toggle_idx >= 0 {
            if let Some(ref mut inst) = instance {
                if let Some(disk) = phys_disks.get(toggle_idx as usize) {
                    match usb.attach_physical(inst.qmp(), disk, &provider) {
                        Ok(()) => {
                            let mut s = menu_state::lock();
                            s.usb_phys_mounted_idx = toggle_idx;
                            s.usb_virtual_mounted = false;
                            drop(s);
                            let bsd = disk.bsd_name.clone();
                            persist_inst(config.instance_id, |s| {
                                s.usb_physical_bsd = Some(bsd);
                                s.usb_virtual_path = None;
                            });
                        }
                        Err(cdj3k_emu_runtime::UsbError::PermissionDenied(_)) => {
                            menu_state::lock().usb_phys_perm_denied = true;
                        }
                        Err(e) => eprintln!("cdj3k-emu: physical USB attach failed: {e}"),
                    }
                }
            }
        }

        // ── Guest-initiated eject (EP122 button → unbind-usb-device.sh) ──────
        // The guest emits `usb_state 0` after it has unmounted everything; we
        // mirror that on the host side (swap medium → placeholder, remount
        // host disk if it was physical) and update menu_state + persistence.
        // `usb_state 1` arrives from the attach script for our own
        // host-initiated mounts and carries no new info, so we ignore it.
        if let Some(false) = cfg_client.poll_usb_state() {
            if let Some(ref mut inst) = instance {
                match usb.acknowledge_guest_eject(inst.qmp(), &provider) {
                    Ok(true) => {
                        let mut s = menu_state::lock();
                        s.usb_virtual_mounted = false;
                        s.usb_phys_mounted_idx = NO_USB_MOUNTED;
                        drop(s);
                        persist_inst(config.instance_id, |s| {
                            s.usb_virtual_path = None;
                            s.usb_physical_bsd = None;
                        });
                    }
                    Ok(false) => {}
                    Err(e) => eprintln!("cdj3k-emu: ack guest eject failed: {e}"),
                }
            }
        }

        // ── Deferred storage remount after QEMU restart ───────────────────────
        if let (Some(ref target), Some(ref mut inst)) = (&pending_remount, instance.as_mut()) {
            if target.try_apply(&mut usb, inst.qmp(), &provider) {
                pending_remount = None;
            }
        }

        // ── Refresh network interface list ────────────────────────────────────
        if last_net_refresh.elapsed() >= NET_REFRESH_INTERVAL {
            menu_state::refresh_net_interfaces();
            last_net_refresh = Instant::now();
        }

        // ── Refresh physical disk list ────────────────────────────────────────
        if last_disk_refresh.elapsed() >= DISK_REFRESH_INTERVAL {
            let fresh = provider.list_removable();
            let display: Vec<menu_state::PhysicalDisk> = fresh
                .iter()
                .map(|d| menu_state::PhysicalDisk {
                    bsd_name: d.bsd_name.clone(),
                    label: d.label.clone(),
                })
                .collect();
            let restore_toggle = {
                let mut s = menu_state::lock();
                if s.usb_phys_disks != display {
                    s.usb_phys_disks = display;
                    s.usb_phys_list_version = s.usb_phys_list_version.wrapping_add(1);
                }
                // One-shot: try to restore the saved physical disk if it just
                // appeared. We must compute the index against the fresh list
                // we just published (not the old guard).
                let restore = (!phys_restore_attempted)
                    .then(|| saved_phys_bsd.as_ref())
                    .flatten()
                    .and_then(|name| fresh.iter().position(|d| &d.bsd_name == name))
                    .map(|idx| idx as i32);
                if restore.is_some() {
                    s.usb_phys_toggle_idx = restore.unwrap();
                }
                restore.is_some()
            };
            phys_disks = fresh;
            last_disk_refresh = Instant::now();
            if restore_toggle || !phys_restore_attempted {
                phys_restore_attempted = true;
            }
        }

        std::thread::sleep(POLL_INTERVAL);
    }
}

/// One-acquisition snapshot of every flag/value the worker needs each tick.
struct Requests {
    boot: bool,
    stop: bool,
    audio_toggle: bool,
    audio_device_toggle: bool,
    alc_toggle: bool,
    restart: bool,
    usb_create: bool,
    usb_virt_mount: bool,
    usb_eject: bool,
    usb_phys_toggle: i32,
    usb_phys_retry: bool,
    audio_enabled: bool,
    audio_device_uid: Option<String>,
    alc_enabled: bool,
    haptic_toggle: bool,
    haptic_enabled: bool,
    selected_iface: u32,
    usb_virtual_img: Option<std::path::PathBuf>,
}

fn apply_menu_to_config(config: &mut QemuConfig) {
    let s = menu_state::lock();
    config.service_mode = s.service_mode;
    config.audio = s.audio_enabled;
    config.audio_device_uid = s.audio_device_uid.clone();
}

/// Mark the virtual USB image at `path` as the current mount for this
/// instance: set the menu mirror (and clear any physical-mount index since
/// attach_virtual auto-ejects), then persist `usb_virtual_path` + clear
/// any prior `usb_physical_bsd` so the next launch restores the same one.
/// Used by both the `usb_create` and `usb_virt_mount` request paths.
fn mark_virtual_mounted(instance_id: u32, path: &std::path::Path) {
    {
        let mut s = menu_state::lock();
        s.usb_virtual_mounted = true;
        s.usb_phys_mounted_idx = NO_USB_MOUNTED;
    }
    let path = path.to_path_buf();
    persist_inst(instance_id, |s| {
        s.usb_virtual_path = Some(path);
        s.usb_physical_bsd = None;
    });
}

/// Load, mutate, and re-save InstanceSettings.  Errors are logged and
/// swallowed - persistence is best-effort and should never break runtime flow.
fn persist_inst<F>(instance_id: u32, mutate: F)
where
    F: FnOnce(&mut cdj3k_emu_storage::InstanceSettings),
{
    let mut s = cdj3k_emu_storage::InstanceSettings::load_or_init(instance_id);
    mutate(&mut s);
    if let Err(e) = s.save(instance_id) {
        eprintln!("cdj3k-emu: persisting instance settings failed: {e}");
    }
}

/// Clear USB manager and mounted-state fields after any QEMU restart.
/// The new QEMU instance always starts with the placeholder; nothing is mounted.
fn reset_usb(usb: &mut UsbManager, config: &QemuConfig, cfg: &CfgClient) {
    *usb = UsbManager::new(config.usb_placeholder_path(), cfg.clone());
    let mut s = menu_state::lock();
    s.usb_virtual_mounted = false;
    s.usb_phys_mounted_idx = NO_USB_MOUNTED;
}

enum PendingRemount {
    Physical {
        disk: cdj3k_emu_runtime::PhysicalDisk,
        idx: i32,
    },
    Virtual {
        path: std::path::PathBuf,
    },
}

impl PendingRemount {
    /// Build from pre-reset state. Returns None if nothing was mounted.
    fn capture(
        prev_phys_idx: i32,
        prev_virt: bool,
        phys_disks: &[cdj3k_emu_runtime::PhysicalDisk],
    ) -> Option<Self> {
        if prev_phys_idx >= 0 {
            phys_disks
                .get(prev_phys_idx as usize)
                .map(|d| Self::Physical {
                    disk: d.clone(),
                    idx: prev_phys_idx,
                })
        } else if prev_virt {
            menu_state::lock()
                .usb_virtual_img
                .clone()
                .map(|path| Self::Virtual { path })
        } else {
            None
        }
    }

    /// Try to apply the remount. Returns true on success (caller clears pending).
    fn try_apply(
        &self,
        usb: &mut UsbManager,
        qmp: &mut cdj3k_emu_runtime::QmpClient,
        provider: &MacOsDiskProvider,
    ) -> bool {
        match self {
            Self::Physical { disk, idx } => match usb.attach_physical(qmp, disk, provider) {
                Ok(()) => {
                    menu_state::lock().usb_phys_mounted_idx = *idx;
                    true
                }
                Err(e) => {
                    eprintln!("cdj3k-emu: physical USB remount pending: {e}");
                    false
                }
            },
            Self::Virtual { path } => match usb.attach_virtual(qmp, provider, path) {
                Ok(()) => {
                    menu_state::lock().usb_virtual_mounted = true;
                    true
                }
                Err(e) => {
                    eprintln!("cdj3k-emu: virtual USB remount pending: {e}");
                    false
                }
            },
        }
    }
}
