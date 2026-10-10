//! The background thread that polls `menu_state` and drives the QEMU instance
//! and the USB manager.

mod mods;
mod net;
mod pc_link;
mod qemu;
mod usb;

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use cdj3k_emu_platform::menu_state::{self, APP_SHUTDOWN, NO_USB_MOUNTED};
use cdj3k_emu_runtime::mods_report::ModsReport;
use cdj3k_emu_runtime::{
    host_disk_provider, register_worker_thread, CfgClient, HostDiskProvider, NetKeepAlive,
    PhysicalDisk, QemuConfig, QemuInstance, UsbManager,
};

use crate::launch::ModsLaunch;

pub use net::adopt_attachment;
use pc_link::PcLinkState;
use usb::PendingRemount;

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const DISK_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const NET_REFRESH_INTERVAL: Duration = Duration::from_secs(3);

/// The network backend, set up before the first QEMU start so that the first
/// process already has the right `-netdev`. The worker holds it for its
/// lifetime, and dropping it tears the bridge down. vmnet needs nothing here,
/// because QEMU opens the interface itself and releases it when it exits.
pub struct PrebuiltNet {
    pub keep: cdj3k_emu_runtime::NetKeepAlive,
    /// The initial value of `prev_net_idx`, so that the worker does not
    /// restart QEMU on its first poll iteration.
    pub initial_net_idx: u32,
}

pub fn spawn(
    instance: Option<QemuInstance>,
    config: QemuConfig,
    prebuilt_net: PrebuiltNet,
    mods: ModsLaunch,
) {
    let handle = std::thread::Builder::new()
        .name("cdj3k-emu-runtime-worker".into())
        .spawn(move || run(instance, config, prebuilt_net, mods))
        .expect("failed to spawn runtime worker");
    register_worker_thread(handle);
}

fn run(
    instance: Option<QemuInstance>,
    config: QemuConfig,
    prebuilt_net: PrebuiltNet,
    mods: ModsLaunch,
) {
    let mut worker = Worker::new(instance, config, prebuilt_net, mods);
    while worker.tick() {
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Everything the worker carries from one tick to the next. Fields drop in
/// declaration order: the PC-link bridge first, while its socket is valid,
/// then the network keep-alive, then the USB manager and the cfg client.
struct Worker {
    pc_link: PcLinkState,
    /// The storage to mount again after the next QEMU boot, once cfg.sock is ready.
    pending_remount: Option<PendingRemount>,
    /// Mods whose journal was requested and has not arrived yet.
    mod_logs_due: Vec<String>,
    /// The last mods report recorded, so each new report is saved only once.
    last_mods_report: ModsReport,
    /// Set once per QEMU lifetime and cleared on restart, so that the newly
    /// loaded kernel module receives the user's saved ALC value once cfg.sock
    /// is up.
    alc_pushed_for_boot: bool,
    last_net_refresh: Instant,
    last_phys_toggle: i32,
    prev_net_idx: u32,
    /// The TAP bridge. Dropping it on app exit or on an interface change tears
    /// down the host TAP. Sockets and directories are removed only by
    /// `cleanup_runtime_files` when the process exits.
    _active_net: NetKeepAlive,
    last_disk_refresh: Instant,
    phys_disks: Vec<PhysicalDisk>,
    usb: UsbManager,
    cfg_client: CfgClient,
    provider: HostDiskProvider,
    mods: ModsLaunch,
    config: QemuConfig,
    instance: Option<QemuInstance>,
    /// The physical disk persisted for this slot, restored on the first disk scan.
    saved_phys_bsd: Option<String>,
    phys_restore_attempted: bool,
}

impl Worker {
    fn new(
        instance: Option<QemuInstance>,
        config: QemuConfig,
        prebuilt_net: PrebuiltNet,
        mods: ModsLaunch,
    ) -> Self {
        {
            let mut s = menu_state::lock();
            s.qemu_running = instance.is_some();
            // A restart through a relaunch forced the shade on. The new QEMU is
            // up, so clear it; the boot shade stays until the first frames arrive.
            if instance.is_some() {
                s.shade_forced = false;
            }
        }
        let provider = host_disk_provider();
        let cfg_client = CfgClient::new(&config.sock_dir());
        let usb = UsbManager::new(&config, cfg_client.clone());
        let pc_link = PcLinkState::new();
        // Fill the interface cache now, so that the menu has data when it first opens.
        menu_state::refresh_net_interfaces();

        // main.rs set usb_virtual_img if the saved image file existed. Queue a
        // mount of it that runs once QEMU is up.
        let pending_remount = menu_state::lock()
            .usb_virtual_img
            .clone()
            .filter(|path| path.exists())
            .map(|path| PendingRemount::Virtual { path });
        let saved_phys_bsd =
            cdj3k_emu_storage::InstanceSettings::load_or_init(config.instance_id).usb_physical_bsd;

        Self {
            pc_link,
            pending_remount,
            mod_logs_due: Vec::new(),
            last_mods_report: ModsReport::Unknown,
            alc_pushed_for_boot: false,
            last_net_refresh: Instant::now() - NET_REFRESH_INTERVAL,
            last_phys_toggle: -1,
            prev_net_idx: prebuilt_net.initial_net_idx,
            _active_net: prebuilt_net.keep,
            last_disk_refresh: Instant::now() - DISK_REFRESH_INTERVAL,
            phys_disks: Vec::new(),
            usb,
            cfg_client,
            provider,
            mods,
            config,
            instance,
            phys_restore_attempted: saved_phys_bsd.is_none(),
            saved_phys_bsd,
        }
    }

    /// Run one poll iteration. Returns false when the worker should exit.
    fn tick(&mut self) -> bool {
        if self.exit_requested() {
            self.shut_down();
            return false;
        }
        if self.guest_exited() {
            self.reap_guest();
            if APP_SHUTDOWN.load(Ordering::Relaxed) {
                menu_state::lock().qemu_respawning = false;
                return false;
            }
            self.respawn_after_guest_exit();
        }
        self.request_mods_restart();

        let req = Requests::drain(self.instance.is_none());
        if req.boot {
            self.spawn_qemu(qemu::SpawnCause::Provisioned);
        }
        if req.stop {
            self.stop_qemu();
        }
        let settings_need_restart = self.apply_setting_toggles(&req);
        let restart_pending = req.restart || settings_need_restart;
        self.reconcile_pc_link(&req);
        self.push_alc_once(req.alc_enabled);
        self.mirror_latency();
        self.track_mods_report();
        self.mirror_guest_cfg_live();
        if restart_pending {
            self.restart_qemu();
        }
        if req.selected_iface != self.prev_net_idx {
            self.switch_network(req.selected_iface);
        }
        self.handle_usb_requests(&req);
        self.acknowledge_guest_eject();
        self.apply_pending_remount();
        self.refresh_net_list();
        self.refresh_disks();
        true
    }

    /// Return true on app shutdown or when the setup window retires this
    /// worker. The worker also retires when a restart is due and a finished
    /// install is waiting; a guest reboot and the menu's Restart both count.
    /// The shell then launches again, and the launch swaps the install in.
    fn exit_requested(&self) -> bool {
        let mut retire = std::mem::take(&mut menu_state::lock().worker_exit_requested);
        let restarting = menu_state::lock().restart_requested
            || self.instance.as_ref().is_some_and(|i| !i.is_running());
        if !retire
            && restarting
            && cdj3k_emu_storage::pending_install(self.config.instance_id).is_some()
        {
            let mut s = menu_state::lock();
            s.restart_requested = false;
            s.relaunch_requested = true;
            s.shade_forced = true;
            retire = true;
        }
        APP_SHUTDOWN.load(Ordering::Relaxed) || retire
    }

    fn shut_down(&mut self) {
        // Save an Enable Mods toggle that the request handling would
        // otherwise miss.
        let toggled = {
            let mut s = menu_state::lock();
            std::mem::take(&mut s.mods_toggle_requested).then_some(s.mods_enabled)
        };
        if let Some(on) = toggled {
            persist_inst(self.config.instance_id, |s| s.mods_enabled = on);
        }
        // Close the PC-link bridge first, so its threads exit while the socket
        // is still valid. Otherwise the reader thread blocks on a half-closed fd.
        self.pc_link.close_bridge();
        self.stop_instance();
        // Clear any restart request, because it would restart the next
        // worker's new QEMU.
        let mut s = menu_state::lock();
        s.qemu_running = false;
        s.restart_requested = false;
    }

    /// Save the toggled settings and apply the ones that act at once. Returns
    /// true when a toggled setting needs a QEMU restart. The audio switch and
    /// the output device need one, because they are QEMU's `-audio` and
    /// `-audiodev` arguments.
    fn apply_setting_toggles(&mut self, req: &Requests) -> bool {
        let id = self.config.instance_id;
        if req.audio_toggle {
            persist_inst(id, |s| s.audio_enabled = req.audio_enabled);
        }
        if req.audio_device_toggle {
            persist_inst(id, |s| {
                s.audio_device_uid = req.audio_device_uid.clone();
            });
        }
        if req.alc_toggle {
            self.set_alc(req.alc_enabled);
        }
        // The haptic setting acts on the host only. The jog physics layer
        // reads `menu_state.haptic_enabled` at each detent crossing.
        if req.haptic_toggle {
            persist_inst(id, |s| s.haptic_enabled = req.haptic_enabled);
        }
        // The restart that the toggle requests reads this setting when it
        // builds the initramfs.
        if req.mods_toggle {
            persist_inst(id, |s| s.mods_enabled = req.mods_enabled);
        }
        req.audio_toggle || req.audio_device_toggle
    }

    /// Push `set audio_sync_enabled <0|1>` to the guest's cfg daemon and save
    /// it. No restart is needed: the sysfs value takes effect at once, and the
    /// LD_PRELOAD shims read it on their next sysfs refresh, which runs every
    /// second.
    fn set_alc(&self, enabled: bool) {
        let v = if enabled { "1" } else { "0" };
        if let Err(e) = self.cfg_client.set_param("audio_sync_enabled", v) {
            eprintln!("cdj3k-emu: setting audio_sync_enabled failed: {e}");
        }
        persist_inst(self.config.instance_id, |s| s.alc_enabled = enabled);
    }

    /// The kernel module's `audio_sync_enabled` is 0 after every module load.
    /// Push the user's saved value once per QEMU lifetime, as soon as cfg.sock
    /// responds. The first latency report is the sign that it responds: it
    /// arrives only after the cfg daemon has completed its handshake on the port.
    fn push_alc_once(&mut self, enabled: bool) {
        if self.alc_pushed_for_boot || self.cfg_client.latency().is_none() {
            return;
        }
        let v = if enabled { "1" } else { "0" };
        if let Err(e) = self.cfg_client.set_param("audio_sync_enabled", v) {
            eprintln!("cdj3k-emu: setting the initial audio_sync_enabled failed: {e}");
        } else {
            self.alc_pushed_for_boot = true;
        }
    }

    fn mirror_latency(&self) {
        if let Some(lat) = self.cfg_client.latency() {
            menu_state::lock().latency_packed =
                menu_state::pack_latency(lat.total_ms, lat.guest_ms, lat.host_ms);
        }
    }

    fn mirror_guest_cfg_live(&self) {
        let cfg_live = self.instance.is_some() && self.cfg_client.guest_heard();
        menu_state::lock().guest_cfg_live = cfg_live;
    }
}

/// A snapshot of every flag and value the worker needs each tick, taken under
/// one lock acquisition.
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
    mods_toggle: bool,
    mods_enabled: bool,
    pc_link_toggle: bool,
    pc_link_enabled: bool,
    selected_iface: u32,
    usb_virtual_img: Option<std::path::PathBuf>,
}

impl Requests {
    /// Drain every one-shot flag in a single lock acquisition. A boot request
    /// counts only while no QEMU runs.
    fn drain(qemu_down: bool) -> Self {
        let mut s = menu_state::lock();
        Self {
            boot: std::mem::take(&mut s.qemu_boot_requested) && qemu_down,
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
            mods_toggle: std::mem::take(&mut s.mods_toggle_requested),
            mods_enabled: s.mods_enabled,
            pc_link_toggle: std::mem::take(&mut s.pc_link_toggle_requested),
            pc_link_enabled: s.pc_link_enabled,
            selected_iface: s.selected_interface,
            usb_virtual_img: s.usb_virtual_img.clone(),
        }
    }
}

/// Load, change and save the InstanceSettings under the settings lock. Errors
/// are only logged, so that a failed save never stops the worker.
fn persist_inst<F>(instance_id: u32, mutate: F)
where
    F: FnOnce(&mut cdj3k_emu_storage::InstanceSettings),
{
    if let Err(e) = cdj3k_emu_storage::InstanceSettings::update(instance_id, mutate) {
        eprintln!("cdj3k-emu: persisting instance settings failed: {e}");
    }
}
