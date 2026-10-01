//! NTFS allocates the whole length of a file that grows with `set_len` unless
//! the file is marked sparse first (`FSCTL_SET_SPARSE`).

use std::io;
use std::os::windows::io::AsRawHandle;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Ioctl::FSCTL_SET_SPARSE;
use windows_sys::Win32::System::IO::DeviceIoControl;

/// Best effort: FAT and exFAT have no sparse files.
pub fn allow_holes(file: &std::fs::File) -> io::Result<()> {
    let mut returned = 0u32;
    // SAFETY: `file` holds an open handle for the duration of the call; no
    // buffers are passed, and `returned` outlives it.
    unsafe {
        DeviceIoControl(
            file.as_raw_handle() as HANDLE,
            FSCTL_SET_SPARSE,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        );
    }
    Ok(())
}
