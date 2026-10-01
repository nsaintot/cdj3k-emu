//! Hosts with no removable-disk back end and no image formatter.

use super::{DiskProvider, HostDiskProvider, PhysicalDisk, RawMedium, UsbError};
use std::path::Path;

pub const PERMISSION_HINT: &str = "";
pub const PASSES_FDS: bool = false;

pub const RETRY_PROMPT: &str = "";

impl DiskProvider for HostDiskProvider {
    fn list_removable(&self) -> Vec<PhysicalDisk> {
        Vec::new()
    }

    fn unmount_host(&self, _name: &str) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "no disk provider for this host",
        ))
    }

    fn remount_host(&self, _name: &str) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "no disk provider for this host",
        ))
    }
}

pub(super) fn open_raw(_disk: &PhysicalDisk, _qmp_fd_socket: &Path) -> Result<RawMedium, UsbError> {
    Err(UsbError::Io(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no raw-disk access on this host",
    )))
}

pub fn virtual_format(_img_path: &Path) -> &'static str {
    "raw"
}

pub fn create_image(_img_path: &Path, _size_bytes: u64) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no exFAT formatter for this host",
    ))
}
