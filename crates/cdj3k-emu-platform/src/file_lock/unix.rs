//! An `flock`, which the kernel drops when the last handle to the file closes.

use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;

pub fn lock(file: &std::fs::File, path: &Path) -> io::Result<()> {
    // SAFETY: `file` holds an open descriptor for the duration of the call.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        // Whatever `errno` says, the caller retries only `WouldBlock`.
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            format!("{} is in use", path.display()),
        ));
    }
    Ok(())
}
