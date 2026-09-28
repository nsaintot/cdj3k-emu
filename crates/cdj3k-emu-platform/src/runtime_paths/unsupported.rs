//! Hosts with no runtime-root layout of their own.

use std::io;
use std::path::{Path, PathBuf};

/// `temp_dir()`, without the per-user isolation of the Unix and Windows
/// roots.
pub const SUN_PATH_LIMIT: usize = usize::MAX;

/// The host's temporary directory, namespaced.
pub fn base_dir() -> PathBuf {
    std::env::temp_dir().join("cdj3k-emu")
}

/// Nothing to tighten: this arm has no per-user isolation to establish.
pub fn tighten(_base: &Path) -> io::Result<()> {
    Ok(())
}
