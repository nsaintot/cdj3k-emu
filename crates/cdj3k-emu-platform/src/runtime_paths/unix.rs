//! The Unix runtime root: `/tmp/cdj3k-emu-<euid>`, mode 0700.

use std::io;
use std::path::{Path, PathBuf};

/// The BSD `sun_path` field, including the NUL, which is the limit that
/// actually truncates a socket path.
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
/// Applied on every call, not only at creation, so an existing root made
/// under a looser umask is tightened too.
pub fn tighten(base: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(base, std::fs::Permissions::from_mode(0o700))
}
