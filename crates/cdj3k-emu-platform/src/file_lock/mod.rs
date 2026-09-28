//! An exclusive claim on an open file, which keeps two app windows off one
//! slot's eMMC.
//!
//! Every implementation is released by the OS when the handle closes, which
//! is what lets a slot stay claimable after a crash.

#[cfg(unix)]
#[path = "unix.rs"]
mod imp;
#[cfg(not(unix))]
#[path = "unsupported.rs"]
mod imp;

use std::io;
use std::path::Path;

/// Take the exclusive claim on the open `file`, failing with
/// [`WouldBlock`](io::ErrorKind::WouldBlock) when another open file holds it.
pub fn lock(file: &std::fs::File, path: &Path) -> io::Result<()> {
    imp::lock(file, path)
}
