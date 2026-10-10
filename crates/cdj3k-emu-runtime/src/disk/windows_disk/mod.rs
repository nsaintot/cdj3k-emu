//! Removable-disk enumeration, volume locking, access grants and VHDX
//! creation on Windows.
//!
//! Disks are whole `\\.\PhysicalDriveN` devices, found by opening each number
//! with no access and asking the storage stack about it, which a standard user
//! may do. Only opening one for I/O needs rights, and the one elevated step
//! that grants them is [`grant::grant_access`].

mod enumerate;
mod grant;
mod holders;
mod passthrough;
mod vhdx;

pub use enumerate::list_removable;
pub use passthrough::{mount_disk, unmount_disk};
pub use vhdx::{create_vhdx, format_vhdx};

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const SHARE_RW: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_os(p: &Path) -> Vec<u16> {
    p.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn open(path: &str, access: u32) -> io::Result<OwnedHandle> {
    let path = wide(path);
    // SAFETY: `path` is NUL-terminated and outlives the call.
    let h = unsafe {
        CreateFileW(
            path.as_ptr(),
            access,
            SHARE_RW,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `h` is a valid handle this call just created.
    Ok(unsafe { OwnedHandle::from_raw_handle(h) })
}

/// Call `DeviceIoControl` and return the number of bytes written to `out`.
fn ioctl(h: &OwnedHandle, code: u32, input: &[u8], out: &mut [u8]) -> io::Result<usize> {
    let mut returned = 0u32;
    // SAFETY: both buffers are live for the call and their lengths are passed.
    let ok = unsafe {
        DeviceIoControl(
            h.as_raw_handle() as HANDLE,
            code,
            if input.is_empty() {
                std::ptr::null()
            } else {
                input.as_ptr().cast()
            },
            input.len() as u32,
            if out.is_empty() {
                std::ptr::null_mut()
            } else {
                out.as_mut_ptr().cast()
            },
            out.len() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(returned as usize)
    }
}
