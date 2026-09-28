//! macOS adapter: DiskArbitration, and the admin prompt that opens a device
//! for writing.

use super::{DiskProvider, HostDiskProvider, PhysicalDisk, RawMedium, UsbError};
use std::path::Path;
use std::process::Command;

/// Where to grant this process access to a raw block device.
pub const PERMISSION_HINT: &str = "Grant Full Disk Access to cdj3k-emu in \
     System Settings → Privacy & Security → Full Disk Access.";

pub const PASSES_FDS: bool = false;

pub const RETRY_PROMPT: &str = "cdj3k-emu needs write access to the raw disk device.\n\n\
     Click Retry to ask for an administrator password and grant \
     temporary write permission. \
     This resets automatically when the drive is unplugged.";

impl DiskProvider for HostDiskProvider {
    fn list_removable(&self) -> Vec<PhysicalDisk> {
        super::macos_disk::list_removable()
    }

    fn unmount_host(&self, bsd_name: &str) -> std::io::Result<()> {
        super::macos_disk::unmount_disk(bsd_name)
    }

    fn remount_host(&self, bsd_name: &str) -> std::io::Result<()> {
        super::macos_disk::mount_disk(bsd_name)
    }
}

/// Check that this process can open the device read-write, as QEMU will, and
/// ask for access once if it cannot. QEMU opens the node by path.
pub(super) fn open_raw(disk: &PhysicalDisk, _qmp_fd_socket: &Path) -> Result<RawMedium, UsbError> {
    let probe = || {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&disk.bsd_path)
    };
    let denied = |e: &std::io::Error| e.kind() == std::io::ErrorKind::PermissionDenied;
    if let Err(e) = probe() {
        if !denied(&e) {
            return Err(UsbError::Io(e));
        }
        unlock_device_write(&disk.bsd_path)
            .map_err(|_| UsbError::PermissionDenied(disk.bsd_path.clone()))?;
        probe().map_err(|e| {
            if denied(&e) {
                UsbError::PermissionDenied(disk.bsd_path.clone())
            } else {
                UsbError::Io(e)
            }
        })?;
    }
    Ok(RawMedium {
        filename: disk.bsd_path.clone(),
        format: Some("raw"),
        fdset: None,
    })
}

/// Show a macOS admin-password dialog and run `chmod 660 <dev>` as root.
/// Returns Ok if the device is now group-writable, Err if the user cancelled
/// or the operation failed.
///
/// `dev_path` is interpolated into an AppleScript string, so it must match the
/// BSD disk shape (`/dev/disk<N>` or `/dev/disk<N>s<M>`) and nothing else.
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

/// Format a raw image file as MBR + exFAT using macOS hdiutil + diskutil.
///
/// Uses `hdiutil attach -nomount` to expose the file as a block device, then
/// `diskutil eraseDisk` to write an MBR partition table with one exFAT partition
/// (label REKORDBOX).  The block device is detached before returning.
///
/// MBR partition table: guest sees /dev/sdb1 (exFAT), which blkid correctly
/// identifies and the attach script mounts.
pub fn format_exfat(img_path: &Path) -> std::io::Result<()> {
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
        return Err(std::io::Error::other(format!(
            "hdiutil attach failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }

    let disk = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();

    if disk.is_empty() {
        return Err(std::io::Error::other(
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
        Ok(s) => Err(std::io::Error::other(format!(
            "diskutil eraseDisk failed (exit {:?})",
            s.code()
        ))),
        Err(e) => Err(e),
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
