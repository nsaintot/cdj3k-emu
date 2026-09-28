//! Where this app keeps what outlives a run: settings, firmware images, the
//! eMMC.
//!
//! [`crate::runtime_paths`] holds per-run scratch (sockets, shm, marker
//! files); this root holds per-install state that must survive a reboot.
//!
//! Every arm ends in the bundle id, so a slot's directory has the same name on
//! every host.

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod imp;
#[cfg(windows)]
#[path = "windows.rs"]
mod imp;
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
#[path = "unsupported.rs"]
mod imp;

use std::path::PathBuf;

/// A directory named by the environment, ignoring an empty value.
///
/// An empty value would join into a relative path under the working
/// directory.
pub(crate) fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// This host's app-data directory, ending in
/// [`BUNDLE_ID`](crate::app_meta::BUNDLE_ID).
///
/// Not created here; callers that write [`create_dir_all`] first.
///
/// [`create_dir_all`]: std::fs::create_dir_all
pub fn app_data_dir() -> PathBuf {
    imp::base_dir().join(crate::app_meta::BUNDLE_ID)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_meta::BUNDLE_ID;

    /// Every host arm ends in the bundle id.
    #[test]
    fn every_host_names_the_directory_after_the_app() {
        assert_eq!(
            app_data_dir().file_name().unwrap().to_string_lossy(),
            BUNDLE_ID
        );
    }

    /// The directory is absolute, never relative to the working directory.
    #[test]
    fn the_directory_is_absolute() {
        assert!(
            app_data_dir().is_absolute(),
            "{:?} is not absolute",
            app_data_dir()
        );
    }
}
