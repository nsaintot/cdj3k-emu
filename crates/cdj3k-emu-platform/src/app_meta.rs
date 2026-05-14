//! Single source of truth for human-facing app identity strings.
//!
//! `BUNDLE_ID` must stay in sync with `bundle.sh` (`CFBundleIdentifier`) —
//! it is the canonical macOS app-data directory name under
//! `~/Library/Application Support/<BUNDLE_ID>/` (see `cdj3k-emu-storage`).

/// Display name used as the eframe app identifier and the main window title.
/// Reads as the app name in macOS Dock / Cmd-Tab.
pub const APP_DISPLAY_NAME: &str = "CDJ3K Emulator";

/// Hardware product the emulator targets — reused in pop-out window titles
/// and as a chassis chrome label.
pub const DEVICE_NAME: &str = "CDJ-3000";

/// macOS bundle identifier - mirrors `CFBundleIdentifier` in `bundle.sh`.
/// Used as the per-user data directory name under `Application Support`.
pub const BUNDLE_ID: &str = "com.cdj3k.emu";
