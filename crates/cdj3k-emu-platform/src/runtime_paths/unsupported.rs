//! Hosts with no runtime-root layout of their own.

use std::io;
use std::path::{Path, PathBuf};

/// A host without AF_UNIX places no limit on socket paths.
pub const SUN_PATH_LIMIT: usize = usize::MAX;

/// A `cdj3k-emu` directory in `temp_dir()`, without the per-user isolation
/// of the Unix and Windows roots.
pub fn base_dir() -> PathBuf {
    std::env::temp_dir().join("cdj3k-emu")
}

/// Nothing to tighten: this arm has no per-user isolation to establish.
pub fn tighten(_base: &Path) -> io::Result<()> {
    Ok(())
}
