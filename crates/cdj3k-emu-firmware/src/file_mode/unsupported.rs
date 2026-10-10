use std::io;
use std::path::Path;

/// The host has no mode bits, so every file reports 0o755.
pub fn permissions(_meta: &std::fs::Metadata) -> u32 {
    0o755
}

/// The host has no mode bits, so this does nothing.
pub fn set_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}
