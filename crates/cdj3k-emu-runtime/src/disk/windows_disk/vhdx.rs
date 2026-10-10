//! VHDX images for virtual sticks.

use std::io;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Storage::Vhd::{
    CreateVirtualDisk, CREATE_VIRTUAL_DISK_FLAG_NONE, CREATE_VIRTUAL_DISK_PARAMETERS,
    CREATE_VIRTUAL_DISK_VERSION_2, VIRTUAL_DISK_ACCESS_NONE, VIRTUAL_STORAGE_TYPE,
    VIRTUAL_STORAGE_TYPE_DEVICE_VHDX, VIRTUAL_STORAGE_TYPE_VENDOR_MICROSOFT,
};

use super::grant::{path_str, run_script};
use super::wide_os;
use crate::disk::windows_parse as parse;

/// Create an empty dynamic VHDX of `size_bytes`, rounded down to a whole MiB.
///
/// This process creates the file so that it belongs to the user. Only
/// attaching it for partitioning needs administrator rights.
pub fn create_vhdx(path: &Path, size_bytes: u64) -> io::Result<()> {
    const MIB: u64 = 1 << 20;
    let size = size_bytes / MIB * MIB;
    if size < 2 * MIB {
        return Err(io::Error::other(format!(
            "{} is too small to hold a partition",
            path.display()
        )));
    }

    let storage_type = VIRTUAL_STORAGE_TYPE {
        DeviceId: VIRTUAL_STORAGE_TYPE_DEVICE_VHDX,
        VendorId: VIRTUAL_STORAGE_TYPE_VENDOR_MICROSOFT,
    };
    // SAFETY: the parameters are plain data; zero means "default" throughout.
    let mut params: CREATE_VIRTUAL_DISK_PARAMETERS = unsafe { std::mem::zeroed() };
    params.Version = CREATE_VIRTUAL_DISK_VERSION_2;
    params.Anonymous.Version2.MaximumSize = size;
    params.Anonymous.Version2.SectorSizeInBytes = 512;
    params.Anonymous.Version2.PhysicalSectorSizeInBytes = 512;
    let path_w = wide_os(path);
    let mut handle: HANDLE = std::ptr::null_mut();
    // SAFETY: every pointer is live for the call; `handle` receives the disk.
    let rc = unsafe {
        CreateVirtualDisk(
            &storage_type,
            path_w.as_ptr(),
            VIRTUAL_DISK_ACCESS_NONE,
            std::ptr::null_mut(),
            CREATE_VIRTUAL_DISK_FLAG_NONE,
            0,
            &params,
            std::ptr::null(),
            &mut handle,
        )
    };
    if rc != 0 {
        return Err(io::Error::from_raw_os_error(rc as i32));
    }
    // SAFETY: `handle` is the valid handle CreateVirtualDisk returned.
    drop(unsafe { OwnedHandle::from_raw_handle(handle) });
    Ok(())
}

/// Attach the VHDX, write an MBR and an exFAT partition, and detach it.
pub fn format_vhdx(path: &Path) -> io::Result<()> {
    let mut err_file = path.as_os_str().to_owned();
    err_file.push(".err");
    let err_file = PathBuf::from(err_file);
    run_script(
        &parse::format_script(&path_str(path)?, &path_str(&err_file)?),
        &err_file,
    )
}
