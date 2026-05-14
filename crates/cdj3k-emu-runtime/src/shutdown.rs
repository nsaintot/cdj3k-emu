//! Soft-shutdown plumbing: lets `CdjApp::on_exit` (in cdj3k-emu-ui) wait for
//! the runtime worker thread (owned by the bin crate) to finish its graceful
//! `instance.stop()` sequence before falling back to a hard kill.
//!
//! Flow on app exit:
//!   1. UI sets `menu_state::lock().app_shutdown = true`.
//!   2. UI calls [`wait_for_worker(timeout)`] - polls until the registered
//!      worker thread finishes (it observes `APP_SHUTDOWN` at the top of its
//!      loop and runs `instance.stop()` before breaking).
//!   3. If the watchdog elapses, UI calls `kill_qemu_child()` as fallback.

use std::sync::{Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

static WORKER_JOIN: OnceLock<Mutex<Option<JoinHandle<()>>>> = OnceLock::new();

/// How often `wait_for_worker` checks `is_finished()`.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Called once after the runtime worker thread is spawned.
pub fn register_worker_thread(handle: JoinHandle<()>) {
    let slot = WORKER_JOIN.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = slot.lock() {
        *g = Some(handle);
    }
}

/// Non-blocking check: true when the registered worker has finished (or was
/// never spawned).  Used by the UI update loop to decide whether to keep
/// deferring the window close while the graceful runtime stop runs.
pub fn worker_is_finished() -> bool {
    let Some(slot) = WORKER_JOIN.get() else {
        return true;
    };
    slot.lock()
        .ok()
        .and_then(|g| g.as_ref().map(|h| h.is_finished()))
        .unwrap_or(true)
}

/// Wait up to `timeout` for the worker to finish (after `APP_SHUTDOWN` is set).
/// Returns `true` when joined cleanly, `false` if the watchdog elapsed.
pub fn wait_for_worker(timeout: Duration) -> bool {
    let Some(slot) = WORKER_JOIN.get() else {
        // Worker never spawned (e.g. firmware-wizard-only path).
        return true;
    };
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let finished = slot
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|h| h.is_finished()))
            .unwrap_or(true);
        if finished {
            // Take and join - drops the JoinHandle so libc cleanup is clean.
            if let Ok(mut g) = slot.lock() {
                if let Some(h) = g.take() {
                    let _ = h.join();
                }
            }
            return true;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    false
}
