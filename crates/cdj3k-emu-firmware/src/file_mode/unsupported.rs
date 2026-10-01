use std::io;
use std::path::Path;

/// No mode bits to set.
pub fn set_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}
