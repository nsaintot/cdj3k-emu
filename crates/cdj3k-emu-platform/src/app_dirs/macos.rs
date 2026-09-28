//! `~/Library/Application Support`.

use std::path::PathBuf;

use super::env_dir;

/// Falls back to the temporary directory, which does not survive a reboot,
/// for a process with no home.
pub fn base_dir() -> PathBuf {
    env_dir("HOME")
        .unwrap_or_else(std::env::temp_dir)
        .join("Library")
        .join("Application Support")
}
