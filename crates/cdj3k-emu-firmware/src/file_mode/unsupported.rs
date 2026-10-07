use std::io;
use std::path::Path;

/// No mode bits to read: everything is readable and executable.
pub fn permissions(_meta: &std::fs::Metadata) -> u32 {
    0o755
}

/// No mode bits to set.
pub fn set_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}
