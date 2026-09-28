//! A host with no file locking: every claim is refused, since two guests on
//! one eMMC image corrupt it.

use std::io;
use std::path::Path;

pub fn lock(_file: &std::fs::File, path: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!(
            "this host cannot lock files, so {} cannot be claimed exclusively",
            path.display()
        ),
    ))
}
