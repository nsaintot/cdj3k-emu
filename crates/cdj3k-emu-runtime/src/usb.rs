use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::qmp::{QmpClient, QmpError};

const USB_DRIVE_ID: &str = "usb0";

// ── Domain types ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct PhysicalDisk {
    pub bsd_name: String, // "disk2"
    pub label: String,    // "PIONEER (32.1 GB)" - for display
    pub bsd_path: String, // "/dev/disk2"
    pub size_bytes: u64,
}

// ── DiskProvider port (hexagonal) ────────────────────────────────────────────

pub trait DiskProvider: Send + Sync {
    fn list_removable(&self) -> Vec<PhysicalDisk>;
    fn unmount_host(&self, bsd_name: &str) -> std::io::Result<()>;
    fn remount_host(&self, bsd_name: &str) -> std::io::Result<()>;
    fn reveal_in_finder(&self, path: &str);
}

// ── macOS adapter ────────────────────────────────────────────────────────────

pub struct MacOsDiskProvider;

#[cfg(target_os = "macos")]
impl DiskProvider for MacOsDiskProvider {
    fn list_removable(&self) -> Vec<PhysicalDisk> {
        crate::macos_disk::list_removable()
    }

    fn unmount_host(&self, bsd_name: &str) -> std::io::Result<()> {
        crate::macos_disk::unmount_disk(bsd_name)
    }

    fn remount_host(&self, bsd_name: &str) -> std::io::Result<()> {
        crate::macos_disk::mount_disk(bsd_name)
    }

    fn reveal_in_finder(&self, path: &str) {
        let _ = Command::new("open").args(["-R", path]).spawn();
    }
}

#[cfg(not(target_os = "macos"))]
impl DiskProvider for MacOsDiskProvider {
    fn list_removable(&self) -> Vec<PhysicalDisk> {
        Vec::new()
    }
    fn unmount_host(&self, _bsd_name: &str) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "MacOsDiskProvider only implemented on macOS",
        ))
    }
    fn remount_host(&self, _bsd_name: &str) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "MacOsDiskProvider only implemented on macOS",
        ))
    }
    fn reveal_in_finder(&self, _path: &str) {}
}

/// Show a macOS admin-password dialog and run `chmod 660 <dev>` as root.
/// Returns Ok if the device is now group-writable, Err if the user cancelled
/// or the operation failed.
///
/// `dev_path` is interpolated into an AppleScript `do shell script` string -
/// AppleScript uses *double* quotes, which `sh_quote`'s single-quote escaping
/// would not protect.  We instead validate the path matches the macOS BSD
/// disk-device shape (`/dev/disk<N>` or `/dev/disk<N>s<M>`) and reject
/// anything else.  In practice `bsd_name` is fed from `diskutil list` so the
/// shape is already constrained; this is defence-in-depth in case the parser
/// or the OS ever surfaces something unexpected.
#[cfg(target_os = "macos")]
fn unlock_device_write(dev_path: &str) -> std::io::Result<()> {
    if !is_valid_bsd_disk_path(dev_path) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("refusing to elevate on suspicious device path: {dev_path}"),
        ));
    }
    let script = format!("do shell script \"chmod 660 {dev_path}\" with administrator privileges");
    let out = Command::new("osascript").args(["-e", &script]).output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

/// `/dev/disk<digits>` optionally followed by `s<digits>`. Matches every BSD
/// disk path macOS emits and rejects everything else (no spaces, no quotes,
/// no `;`, no `..`).
#[cfg(target_os = "macos")]
fn is_valid_bsd_disk_path(p: &str) -> bool {
    let Some(rest) = p.strip_prefix("/dev/disk") else {
        return false;
    };
    let mut iter = rest.split('s');
    let Some(major) = iter.next() else {
        return false;
    };
    if major.is_empty() || !major.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    match iter.next() {
        None => true,
        Some(minor) => {
            iter.next().is_none() && !minor.is_empty() && minor.bytes().all(|b| b.is_ascii_digit())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_valid_bsd_disk_path;

    #[test]
    fn accepts_real_bsd_paths() {
        assert!(is_valid_bsd_disk_path("/dev/disk2"));
        assert!(is_valid_bsd_disk_path("/dev/disk2s1"));
        assert!(is_valid_bsd_disk_path("/dev/disk20s10"));
    }

    #[test]
    fn rejects_injection_attempts() {
        assert!(!is_valid_bsd_disk_path("/dev/disk2\" ; rm -rf /"));
        assert!(!is_valid_bsd_disk_path("/dev/disk2; reboot"));
        assert!(!is_valid_bsd_disk_path("/dev/disk2 s1"));
        assert!(!is_valid_bsd_disk_path("/dev/disk"));
        assert!(!is_valid_bsd_disk_path("/dev/diskX"));
        assert!(!is_valid_bsd_disk_path("disk2"));
        assert!(!is_valid_bsd_disk_path("/dev/disk2s"));
        assert!(!is_valid_bsd_disk_path("/dev/disk2s1s2"));
        assert!(!is_valid_bsd_disk_path("/dev/../disk2"));
    }
}

#[cfg(not(target_os = "macos"))]
fn unlock_device_write(_dev_path: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "unlock_device_write not implemented on this platform",
    ))
}

// ── UsbManager ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum UsbError {
    Qmp(QmpError),
    Io(std::io::Error),
    /// QEMU returned EACCES opening the block device - FDA not granted.
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
            UsbError::PermissionDenied(dev) => write!(
                f,
                "Permission denied opening {dev}. \
                 Grant Full Disk Access to cdj3k-emu in \
                 System Settings → Privacy & Security → Full Disk Access."
            ),
        }
    }
}

enum AttachedMode {
    Virtual,
    Physical { bsd_name: String },
}

struct ActiveUsb {
    mode: AttachedMode,
}

/// Format a raw image file as MBR + exFAT using macOS hdiutil + diskutil.
///
/// Uses `hdiutil attach -nomount` to expose the file as a block device, then
/// `diskutil eraseDisk` to write an MBR partition table with one exFAT partition
/// (label REKORDBOX).  The block device is detached before returning.
///
/// MBR partition table: guest sees /dev/sdb1 (exFAT), which blkid correctly
/// identifies and the attach script mounts.
#[cfg(target_os = "macos")]
fn format_exfat(img_path: &Path) -> std::io::Result<()> {
    // Attach image without mounting - get back a /dev/diskN device.
    let out = Command::new("hdiutil")
        .args([
            "attach",
            "-nomount",
            "-imagekey",
            "diskimage-class=CRawDiskImage",
        ])
        .arg(img_path)
        .output()?;

    if !out.status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!(
                "hdiutil attach failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        ));
    }

    let disk = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();

    if disk.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "hdiutil attach: could not parse disk device from output",
        ));
    }

    // Format: MBR partition table + single exFAT partition labelled REKORDBOX.
    let fmt = Command::new("diskutil")
        .args(["eraseDisk", "ExFAT", "REKORDBOX", "MBR", &disk])
        .status();

    // Always detach, even on format failure.
    let _ = Command::new("hdiutil").args(["detach", &disk]).status();

    match fmt {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("diskutil eraseDisk failed (exit {:?})", s.code()),
        )),
        Err(e) => Err(e),
    }
}

#[cfg(not(target_os = "macos"))]
fn format_exfat(img_path: &Path) -> std::io::Result<()> {
    let status = Command::new("mkfs.exfat")
        .args(["-L", "REKORDBOX"])
        .arg(img_path)
        .status()?;
    if !status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "mkfs.exfat failed",
        ));
    }
    Ok(())
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
    cfg: crate::CfgClient,
    current: Option<ActiveUsb>,
}

impl UsbManager {
    pub fn new(placeholder_path: PathBuf, cfg: crate::CfgClient) -> Self {
        Self {
            placeholder_path,
            cfg,
            current: None,
        }
    }

    pub fn is_attached(&self) -> bool {
        self.current.is_some()
    }

    /// Create an exFAT-formatted raw image at `img_path` then attach it as a virtual USB drive.
    pub fn create_and_attach_virtual(
        &mut self,
        qmp: &mut QmpClient,
        provider: &dyn DiskProvider,
        img_path: &Path,
        size_bytes: u64,
    ) -> Result<(), UsbError> {
        let status = Command::new(cdj3k_emu_platform::bundled::tool("qemu-img"))
            .args(["create", "-f", "raw"])
            .arg(img_path)
            .arg(size_bytes.to_string())
            .status()?;
        if !status.success() {
            return Err(UsbError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("qemu-img create failed (exit {:?})", status.code()),
            )));
        }
        format_exfat(img_path)?;
        self.attach_virtual(qmp, provider, img_path)
    }

    /// Hot-swap the USB slot to a virtual `.img` file and notify the guest.
    /// Auto-ejects whatever was previously attached.
    pub fn attach_virtual(
        &mut self,
        qmp: &mut QmpClient,
        provider: &dyn DiskProvider,
        img_path: &Path,
    ) -> Result<(), UsbError> {
        self.detach(qmp, provider)?;
        let path_str = img_path.to_string_lossy().into_owned();
        qmp.blockdev_change_medium(USB_DRIVE_ID, &path_str, "raw")?;
        self.guest_usb_attach()?;
        self.current = Some(ActiveUsb {
            mode: AttachedMode::Virtual,
        });
        Ok(())
    }

    /// Unmount macOS volumes on `disk`, hot-swap the USB slot to the raw device,
    /// and notify the guest.  Auto-ejects whatever was previously attached.
    pub fn attach_physical(
        &mut self,
        qmp: &mut QmpClient,
        disk: &PhysicalDisk,
        provider: &dyn DiskProvider,
    ) -> Result<(), UsbError> {
        self.detach(qmp, provider)?;
        // Unmount first so the device is free - macOS auto-remounts USB drives
        // after QEMU releases them, making any earlier open() return EBUSY.
        provider.unmount_host(&disk.bsd_name)?;

        // Probe O_RDWR from this process before handing the path to QEMU.
        // /dev/diskN is mode 0640: operator group has read, not write.
        // On EACCES, show a one-shot osascript admin dialog (chmod 660), then retry.
        // On any other error, remount the host volume and propagate.
        let probe = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&disk.bsd_path);
        if let Err(e) = probe {
            match e.raw_os_error() {
                Some(libc::EACCES) | Some(libc::EPERM) => {
                    if unlock_device_write(&disk.bsd_path).is_err() {
                        let _ = provider.remount_host(&disk.bsd_name);
                        return Err(UsbError::PermissionDenied(disk.bsd_path.clone()));
                    }
                    // Retry after chmod.
                    if let Err(e2) = std::fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(&disk.bsd_path)
                    {
                        let _ = provider.remount_host(&disk.bsd_name);
                        return Err(match e2.raw_os_error() {
                            Some(libc::EACCES) | Some(libc::EPERM) => {
                                UsbError::PermissionDenied(disk.bsd_path.clone())
                            }
                            _ => UsbError::Io(e2),
                        });
                    }
                }
                _ => {
                    let _ = provider.remount_host(&disk.bsd_name);
                    return Err(UsbError::Io(e));
                }
            }
        }

        if let Err(QmpError::QemuError(msg)) =
            qmp.blockdev_change_medium(USB_DRIVE_ID, &disk.bsd_path, "raw")
        {
            let _ = provider.remount_host(&disk.bsd_name);
            if msg.contains("Permission denied") {
                return Err(UsbError::PermissionDenied(disk.bsd_path.clone()));
            }
            return Err(UsbError::Qmp(QmpError::QemuError(msg)));
        }
        self.guest_usb_attach()?;
        self.current = Some(ActiveUsb {
            mode: AttachedMode::Physical {
                bsd_name: disk.bsd_name.clone(),
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
        qmp.blockdev_change_medium(USB_DRIVE_ID, &placeholder, "raw")?;
        if let AttachedMode::Physical { bsd_name } = active.mode {
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
