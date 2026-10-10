//! USB storage: menu mount/eject requests, guest-initiated ejects, the
//! remount after a QEMU restart, and the host's removable-disk list.

use std::time::Instant;

use cdj3k_emu_platform::menu_state::{self, NO_USB_MOUNTED};
use cdj3k_emu_runtime::{
    CfgClient, DiskProvider, HostDiskProvider, PhysicalDisk, QemuConfig, UsbManager,
};

use super::{persist_inst, Requests, Worker, DISK_REFRESH_INTERVAL};

impl Worker {
    /// attach_virtual and attach_physical eject whatever was mounted, so
    /// switching between a virtual and a physical disk needs no detach step.
    pub(super) fn handle_usb_requests(&mut self, req: &Requests) {
        if req.usb_create {
            self.create_virtual(req.usb_virtual_img.clone());
        }
        if req.usb_virt_mount {
            self.mount_virtual(req.usb_virtual_img.clone());
        }
        // Eject works for both virtual and physical disks.
        if req.usb_eject {
            self.eject();
        }
        let toggle_idx = self.physical_target(req);
        if toggle_idx >= 0 {
            self.attach_physical(toggle_idx);
        }
    }

    fn create_virtual(&mut self, path: Option<std::path::PathBuf>) {
        if let (Some(path), Some(inst)) = (path, self.instance.as_mut()) {
            const DEFAULT_SIZE: u64 = 32_000_000_000;
            match self.usb.create_and_attach_virtual(
                inst.qmp(),
                &self.provider,
                &path,
                DEFAULT_SIZE,
            ) {
                Ok(()) => mark_virtual_mounted(self.config.instance_id, &path),
                Err(e) => eprintln!("cdj3k-emu: creating the USB image failed: {e}"),
            }
        }
    }

    fn mount_virtual(&mut self, path: Option<std::path::PathBuf>) {
        if let (Some(path), Some(inst)) = (path, self.instance.as_mut()) {
            match self.usb.attach_virtual(inst.qmp(), &self.provider, &path) {
                Ok(()) => mark_virtual_mounted(self.config.instance_id, &path),
                Err(e) => eprintln!("cdj3k-emu: mounting the USB image failed: {e}"),
            }
        }
    }

    fn eject(&mut self) {
        if let Some(inst) = self.instance.as_mut() {
            match self.usb.detach(inst.qmp(), &self.provider) {
                Ok(()) => mark_unmounted(self.config.instance_id),
                Err(e) => eprintln!("cdj3k-emu: ejecting the USB disk failed: {e}"),
            }
        }
    }

    /// The index of the physical disk to attach this tick, or -1. The disk is
    /// either a new menu pick or a retry of the last one. The menu already
    /// ignores a re-selection of the mounted disk, and ejecting goes through
    /// `usb_eject`.
    fn physical_target(&mut self, req: &Requests) -> i32 {
        if req.usb_phys_toggle >= 0 {
            self.last_phys_toggle = req.usb_phys_toggle;
            req.usb_phys_toggle
        } else if req.usb_phys_retry {
            self.last_phys_toggle
        } else {
            -1
        }
    }

    fn attach_physical(&mut self, idx: i32) {
        let Some(inst) = self.instance.as_mut() else {
            return;
        };
        let Some(disk) = self.phys_disks.get(idx as usize) else {
            return;
        };
        match self.usb.attach_physical(inst.qmp(), disk, &self.provider) {
            Ok(()) => mark_physical_mounted(self.config.instance_id, idx, disk.bsd_name.clone()),
            Err(cdj3k_emu_runtime::UsbError::PermissionDenied(_)) => {
                menu_state::lock().usb_phys_perm_denied =
                    Some(cdj3k_emu_runtime::disk::RETRY_PROMPT);
            }
            Err(e) => {
                eprintln!("cdj3k-emu: attaching the physical USB disk failed: {e}");
                menu_state::lock().usb_error_message = Some(match &e {
                    cdj3k_emu_runtime::UsbError::Io(io) => io.to_string(),
                    e => e.to_string(),
                });
            }
        }
    }

    /// Handle an eject started in the guest, where the deck's eject button
    /// runs unbind-usb-device.sh. The guest emits `usb_state 0` after it has
    /// unmounted everything. The host then swaps the medium for the
    /// placeholder, remounts the host disk if it was physical, and updates
    /// menu_state and the saved settings. The attach script emits
    /// `usb_state 1` for mounts started on the host; it carries no new
    /// information, so it is ignored.
    pub(super) fn acknowledge_guest_eject(&mut self) {
        if let Some(false) = self.cfg_client.poll_usb_state() {
            if let Some(inst) = self.instance.as_mut() {
                match self.usb.acknowledge_guest_eject(inst.qmp(), &self.provider) {
                    Ok(true) => mark_unmounted(self.config.instance_id),
                    Ok(false) => {}
                    Err(e) => eprintln!("cdj3k-emu: acknowledging the guest's eject failed: {e}"),
                }
            }
        }
    }

    /// Reset the USB manager for a new QEMU and queue a remount of the disk
    /// that was mounted before. `prev_phys_idx` and `prev_virt` come from the
    /// menu.
    pub(super) fn reset_usb_and_queue_remount(&mut self, prev_phys_idx: i32, prev_virt: bool) {
        reset_usb(&mut self.usb, &self.config, &self.cfg_client);
        self.pending_remount = PendingRemount::capture(prev_phys_idx, prev_virt, &self.phys_disks);
    }

    pub(super) fn apply_pending_remount(&mut self) {
        if let (Some(target), Some(inst)) = (&self.pending_remount, self.instance.as_mut()) {
            if target.try_apply(&mut self.usb, inst.qmp(), &self.provider) {
                self.pending_remount = None;
            }
        }
    }

    /// Publish the host's removable disks to the menu again. The first scan also
    /// picks the slot's saved physical disk if it is present.
    pub(super) fn refresh_disks(&mut self) {
        if self.last_disk_refresh.elapsed() < DISK_REFRESH_INTERVAL {
            return;
        }
        let fresh = self.provider.list_removable();
        let restore = self.saved_disk_index(&fresh);
        publish_disks(&fresh, restore);
        self.phys_disks = fresh;
        self.last_disk_refresh = Instant::now();
        self.phys_restore_attempted = true;
    }

    fn saved_disk_index(&self, fresh: &[PhysicalDisk]) -> Option<i32> {
        if self.phys_restore_attempted {
            return None;
        }
        let name = self.saved_phys_bsd.as_ref()?;
        fresh
            .iter()
            .position(|d| &d.bsd_name == name)
            .map(|idx| idx as i32)
    }
}

/// Mirror `fresh` into the menu's disk list, and queue `restore` as a
/// physical-disk pick.
fn publish_disks(fresh: &[PhysicalDisk], restore: Option<i32>) {
    let display: Vec<menu_state::PhysicalDisk> = fresh
        .iter()
        .map(|d| menu_state::PhysicalDisk {
            bsd_name: d.bsd_name.clone(),
            label: d.label.clone(),
        })
        .collect();
    let mut s = menu_state::lock();
    if s.usb_phys_disks != display {
        s.usb_phys_disks = display;
        s.usb_phys_list_version = s.usb_phys_list_version.wrapping_add(1);
    }
    if let Some(idx) = restore {
        s.usb_phys_toggle_idx = idx;
    }
}

/// Mark the virtual USB image at `path` as this slot's current mount. Update
/// the menu and clear its physical-mount index, because attach_virtual ejects
/// any physical disk. Then save `usb_virtual_path` and clear any earlier
/// `usb_physical_bsd`, so that the next launch restores the same image.
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

/// Mark physical disk `idx` (`bsd_name`) as the current mount, in the menu
/// and in the slot's settings.
fn mark_physical_mounted(instance_id: u32, idx: i32, bsd_name: String) {
    {
        let mut s = menu_state::lock();
        s.usb_phys_mounted_idx = idx;
        s.usb_virtual_mounted = false;
    }
    persist_inst(instance_id, |s| {
        s.usb_physical_bsd = Some(bsd_name);
        s.usb_virtual_path = None;
    });
}

/// Mark nothing mounted, in the menu and in the slot's settings.
fn mark_unmounted(instance_id: u32) {
    {
        let mut s = menu_state::lock();
        s.usb_virtual_mounted = false;
        s.usb_phys_mounted_idx = NO_USB_MOUNTED;
    }
    persist_inst(instance_id, |s| {
        s.usb_virtual_path = None;
        s.usb_physical_bsd = None;
    });
}

/// Reset the USB manager and the mounted-state fields after a QEMU restart. A
/// new QEMU instance always starts with the placeholder, so nothing is mounted.
pub(super) fn reset_usb(usb: &mut UsbManager, config: &QemuConfig, cfg: &CfgClient) {
    *usb = UsbManager::new(config, cfg.clone());
    let mut s = menu_state::lock();
    s.usb_virtual_mounted = false;
    s.usb_phys_mounted_idx = NO_USB_MOUNTED;
}

pub(super) enum PendingRemount {
    Physical { disk: PhysicalDisk, idx: i32 },
    Virtual { path: std::path::PathBuf },
}

impl PendingRemount {
    /// Build the remount from the state before the reset. Returns None if
    /// nothing was mounted.
    fn capture(prev_phys_idx: i32, prev_virt: bool, phys_disks: &[PhysicalDisk]) -> Option<Self> {
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

    /// Try to apply the remount. Returns true on success, and the caller then
    /// clears the pending remount.
    fn try_apply(
        &self,
        usb: &mut UsbManager,
        qmp: &mut cdj3k_emu_runtime::QmpClient,
        provider: &HostDiskProvider,
    ) -> bool {
        match self {
            Self::Physical { disk, idx } => match usb.attach_physical(qmp, disk, provider) {
                Ok(()) => {
                    menu_state::lock().usb_phys_mounted_idx = *idx;
                    true
                }
                Err(e) => {
                    eprintln!("cdj3k-emu: remounting the physical USB disk failed; retrying: {e}");
                    false
                }
            },
            Self::Virtual { path } => match usb.attach_virtual(qmp, provider, path) {
                Ok(()) => {
                    menu_state::lock().usb_virtual_mounted = true;
                    true
                }
                Err(e) => {
                    eprintln!("cdj3k-emu: remounting the virtual USB image failed; retrying: {e}");
                    false
                }
            },
        }
    }
}
