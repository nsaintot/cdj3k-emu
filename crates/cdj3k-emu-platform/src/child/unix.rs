use std::process::Command;

/// A Unix child opens no window unless it asks for one.
pub fn quiet(_command: &mut Command) {}

/// Where a desktop session's `PATH` misses system tools: GNOME hands an app
/// `/usr/local/bin:/usr/bin:/bin`, without `sfdisk`, `losetup` or `mkfs.*`.
pub const EXTRA_TOOL_DIRS: &[&str] = &["/usr/sbin", "/sbin", "/usr/local/sbin"];

pub fn is_runnable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

pub fn set_runnable(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(perms.mode() | 0o111);
    std::fs::set_permissions(path, perms)
}
