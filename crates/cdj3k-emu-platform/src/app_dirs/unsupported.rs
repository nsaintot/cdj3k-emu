//! Hosts with no app-data convention.
//!
//! The temporary directory, so settings last until the next reboot.

use std::path::PathBuf;

pub fn base_dir() -> PathBuf {
    std::env::temp_dir()
}
