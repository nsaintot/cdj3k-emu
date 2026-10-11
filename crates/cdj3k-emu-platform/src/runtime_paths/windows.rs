//! The Windows runtime root: `%TEMP%\cdj3k-emu`.

use std::io;
use std::path::{Path, PathBuf};

/// Windows AF_UNIX allows a longer path than the BSD `sun_path`.
pub const SUN_PATH_LIMIT: usize = 108;

/// `%TEMP%\cdj3k-emu`, which is per-user by ACL.
pub fn base_dir() -> PathBuf {
    std::env::temp_dir().join("cdj3k-emu")
}

/// Nothing to tighten: the root inherits the user's ACL from `%TEMP%`.
pub fn tighten(_base: &Path) -> io::Result<()> {
    Ok(())
}
