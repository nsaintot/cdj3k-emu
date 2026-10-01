//! The USB slot: what medium is in it, and how each host gets one there.
//!
//! The virtual drive is always present in QEMU's argv, backed by a 1-sector
//! placeholder at boot. Attach/detach hot-swap the medium through QMP and tell
//! the guest over `cdj3k.cfg`; what a host can offer as a medium, and what it
//! takes to make one, is the adapter module's business.

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod imp;
#[cfg(windows)]
#[path = "windows.rs"]
mod imp;
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
#[path = "unsupported.rs"]
mod imp;

#[cfg(target_os = "macos")]
mod macos_disk;
mod linux_disk;
#[cfg(windows)]
mod windows_disk;
mod windows_parse;

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::qmp::{QmpClient, QmpError};

const USB_DRIVE_ID: &str = "usb0";

/// What asking the host again for a refused raw disk involves, for the
/// prompt that offers it.
pub use imp::RETRY_PROMPT;

/// Whether a raw disk reaches QEMU as a descriptor, which needs the second
/// monitor ([`crate::QemuConfig::qmp_fd_socket`]).
pub use imp::PASSES_FDS;

/// Each host's adapter implements [`DiskProvider`]; a host that does not fails
/// the build here rather than at the first caller.
const _: fn() = || {
    fn assert_provider<T: DiskProvider>() {}
    assert_provider::<HostDiskProvider>();
};

// ── Domain types ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct PhysicalDisk {
    /// The host's name for the whole disk: `disk2`, `sdb`.
    pub bsd_name: String,
    /// For display: `PIONEER (32.1 GB)`.
    pub label: String,
    /// The device node: `/dev/disk2`, `/dev/sdb`.
    pub bsd_path: String,
    pub size_bytes: u64,
}

// ── DiskProvider port (hexagonal) ────────────────────────────────────────────

pub trait DiskProvider: Send + Sync {
    fn list_removable(&self) -> Vec<PhysicalDisk>;
    fn unmount_host(&self, bsd_name: &str) -> std::io::Result<()>;
    fn remount_host(&self, bsd_name: &str) -> std::io::Result<()>;
    /// Show the image in the host's file manager.
    fn reveal_in_file_manager(&self, path: &str) {
        cdj3k_emu_platform::desktop::reveal_in_file_manager(Path::new(path));
    }
}

// ── Host adapter ─────────────────────────────────────────────────────────────

/// The disk provider for this host; exactly one adapter is compiled per build.
pub struct HostDiskProvider;

/// The provider for this host.
pub fn host_disk_provider() -> HostDiskProvider {
    HostDiskProvider
}

/// Create a raw image file of `size_bytes`, for the adapters that format one.
// Adapters that make another kind of image do not call it.
#[allow(dead_code)]
pub(crate) fn create_raw_image(img_path: &Path, size_bytes: u64) -> std::io::Result<()> {
    let status = cdj3k_emu_platform::child::command(cdj3k_emu_platform::bundled::tool("qemu-img"))
        .args(["create", "-f", "raw"])
        .arg(img_path)
        .arg(size_bytes.to_string())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "qemu-img create failed (exit {:?})",
            status.code()
        )))
    }
}

// ── UsbManager ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum UsbError {
    Qmp(QmpError),
    Io(std::io::Error),
    /// The host refused this process access to the block device.
    PermissionDenied(String),
}

impl From<QmpError> for UsbError {
    fn from(e: QmpError) -> Self {
        UsbError::Qmp(e)
    }
}

impl From<std::io::Error> for UsbError {
    fn from(e: std::io::Error) -> Self {
        UsbError::Io(e)
    }
}

impl std::fmt::Display for UsbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UsbError::Qmp(e) => write!(f, "QMP error: {e:?}"),
            UsbError::Io(e) => write!(f, "I/O error: {e}"),
            UsbError::PermissionDenied(dev) => {
                write!(f, "Permission denied opening {dev}. {}", imp::PERMISSION_HINT)
            }
        }
    }
}

enum AttachedMode {
    Virtual,
    Physical {
        bsd_name: String,
        /// The fdset QEMU received the device through, released on eject.
        fdset: Option<u64>,
    },
}

/// A raw device ready for QEMU: the name it opens, the format to open it
/// with, and the fdset behind it when the device was handed over as a
/// descriptor.
pub(crate) struct RawMedium {
    filename: String,
    format: Option<&'static str>,
    fdset: Option<u64>,
}

struct ActiveUsb {
    mode: AttachedMode,
}

/// Manages USB hot-plug for one QEMU instance.
///
/// The USB virtio-blk slot (id=usb0 → /dev/sdb in guest) is always present in
/// QEMU's argv, backed by a 1-sector placeholder at boot.  Attach/detach hot-swap
/// the medium via QMP blockdev-change-medium, then send a command line to the
/// guest over the `cdj3k.cfg` virtio-serial port (via `CfgClient`) so the
/// in-guest `cdj3k-cfgd` runs the attach script - no SSH / network forwarding
/// required.
pub struct UsbManager {
    placeholder_path: PathBuf,
    qmp_fd_socket: PathBuf,
    cfg: crate::CfgClient,
    current: Option<ActiveUsb>,
}

impl UsbManager {
    pub fn new(config: &crate::QemuConfig, cfg: crate::CfgClient) -> Self {
        Self {
            placeholder_path: config.usb_placeholder_path(),
            qmp_fd_socket: config.qmp_fd_socket(),
            cfg,
            current: None,
        }
    }

    /// Create an exFAT-formatted image at `img_path` then attach it as a virtual USB drive.
    pub fn create_and_attach_virtual(
        &mut self,
        qmp: &mut QmpClient,
        provider: &dyn DiskProvider,
        img_path: &Path,
        size_bytes: u64,
    ) -> Result<(), UsbError> {
        imp::create_image(img_path, size_bytes)?;
        self.attach_virtual(qmp, provider, img_path)
    }

    /// Hot-swap the USB slot to a virtual image file and notify the guest.
    /// Auto-ejects whatever was previously attached.
    pub fn attach_virtual(
        &mut self,
        qmp: &mut QmpClient,
        provider: &dyn DiskProvider,
        img_path: &Path,
    ) -> Result<(), UsbError> {
        self.detach(qmp, provider)?;
        let path_str = img_path.to_string_lossy().into_owned();
        qmp.blockdev_change_medium(USB_DRIVE_ID, &path_str, Some(imp::virtual_format(img_path)))?;
        self.guest_usb_attach()?;
        self.current = Some(ActiveUsb {
            mode: AttachedMode::Virtual,
        });
        Ok(())
    }

    /// Unmount the host's volumes on `disk`, hot-swap the USB slot to the raw
    /// device, and notify the guest.  Auto-ejects whatever was previously
    /// attached.
    pub fn attach_physical(
        &mut self,
        qmp: &mut QmpClient,
        disk: &PhysicalDisk,
        provider: &dyn DiskProvider,
    ) -> Result<(), UsbError> {
        self.detach(qmp, provider)?;
        // Unmount first so the device is free: desktops auto-mount removable
        // media, and a mounted disk cannot be opened for the guest.
        provider.unmount_host(&disk.bsd_name).map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                UsbError::PermissionDenied(disk.bsd_path.clone())
            } else {
                UsbError::Io(e)
            }
        })?;

        let medium = match imp::open_raw(disk, &self.qmp_fd_socket) {
            Ok(m) => m,
            Err(e) => {
                let _ = provider.remount_host(&disk.bsd_name);
                return Err(e);
            }
        };
        if let Err(e) = qmp.blockdev_change_medium(USB_DRIVE_ID, &medium.filename, medium.format) {
            if let Some(id) = medium.fdset {
                let _ = qmp.remove_fd(id);
            }
            let _ = provider.remount_host(&disk.bsd_name);
            return Err(match e {
                QmpError::QemuError(msg) if msg.contains("Permission denied") => {
                    UsbError::PermissionDenied(disk.bsd_path.clone())
                }
                e => UsbError::Qmp(e),
            });
        }
        self.guest_usb_attach()?;
        self.current = Some(ActiveUsb {
            mode: AttachedMode::Physical {
                bsd_name: disk.bsd_name.clone(),
                fdset: medium.fdset,
            },
        });
        Ok(())
    }

    /// Host-initiated eject: tell the guest to release FDs + umount, then
    /// swap the USB slot back to placeholder and remount the host disk if
    /// we had attached a physical one.  No-op when nothing is attached, so
    /// this doubles as the "auto-eject" hook at the top of every attach
    /// path.
    ///
    /// The guest-side hook (cfgd's `usb detach`) writes `umount /dev/sdb` to
    /// `/proc/udev_usb1` so EP122 closes its filesystem handles, then lazy-
    /// unmounts `/media/usb/sd*`.  We sleep briefly to let that complete
    /// before yanking the medium - swapping while EP122 still holds an open
    /// FD on the FS would produce I/O errors on the host disk.
    pub fn detach(
        &mut self,
        qmp: &mut QmpClient,
        provider: &dyn DiskProvider,
    ) -> Result<(), UsbError> {
        if self.current.is_none() {
            return Ok(());
        }
        let _ = self.cfg.usb_detach();
        // cfgd's detach handler does 150 ms sleep + umount -l; 400 ms covers
        // that plus a comfortable margin for the EP122 FD-close latency.
        std::thread::sleep(Duration::from_millis(400));
        self.host_side_eject(qmp, provider)
    }

    /// Guest-initiated eject: EP122's `unbind-usb-device.sh` already ran,
    /// the guest has unmounted everything, and `usb_state 0` arrived on
    /// cdj3k.cfg.  We just mirror the state on the host side - swap the
    /// medium back to placeholder, remount any physical disk - skipping
    /// the cfg round-trip so we don't trigger a feedback loop.
    pub fn acknowledge_guest_eject(
        &mut self,
        qmp: &mut QmpClient,
        provider: &dyn DiskProvider,
    ) -> Result<bool, UsbError> {
        if self.current.is_none() {
            return Ok(false);
        }
        self.host_side_eject(qmp, provider)?;
        Ok(true)
    }

    /// Common host-side cleanup shared by `detach` and `acknowledge_guest_eject`.
    /// Assumes `self.current` is Some; consumes it.
    fn host_side_eject(
        &mut self,
        qmp: &mut QmpClient,
        provider: &dyn DiskProvider,
    ) -> Result<(), UsbError> {
        let active = self.current.take().expect("caller checked is_some()");
        let placeholder = self.placeholder_path.to_string_lossy().into_owned();
        qmp.blockdev_change_medium(USB_DRIVE_ID, &placeholder, Some("raw"))?;
        if let AttachedMode::Physical { bsd_name, fdset } = active.mode {
            // The fdset's descriptor holds the exclusive claim: the host
            // cannot mount the disk until it is closed.
            if let Some(id) = fdset {
                let _ = qmp.remove_fd(id);
            }
            let _ = provider.remount_host(&bsd_name);
        }
        Ok(())
    }

    /// Wait for virtblk_config_changed to settle, then ask the guest agent to
    /// run usb-external-attach.sh (which mounts the device and writes to
    /// /proc/udev_usb1 - the interface EP122 actually watches for USB state).
    fn guest_usb_attach(&self) -> std::io::Result<()> {
        // Give the guest driver time to run virtblk_config_changed_work.
        // virtio-blk revalidates the partition table automatically on config change;
        // no manual blockdev --rereadpt needed (and it corrupts the FAT32 device state).
        std::thread::sleep(Duration::from_millis(600));
        self.cfg.usb_attach()
    }
}
