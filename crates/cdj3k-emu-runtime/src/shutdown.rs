//! Soft shutdown. `CdjApp::on_exit` (in cdj3k-emu-ui) uses this module to wait
//! for the runtime worker thread, which the bin crate owns, to finish its
//! graceful `instance.stop()` sequence before it falls back to a hard kill.
//!
//! On app exit:
//!   1. The UI sets `menu_state::lock().app_shutdown = true`.
//!   2. The UI calls [`wait_for_worker(timeout)`], which polls until the
//!      registered worker thread finishes. The worker checks `APP_SHUTDOWN` at
//!      the top of its loop and runs `instance.stop()` before it breaks.
//!   3. If the watchdog elapses, the UI calls `kill_qemu_child()` as a fallback.

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

/// Return true, without blocking, when the registered worker has finished or
/// was never spawned. The UI update loop uses it to decide whether to keep
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

/// Join the registered worker if it has finished, without blocking. Return
/// `true` when no live worker remains: it was never spawned, was already
/// reaped, or is reaped by this call.
pub fn reap_finished_worker() -> bool {
    let Some(slot) = WORKER_JOIN.get() else {
        return true;
    };
    let Ok(mut g) = slot.lock() else {
        return true;
    };
    match g.as_ref() {
        None => true,
        Some(h) if h.is_finished() => {
            if let Some(h) = g.take() {
                let _ = h.join();
            }
            true
        }
        Some(_) => false,
    }
}

/// Wait up to `timeout` for the worker to finish (after `APP_SHUTDOWN` is set).
/// Returns `true` when joined cleanly, `false` if the watchdog elapsed.
pub fn wait_for_worker(timeout: Duration) -> bool {
    let Some(slot) = WORKER_JOIN.get() else {
        // The worker was never spawned, for example when only the firmware wizard ran.
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
            // Take the JoinHandle and join the thread, so that the libc cleanup at exit is clean.
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
