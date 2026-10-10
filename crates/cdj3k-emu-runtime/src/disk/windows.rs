//! The Windows adapter. It passes whole disks through as
//! `\\.\PhysicalDriveN`, and uses VHDX images for virtual sticks.
//!
//! QEMU opens the drive by path, which needs rights that a standard user does
//! not have on it. When the open is refused, the app asks once, through the
//! elevation prompt, for a grant on the disk and its volumes that lasts until
//! the disk is unplugged. The app then keeps every volume of the disk locked
//! and dismounted while the guest has it.

use super::windows_disk;
use super::windows_parse::is_vhdx;
use super::{DiskProvider, HostDiskProvider, PhysicalDisk, RawMedium, UsbError};
use std::io::Read;
use std::path::Path;

/// The hint shown when Windows refuses to open the raw device.
pub const PERMISSION_HINT: &str = "Windows did not grant access to the disk.";

pub const PASSES_FDS: bool = false;

pub const RETRY_PROMPT: &str = "cdj3k-emu needs write access to the raw disk device.\n\n\
     Click Retry to ask for administrator approval. \
     Access lasts until the disk is unplugged.";

impl DiskProvider for HostDiskProvider {
    fn list_removable(&self) -> Vec<PhysicalDisk> {
        windows_disk::list_removable()
    }

    fn unmount_host(&self, name: &str) -> std::io::Result<()> {
        windows_disk::unmount_disk(name)
    }

    fn remount_host(&self, name: &str) -> std::io::Result<()> {
        windows_disk::mount_disk(name)
    }
}

/// Check that this process can open the device read-write, as QEMU will, and
/// give QEMU the device name with an explicit driver chain.
///
/// `unmount_host` has already locked and dismounted the volumes, and asked for
/// administrator approval if it was needed.
pub(super) fn open_raw(disk: &PhysicalDisk, _qmp_fd_socket: &Path) -> Result<RawMedium, UsbError> {
    if let Err(e) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&disk.bsd_path)
    {
        return Err(UsbError::opening(&disk.bsd_path, e));
    }
    let chain = serde_json::json!({
        "driver": "raw",
        "file": { "driver": "host_device", "filename": disk.bsd_path },
    });
    Ok(RawMedium {
        filename: format!("json:{chain}"),
        format: None,
        fdset: None,
    })
}

/// Return `vhdx` for a VHDX file and `raw` for anything else, so that an
/// image made on another host still attaches.
pub fn virtual_format(img_path: &Path) -> &'static str {
    let mut head = [0u8; 8];
    let signed = std::fs::File::open(img_path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && is_vhdx(&head);
    if signed {
        "vhdx"
    } else {
        "raw"
    }
}

/// Create a dynamic VHDX of `size_bytes` with an MBR partition table and one
/// exFAT partition labelled REKORDBOX, the layout that the other hosts give a
/// raw image.
pub fn create_image(img_path: &Path, size_bytes: u64) -> std::io::Result<()> {
    match std::fs::remove_file(img_path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    windows_disk::create_vhdx(img_path, size_bytes)?;
    windows_disk::format_vhdx(img_path).inspect_err(|_| {
        let _ = std::fs::remove_file(img_path);
    })
}
