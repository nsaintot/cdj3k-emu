//! The Unix runtime root: `/tmp/cdj3k-emu-<euid>`, mode 0700.

use std::io;
use std::path::{Path, PathBuf};

/// The macOS `sun_path` size, including the NUL. Linux allows 108, so this
/// limit holds on both.
pub const SUN_PATH_LIMIT: usize = 104;

/// `/tmp/cdj3k-emu-<euid>`.
///
/// `/tmp` is world-writable: the euid suffix separates users' namespaces, and
/// [`tighten`] makes the root 0700, since the suffix alone is guessable.
pub fn base_dir() -> PathBuf {
    PathBuf::from(format!("/tmp/cdj3k-emu-{}", euid()))
}

fn euid() -> u32 {
    // SAFETY: `geteuid()` has no preconditions and cannot fail.
    unsafe { libc::geteuid() as u32 }
}

/// Set the root to 0700.
///
/// This runs on every call, so an existing root made under a looser umask is
/// tightened too.
pub fn tighten(base: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(base, std::fs::Permissions::from_mode(0o700))
}
