//! The host's own updater, where there is one: Sparkle on macOS.
//!
//! Sparkle draws its own window, downloads, verifies the `.dmg` against the
//! EdDSA key in `Info.plist`, swaps the bundle and relaunches. The delegate
//! hands two moments back to the app as [`Event`]s, which the app answers
//! once the other slots are closed and its own emulation stopped:
//!
//! * [`Event::RelaunchRequested`] - the user chose to install now; answer
//!   with [`proceed_relaunch`].
//! * [`Event::InstallOnQuitReady`] - an update waits for the app to quit;
//!   [`install_now`] puts it in, relaunching or not.
//!
//! Sparkle relaunches the app once, as a plain launch would.

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;
#[cfg(not(target_os = "macos"))]
#[path = "unsupported.rs"]
mod imp;

/// What the host's updater needs the app to act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    RelaunchRequested,
    InstallOnQuitReady,
}

/// Load the host's updater for slot `own` and start its scheduled checks.
/// False where there is none, or it would not load.
pub fn start(own: u32) -> bool {
    imp::start(own)
}

/// What the updater has asked of the app since the last call.
pub fn take_events() -> Vec<Event> {
    imp::take_events()
}

/// A check the user asked for: the updater shows its window whatever it
/// finds.
pub fn check_now() {
    imp::check_now()
}

/// Let a requested relaunch go ahead. False if none is waiting.
pub fn proceed_relaunch() -> bool {
    imp::proceed_relaunch()
}

/// Whether an update is waiting for the app to quit.
pub fn install_pending() -> bool {
    imp::install_pending()
}

/// Install the update that waits for the quit now. The updater then ends
/// this process itself, and relaunches the app when `relaunch` is set.
/// False if none is waiting.
pub fn install_now(relaunch: bool) -> bool {
    imp::install_now(relaunch)
}
