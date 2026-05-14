pub mod emmc;
pub mod gpt;
pub mod settings;

pub use emmc::{default_path, provision_emmc, EmmcConfig, FirmwareInfo};
pub use settings::{AppSettings, InstanceSettings};

use std::path::PathBuf;

/// `~/Library/Application Support/<BUNDLE_ID>/` on macOS,
/// `$XDG_DATA_HOME/<BUNDLE_ID>/` (fallback: `~/.local/share/<BUNDLE_ID>/`)
/// elsewhere. The directory name is the macOS bundle identifier, matching
/// the OS convention (reverse-DNS, never the human display name).
pub fn app_data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    let base = PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join("Library")
        .join("Application Support");
    #[cfg(not(target_os = "macos"))]
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share")
        });
    base.join(cdj3k_emu_platform::app_meta::BUNDLE_ID)
}
