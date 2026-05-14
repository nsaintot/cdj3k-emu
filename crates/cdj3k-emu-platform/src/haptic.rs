//! Direct trackpad actuator access via `MultitouchSupport.framework` (private).
//!
//! API surface: `init()` once at startup, `actuate(waveform)` per click,
//! `shutdown()` on exit.  Each `actuate()` is a non-blocking enqueue;
//! `MTActuatorActuate` runs on a dedicated dispatcher thread so it can
//! block on the trackpad's mechanical actuation period (~5-30 ms depending
//! on waveform) without stalling the UI.
//!
//! Rate is self-paced by the hardware: `MTActuatorActuate` returns only
//! after the previous waveform's mechanical stroke completes, so the
//! dispatcher naturally fires at whatever rate the chosen waveform allows.
//! No software-side rate limit is needed; excess requests at very high
//! jog speeds are dropped at the bounded-channel `try_send`.
//!
//!
//!   1  - weak click            (~5 ms, ~150 Hz natural cap)
//!   2  - strong click          (~10 ms, ~80 Hz)        - Force Touch feel
//!   3  - buzz / notification   (longer, blends to drone at high rate)
//!   4  - light tap             (~8 ms, ~100 Hz)
//!   5  - medium tap            (~12 ms, ~65 Hz)
//!   6  - strong tap            (~20 ms, ~48 Hz)        - sharp hard punch
//!   15 - soft thud
//!   16 - strong thud           (~30 ms, ~30 Hz)        - heaviest single pulse
//!
//! Loaded dynamically; missing on non-Force-Touch hardware (every actuate()
//! call becomes a silent no-op once we discover that fact at init time).
//! Uses [`dlopen`]/[`dlsym`] rather than `extern "C"` declarations so we
//! don't fall foul of arm64e pointer authentication.

/// Bounded queue depth between `actuate()` callers and the dispatcher thread.
/// Sized to absorb a brief burst of detents at extreme jog speeds without
/// either blocking the caller or letting a backlog grow.  When the queue is
/// full, new requests are dropped (the actuator is already firing as fast as
/// the hardware allows - those extra clicks couldn't be played anyway).
const HAPTIC_QUEUE_DEPTH: usize = 8;

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::CString;
    use std::os::raw::{c_int, c_long, c_void};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc::{sync_channel, SyncSender};
    use std::sync::{Mutex, OnceLock};
    use std::thread;

    type IOReturn = c_int;

    // ── Private MultitouchSupport.framework symbols ─────────────────────────
    type FnActuatorCreateFromDeviceID = unsafe extern "C" fn(u64) -> *mut c_void;
    type FnActuatorOpen = unsafe extern "C" fn(*mut c_void, u32) -> IOReturn;
    type FnActuatorClose = unsafe extern "C" fn(*mut c_void) -> IOReturn;
    type FnActuatorActuate = unsafe extern "C" fn(*mut c_void, i32, u32, u32, u32) -> IOReturn;
    type FnDeviceCreateList = unsafe extern "C" fn() -> *mut c_void; // CFMutableArrayRef

    // ── CoreFoundation symbols (also via dlsym so the build doesn't grow a
    // direct link to CoreFoundation; we already link it transitively). ──────
    type FnArrayGetCount = unsafe extern "C" fn(*mut c_void) -> c_long;
    type FnArrayGetValueAtIndex = unsafe extern "C" fn(*mut c_void, c_long) -> *mut c_void;
    type FnRelease = unsafe extern "C" fn(*mut c_void);

    const MT_FW: &str =
        "/System/Library/PrivateFrameworks/MultitouchSupport.framework/MultitouchSupport";
    const CF_FW: &str = "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation";

    /// Offset of the `uint64_t` device-ID field inside the opaque MTDevice
    /// struct. Found empirically on M-series machines,
    /// May change across macOS versions - if it does, we fail closed at init.
    const MTDEVICE_ID_OFFSET: usize = 64;

    struct Symbols {
        actuator_create: FnActuatorCreateFromDeviceID,
        actuator_open: FnActuatorOpen,
        actuator_close: FnActuatorClose,
        actuator_actuate: FnActuatorActuate,
        device_create_list: FnDeviceCreateList,
        cf_array_get_count: FnArrayGetCount,
        cf_array_get_value_at_index: FnArrayGetValueAtIndex,
        cf_release: FnRelease,
    }

    // SAFETY: function pointers from dlsym are immutable for the life of the
    // process and the underlying functions are documented as thread-safe.
    unsafe impl Send for Symbols {}
    unsafe impl Sync for Symbols {}

    static SYMBOLS: OnceLock<Option<Symbols>> = OnceLock::new();
    /// The opened actuator (CFTypeRef cast to usize - raw pointers aren't
    /// `Sync` and we don't want a `Mutex` on the hot actuate path).
    static ACTUATOR: AtomicUsize = AtomicUsize::new(0);
    static AVAILABLE: AtomicBool = AtomicBool::new(false);

    // ── Telemetry counters (monotonic, never reset) ─────────────────────────
    // Read via [`stats()`].  Callers diff against a previous snapshot to
    // compute rates.  Relaxed atomics: imprecise around a sample boundary
    // by at most a few counts, irrelevant for human-eyed tuning.
    pub(super) static FIRED_COUNT: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    pub(super) static DROPPED_COUNT: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);

    /// Bounded channel into the dispatcher thread.  `actuate()` does one
    /// non-blocking `try_send`; the dispatcher drains it as fast as the
    /// trackpad hardware allows.  Stored in a `Mutex<Option<...>>` so
    /// `shutdown()` can drop the sender, which closes the channel and lets
    /// the worker thread exit cleanly.
    static SENDER: OnceLock<Mutex<Option<SyncSender<i32>>>> = OnceLock::new();

    unsafe fn dlopen_lib(path: &str) -> Option<*mut c_void> {
        let c = CString::new(path).ok()?;
        let h = libc::dlopen(c.as_ptr(), libc::RTLD_LAZY);
        if h.is_null() {
            None
        } else {
            Some(h)
        }
    }

    unsafe fn dlsym_required<T>(handle: *mut c_void, name: &str) -> Option<T> {
        let c = CString::new(name).ok()?;
        let p = libc::dlsym(handle, c.as_ptr());
        if p.is_null() {
            None
        } else {
            // SAFETY: caller provides T sized to a function pointer, dlsym
            // returns the requested symbol's address.
            Some(std::mem::transmute_copy::<*mut c_void, T>(&p))
        }
    }

    fn load_symbols() -> Option<Symbols> {
        unsafe {
            let mt = dlopen_lib(MT_FW)?;
            let cf = dlopen_lib(CF_FW)?;
            Some(Symbols {
                actuator_create: dlsym_required(mt, "MTActuatorCreateFromDeviceID")?,
                actuator_open: dlsym_required(mt, "MTActuatorOpen")?,
                actuator_close: dlsym_required(mt, "MTActuatorClose")?,
                actuator_actuate: dlsym_required(mt, "MTActuatorActuate")?,
                device_create_list: dlsym_required(mt, "MTDeviceCreateList")?,
                cf_array_get_count: dlsym_required(cf, "CFArrayGetCount")?,
                cf_array_get_value_at_index: dlsym_required(cf, "CFArrayGetValueAtIndex")?,
                cf_release: dlsym_required(cf, "CFRelease")?,
            })
        }
    }

    /// Open the trackpad actuator and spawn the dispatcher thread.
    /// Returns true on success.  Idempotent.
    ///
    /// Must be called once at app startup. After this, [`actuate`] silently
    /// no-ops on systems without a Force Touch trackpad (or on macOS releases
    /// where the MT struct layout has drifted).
    pub fn init() -> bool {
        if AVAILABLE.load(Ordering::Relaxed) {
            return true;
        }
        let Some(syms) = SYMBOLS.get_or_init(load_symbols) else {
            eprintln!("[haptic] MultitouchSupport.framework not loadable - actuator disabled");
            return false;
        };
        let actuator_ptr = unsafe {
            let devices = (syms.device_create_list)();
            if devices.is_null() {
                eprintln!("[haptic] MTDeviceCreateList returned NULL");
                return false;
            }
            let count = (syms.cf_array_get_count)(devices);
            let mut found: *mut c_void = std::ptr::null_mut();
            for i in 0..count {
                let dev = (syms.cf_array_get_value_at_index)(devices, i);
                if dev.is_null() {
                    continue;
                }
                let device_id = std::ptr::read_unaligned(
                    (dev as *const u8).add(MTDEVICE_ID_OFFSET) as *const u64,
                );
                let actuator = (syms.actuator_create)(device_id);
                if actuator.is_null() {
                    continue;
                }
                let ret = (syms.actuator_open)(actuator, 0);
                if ret == 0 {
                    found = actuator;
                    break;
                }
                (syms.cf_release)(actuator);
            }
            (syms.cf_release)(devices);
            found
        };
        if actuator_ptr.is_null() {
            eprintln!("[haptic] no trackpad with haptic actuator found");
            return false;
        }
        ACTUATOR.store(actuator_ptr as usize, Ordering::Relaxed);

        // Spawn the dispatcher thread.  It exists only to take the blocking
        // `MTActuatorActuate` call off the UI thread - the hardware itself
        // paces the rate (the call returns after the previous waveform's
        // mechanical stroke completes).  No software rate limit.
        let (tx, rx) = sync_channel::<i32>(super::HAPTIC_QUEUE_DEPTH);
        let actuator_usize = actuator_ptr as usize;
        thread::Builder::new()
            .name("haptic-dispatcher".into())
            .spawn(move || dispatcher_loop(rx, actuator_usize))
            .expect("spawn haptic-dispatcher thread");
        let _ = SENDER
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map(|mut g| *g = Some(tx));

        AVAILABLE.store(true, Ordering::Relaxed);
        eprintln!("[haptic] trackpad actuator opened");
        true
    }

    fn dispatcher_loop(rx: std::sync::mpsc::Receiver<i32>, actuator_usize: usize) {
        let Some(syms) = SYMBOLS.get().and_then(|o| o.as_ref()) else {
            return;
        };
        let actuator = actuator_usize as *mut c_void;

        for waveform in rx {
            if !AVAILABLE.load(Ordering::Relaxed) {
                break;
            }

            // Step 1: prime - open the actuator right before the actuate.
            let open_ret = unsafe { (syms.actuator_open)(actuator, 0) };
            if open_ret != 0 {
                eprintln!("[haptic.trigger] open failed ret={open_ret}");
                DROPPED_COUNT.fetch_add(1, Ordering::Relaxed);
                continue;
            }

            // Step 2: actuate.
            let ret = unsafe { (syms.actuator_actuate)(actuator, waveform, 0, 0, 0) };
            if ret == 0 {
                FIRED_COUNT.fetch_add(1, Ordering::Relaxed);
            } else {
                let name = match ret {
                    -536870212 => "kIOReturnError",
                    -536870201 => "kIOReturnNotPermitted",
                    -536870189 => "kIOReturnBusy",
                    _ => "unknown",
                };
                eprintln!(
                    "[haptic.trigger] wf={waveform} ret={ret} ({name}, 0x{:08X})",
                    ret as u32,
                );
                DROPPED_COUNT.fetch_add(1, Ordering::Relaxed);
            }

            // Step 3: close so the next iteration starts fresh.  The next
            // open() call (on the following iteration) is what actually
            // serves as the inter-pulse gap - reopening immediately after
            // close hits kIOReturnError, but the channel recv between
            // iterations plus the hardware's own stroke time keeps us safe.
            unsafe {
                (syms.actuator_close)(actuator);
            }
        }
    }

    /// True if an actuator was opened at init time.  Cheap to call from
    /// the hot path - one relaxed atomic load.
    pub fn available() -> bool {
        AVAILABLE.load(Ordering::Relaxed)
    }

    /// Queue one haptic pulse with the given waveform ID.  Non-blocking:
    /// the call returns in microseconds (one channel `try_send`).  The
    /// dispatcher thread fires the pulse as fast as the trackpad hardware
    /// allows for that waveform.  See module doc for the waveform table.
    pub fn actuate(waveform: i32) {
        if !AVAILABLE.load(Ordering::Relaxed) {
            DROPPED_COUNT.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let Some(slot) = SENDER.get() else {
            DROPPED_COUNT.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let Ok(guard) = slot.lock() else {
            DROPPED_COUNT.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if let Some(sender) = guard.as_ref() {
            if sender.try_send(waveform).is_err() {
                DROPPED_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        } else {
            DROPPED_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Snapshot of (fired_total, dropped_total) since process start.  Callers
    /// diff against a previous snapshot to compute rates.  See
    /// [`super::max_rate_hz`] for the configured ceiling to compare against.
    pub fn stats() -> (u64, u64) {
        (
            FIRED_COUNT.load(Ordering::Relaxed),
            DROPPED_COUNT.load(Ordering::Relaxed),
        )
    }

    /// Close the actuator and stop the dispatcher.  macOS reclaims the
    /// actuator on process exit, but calling this from an `on_exit` hook
    /// is the polite path.
    pub fn shutdown() {
        if !AVAILABLE.swap(false, Ordering::Relaxed) {
            return;
        }
        // Drop the sender so the dispatcher thread observes the channel as
        // closed and exits its `for waveform in rx` loop.
        if let Some(slot) = SENDER.get() {
            if let Ok(mut g) = slot.lock() {
                g.take();
            }
        }
        let actuator = ACTUATOR.swap(0, Ordering::Relaxed);
        if actuator == 0 {
            return;
        }
        let Some(syms) = SYMBOLS.get().and_then(|o| o.as_ref()) else {
            return;
        };
        unsafe {
            let ptr = actuator as *mut c_void;
            (syms.actuator_close)(ptr);
            (syms.cf_release)(ptr);
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::{actuate, available, init, shutdown, stats};

#[cfg(not(target_os = "macos"))]
pub fn init() -> bool {
    false
}
#[cfg(not(target_os = "macos"))]
pub fn available() -> bool {
    false
}
#[cfg(not(target_os = "macos"))]
pub fn actuate(_waveform: i32) {}
#[cfg(not(target_os = "macos"))]
pub fn shutdown() {}
#[cfg(not(target_os = "macos"))]
pub fn stats() -> (u64, u64) {
    (0, 0)
}
