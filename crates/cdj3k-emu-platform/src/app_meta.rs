//! The app's name, version and identifiers, defined once for every crate.
//!
//! `BUNDLE_ID` must match `CFBundleIdentifier` in `packaging/macos/stage.sh`,
//! and every host names the per-user data directory after it
//! ([`crate::app_dirs`]).

/// Display name used as the eframe app identifier and the main window title.
/// It is the app name shown in the macOS Dock and the Cmd-Tab switcher.
pub const APP_DISPLAY_NAME: &str = "CDJ3K Emulator";

/// The build: the release's version when CD sets `CDJ3K_VERSION` from the
/// tag, the workspace version otherwise.
pub const VERSION: &str = match option_env!("CDJ3K_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

/// Window compositor identifier.
pub const APP_ID: &str = "cdj3k-emu";

/// The macOS bundle identifier, which must match `CFBundleIdentifier` in
/// `packaging/macos/stage.sh`. Every host uses it as the name of the per-user
/// data directory.
pub const BUNDLE_ID: &str = "com.cdj3k.emu";
