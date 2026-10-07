use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// The permission bits of `meta`.
pub fn permissions(meta: &std::fs::Metadata) -> u32 {
    meta.permissions().mode() & 0o7777
}

pub fn set_executable(path: &Path) -> io::Result<()> {
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(perms.mode() | 0o111);
    std::fs::set_permissions(path, perms)
}
