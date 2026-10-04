//! No host updater: the app's own window and installers do the work.

use super::Event;

pub(super) fn start(_own: u32) -> bool {
    false
}

pub(super) fn take_events() -> Vec<Event> {
    Vec::new()
}

pub(super) fn check_now() {}

pub(super) fn proceed_relaunch() -> bool {
    false
}

pub(super) fn install_pending() -> bool {
    false
}

pub(super) fn install_now(_relaunch: bool) -> bool {
    false
}
