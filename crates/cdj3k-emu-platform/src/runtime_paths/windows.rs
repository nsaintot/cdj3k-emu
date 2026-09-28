//! The Windows runtime root: `%LOCALAPPDATA%\cdj3k-emu`.

use std::io;
use std::path::{Path, PathBuf};

/// Windows AF_UNIX allows a longer path than the BSD `sun_path`.
pub const SUN_PATH_LIMIT: usize = 108;

/// `%LOCALAPPDATA%\cdj3k-emu`.
///
/// No per-user suffix: `%LOCALAPPDATA%` is per-user by ACL.
///
/// Not `%TEMP%`: Disk Cleanup and Storage Sense delete from it, which would
/// remove the sockets from under a running QEMU.
pub fn base_dir() -> PathBuf {
    // A service account can have no profile; `temp_dir()` is still per-user
    // there.
    crate::app_dirs::env_dir("LOCALAPPDATA")
        .unwrap_or_else(std::env::temp_dir)
        .join("cdj3k-emu")
}

/// Nothing to tighten: the root inherits the user's ACL by living under
/// `%LOCALAPPDATA%`.
pub fn tighten(_base: &Path) -> io::Result<()> {
    Ok(())
}
