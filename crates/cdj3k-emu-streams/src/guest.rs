//! The stream threads wait for a running guest before they look for the
//! sockets and shared memory that QEMU creates.

use std::thread;
use std::time::Duration;

use cdj3k_emu_platform::menu_state;

/// How often a waiting stream checks whether QEMU has started.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Block until the runtime reports a running QEMU. Returns whether it had to
/// wait, so a caller can restart its own retry count for the new guest.
pub(crate) fn wait_until_running() -> bool {
    let mut waited = false;
    while !menu_state::lock().qemu_running {
        waited = true;
        thread::sleep(POLL_INTERVAL);
    }
    waited
}
