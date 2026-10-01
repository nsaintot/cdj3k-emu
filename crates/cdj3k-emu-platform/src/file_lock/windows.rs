//! A `LockFileEx` byte-range lock, which Windows drops when the last handle to
//! the file closes.
//!
//! The range sits past the end of the image: Windows enforces the lock on
//! every other handle's I/O, and QEMU opens the same file. Locking past the
//! end of a file is allowed.

use std::io;
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Storage::FileSystem::{
    LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
};
use windows_sys::Win32::System::IO::OVERLAPPED;

/// Where the one-byte claim sits, in bytes from the start of the file.
const CLAIM_OFFSET: u64 = 1 << 62;

pub fn lock(file: &std::fs::File, path: &Path) -> io::Result<()> {
    // SAFETY: an all-zero OVERLAPPED is valid; only the offset is read.
    let mut at: OVERLAPPED = unsafe { std::mem::zeroed() };
    at.Anonymous.Anonymous.Offset = CLAIM_OFFSET as u32;
    at.Anonymous.Anonymous.OffsetHigh = (CLAIM_OFFSET >> 32) as u32;
    // SAFETY: `file` holds an open handle for the duration of the call, and
    // `at` outlives it.
    let ok = unsafe {
        LockFileEx(
            file.as_raw_handle() as HANDLE,
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            1,
            0,
            &mut at,
        )
    };
    if ok == 0 {
        // The caller retries only `WouldBlock`.
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            format!("{} is in use", path.display()),
        ));
    }
    Ok(())
}
