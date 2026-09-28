//! `$XDG_DATA_HOME`, else `~/.local/share`, per the XDG base directory spec.

use std::path::PathBuf;

use super::env_dir;

/// Falls back to the temporary directory, which does not survive a reboot,
/// for a process with neither variable.
pub fn base_dir() -> PathBuf {
    env_dir("XDG_DATA_HOME")
        .or_else(|| env_dir("HOME").map(|home| home.join(".local/share")))
        .unwrap_or_else(std::env::temp_dir)
}
