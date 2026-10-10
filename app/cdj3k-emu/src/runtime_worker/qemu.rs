//! The QEMU process: starting it, stopping it, and restarting it after the
//! guest exits or the menu asks.

use cdj3k_emu_platform::menu_state;
use cdj3k_emu_runtime::mods_report::ModsReport;
use cdj3k_emu_runtime::{InstanceError, QemuConfig, QemuInstance};
use cdj3k_emu_storage::mods::BootMods;

use super::Worker;
use crate::launch::ModsLaunch;

/// Why the worker starts QEMU. Each cause logs its own lines.
#[derive(Clone, Copy)]
pub(super) enum SpawnCause {
    /// The menu's boot request after the firmware was installed.
    Provisioned,
    /// The menu's restart, or a setting that needs a restart.
    Restart,
    /// The guest shut down by itself.
    GuestExit,
}

impl SpawnCause {
    fn log_started(self) {
        match self {
            Self::Provisioned => eprintln!("cdj3k-emu: QEMU started after provisioning"),
            Self::Restart => eprintln!("cdj3k-emu: QEMU restarted"),
            Self::GuestExit => {}
        }
    }

    fn log_failed(self, e: &InstanceError) {
        match self {
            Self::Provisioned => {
                eprintln!("cdj3k-emu: starting QEMU after provisioning failed: {e:?}")
            }
            Self::Restart => eprintln!("cdj3k-emu: restarting QEMU failed: {e:?}"),
            Self::GuestExit => {
                eprintln!("cdj3k-emu: restarting QEMU after the guest exited failed: {e:?}")
            }
        }
    }
}

impl Worker {
    pub(super) fn guest_exited(&self) -> bool {
        self.instance
            .as_ref()
            .map(|i| !i.is_running())
            .unwrap_or(false)
    }

    /// Clean up after the guest shuts down by itself. The PC-link bridge stays
    /// open, because its endpoints must outlive the restart or the HID name
    /// that rekordbox polls goes stale. The socket closes with the guest; the
    /// reader gets EOF and the liveness check reconnects.
    pub(super) fn reap_guest(&mut self) {
        self.stop_instance();
        self.pc_link.on_qemu_stopped();
        let mut s = menu_state::lock();
        s.qemu_running = false;
        s.qemu_respawning = true;
    }

    pub(super) fn respawn_after_guest_exit(&mut self) {
        eprintln!("cdj3k-emu: QEMU exited; restarting it");
        let (prev_phys, prev_virt) = {
            let s = menu_state::lock();
            (s.usb_phys_mounted_idx, s.usb_virtual_mounted)
        };
        self.respawn(prev_phys, prev_virt, SpawnCause::GuestExit);
    }

    /// Handle the menu's Stop by stopping QEMU and the PC-link bridge.
    pub(super) fn stop_qemu(&mut self) {
        self.pc_link.stop();
        self.stop_instance();
        menu_state::lock().qemu_running = false;
    }

    pub(super) fn restart_qemu(&mut self) {
        self.stop_instance();
        self.pc_link.on_qemu_stopped();
        let (prev_phys, prev_virt) = {
            let mut s = menu_state::lock();
            s.qemu_running = false;
            s.qemu_respawning = true;
            (s.usb_phys_mounted_idx, s.usb_virtual_mounted)
        };
        self.respawn(prev_phys, prev_virt, SpawnCause::Restart);
    }

    /// Start a new QEMU after the old one stopped, and remount the storage
    /// that was mounted (`prev_phys_idx` / `prev_virt`, read from the menu).
    fn respawn(&mut self, prev_phys_idx: i32, prev_virt: bool, cause: SpawnCause) {
        self.reset_usb_and_queue_remount(prev_phys_idx, prev_virt);
        self.spawn_qemu(cause);
        menu_state::lock().qemu_respawning = false;
    }

    pub(super) fn stop_instance(&mut self) {
        if let Some(mut inst) = self.instance.take() {
            inst.stop();
        }
    }

    /// Start QEMU with the menu's current settings and a freshly prepared
    /// mods initramfs.
    pub(super) fn spawn_qemu(&mut self, cause: SpawnCause) {
        let boot = apply_menu_to_config(&mut self.config, &self.mods);
        self.cfg_client.new_boot();
        self.last_mods_report = ModsReport::Unknown;
        match QemuInstance::spawn(self.config.clone()) {
            Ok(inst) => {
                cdj3k_emu_storage::mods::set_current_boot(boot);
                cause.log_started();
                self.instance = Some(inst);
                self.alc_pushed_for_boot = false;
                let mut s = menu_state::lock();
                s.qemu_running = true;
                s.shade_forced = false;
            }
            Err(e) => cause.log_failed(&e),
        }
    }
}

/// Copy the menu's boot settings into `config` and prepare the boot's mods.
/// Returns the boot's mods, for `set_current_boot` once QEMU has started.
pub(super) fn apply_menu_to_config(config: &mut QemuConfig, mods: &ModsLaunch) -> BootMods {
    {
        let s = menu_state::lock();
        config.service_mode = s.service_mode;
        config.audio = s.audio_enabled;
        config.audio_device_uid = s.audio_device_uid.clone();
    }
    let boot = mods.boot(config.instance_id, config.model);
    config.initramfs = boot.initramfs.clone();
    boot
}
