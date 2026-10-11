//! `%LOCALAPPDATA%`.
//!
//! The directory is in the local profile because firmware images and eMMC
//! files are hundreds of megabytes and belong to one machine. A roaming
//! profile would sync them to every domain machine the user logs into.
//!
//! Falls back to the temporary directory, which does not survive a reboot,
//! for a service account with no profile.

use std::path::PathBuf;

use super::env_dir;

pub fn base_dir() -> PathBuf {
    env_dir("LOCALAPPDATA").unwrap_or_else(std::env::temp_dir)
}
