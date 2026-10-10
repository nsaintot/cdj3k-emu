//! The actuator, through the private `MultitouchSupport.framework`.
//!
//! Symbols are resolved with `dlopen` and `dlsym` instead of being declared
//! `extern "C"`, so the build neither links the framework nor runs foul of
//! arm64e pointer authentication.
//!
//! `MTActuatorActuate` blocks for the waveform's mechanical stroke (~5–30 ms,
//! per the table in [`super`]), so it runs on a dedicated dispatcher thread and
//! the caller only ever does one non-blocking `try_send`. The blocking call
//! also sets the pace: it returns when the previous stroke completes, so no
//! software rate limit is needed, and requests that arrive faster than the
//! chosen waveform allows are dropped at the bounded channel.
//!
//! The framework is missing on hardware without Force Touch, and the
//! `MTDevice` layout can change between macOS releases. Either case fails
//! closed at [`init`], and every later `actuate()` does nothing.

use std::ffi::CString;
use std::os::raw::{c_int, c_long, c_void};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Mutex, OnceLock};
use std::thread;

/// Bounded queue depth between `actuate()` callers and the dispatcher thread.
/// Sized to absorb a brief burst of detents at extreme jog speeds without
/// either blocking the caller or letting a backlog grow. When the queue is
/// full, new requests are dropped, because the actuator already fires as fast
/// as the hardware allows.
const HAPTIC_QUEUE_DEPTH: usize = 8;

type IOReturn = c_int;

// ── Private MultitouchSupport.framework symbols ─────────────────────────
type FnActuatorCreateFromDeviceID = unsafe extern "C" fn(u64) -> *mut c_void;
type FnActuatorOpen = unsafe extern "C" fn(*mut c_void, u32) -> IOReturn;
type FnActuatorClose = unsafe extern "C" fn(*mut c_void) -> IOReturn;
type FnActuatorActuate = unsafe extern "C" fn(*mut c_void, i32, u32, u32, u32) -> IOReturn;
type FnDeviceCreateList = unsafe extern "C" fn() -> *mut c_void; // CFMutableArrayRef

// ── CoreFoundation symbols ──────────────────────────────────────────────
// These are also resolved with dlsym, so the build adds no direct link to
// CoreFoundation, which is already linked transitively.
type FnArrayGetCount = unsafe extern "C" fn(*mut c_void) -> c_long;
type FnArrayGetValueAtIndex = unsafe extern "C" fn(*mut c_void, c_long) -> *mut c_void;
type FnRelease = unsafe extern "C" fn(*mut c_void);

const MT_FW: &str =
    "/System/Library/PrivateFrameworks/MultitouchSupport.framework/MultitouchSupport";
const CF_FW: &str = "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation";

/// Offset of the `uint64_t` device-ID field inside the opaque MTDevice
/// struct on Apple silicon. It can change between macOS versions, and init
/// then fails closed.
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
/// The opened actuator, a CFTypeRef stored as a usize because raw pointers
/// are not `Sync`.
static ACTUATOR: AtomicUsize = AtomicUsize::new(0);
static AVAILABLE: AtomicBool = AtomicBool::new(false);

// ── Telemetry counters ──────────────────────────────────────────────────
// The counters only increase. [`stats()`] reads them, and callers diff two
// snapshots to get rates. With relaxed atomics, a count can be off by a few
// around a sample boundary, which does not matter for tuning by eye.
static FIRED_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static DROPPED_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The bounded channel into the dispatcher thread. `actuate()` does one
/// non-blocking `try_send`, and the dispatcher drains it as fast as the
/// trackpad hardware allows. It is stored in a `Mutex<Option<...>>` so that
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
        // SAFETY: the caller passes a `T` the size of a function pointer, and
        // dlsym returned the symbol's address.
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

/// Open the trackpad actuator and start the dispatcher thread. Returns true
/// on success; a second call after a success does nothing.
///
/// Call it once at app startup. On a Mac without a Force Touch trackpad, or
/// on a macOS release where the MTDevice layout has changed, it returns false
/// and [`actuate`] does nothing.
pub fn init() -> bool {
    if AVAILABLE.load(Ordering::Relaxed) {
        return true;
    }
    let Some(syms) = SYMBOLS.get_or_init(load_symbols) else {
        eprintln!(
            "[haptic] MultitouchSupport.framework cannot be loaded; the actuator is disabled"
        );
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
            let device_id =
                std::ptr::read_unaligned((dev as *const u8).add(MTDEVICE_ID_OFFSET) as *const u64);
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
        eprintln!("[haptic] no trackpad with a haptic actuator was found");
        return false;
    }
    ACTUATOR.store(actuator_ptr as usize, Ordering::Relaxed);

    // The dispatcher thread exists only to take the blocking
    // `MTActuatorActuate` call off the UI thread. The hardware paces the
    // rate, because the call returns after the previous waveform's stroke
    // completes.
    let (tx, rx) = sync_channel::<i32>(HAPTIC_QUEUE_DEPTH);
    let actuator_usize = actuator_ptr as usize;
    thread::Builder::new()
        .name("haptic-dispatcher".into())
        .spawn(move || dispatcher_loop(rx, actuator_usize))
        .expect("the haptic-dispatcher thread could not be started");
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

        // Open the actuator right before each actuation.
        let open_ret = unsafe { (syms.actuator_open)(actuator, 0) };
        if open_ret != 0 {
            eprintln!("[haptic.trigger] the actuator did not open: ret={open_ret}");
            DROPPED_COUNT.fetch_add(1, Ordering::Relaxed);
            continue;
        }

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
                "[haptic.trigger] waveform {waveform} failed: ret={ret} ({name}, 0x{:08X})",
                ret as u32,
            );
            DROPPED_COUNT.fetch_add(1, Ordering::Relaxed);
        }

        // Close the actuator so that the next iteration starts fresh.
        // Reopening it immediately after a close returns kIOReturnError; the
        // wait on the channel and the stroke time leave enough gap before the
        // next open.
        unsafe {
            (syms.actuator_close)(actuator);
        }
    }
}

/// Whether an actuator was opened at init. It is one relaxed atomic load, so
/// the hot path can call it.
pub fn available() -> bool {
    AVAILABLE.load(Ordering::Relaxed)
}

/// Queue one haptic pulse with the given waveform ID. The call does not
/// block: it is one channel `try_send`. The dispatcher thread fires the pulse
/// as fast as the trackpad hardware allows for that waveform. See [`super`]
/// for the waveform table.
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

/// A snapshot of (fired_total, dropped_total) since the process started.
/// Callers diff two snapshots to get rates.
pub fn stats() -> (u64, u64) {
    (
        FIRED_COUNT.load(Ordering::Relaxed),
        DROPPED_COUNT.load(Ordering::Relaxed),
    )
}

/// Close the actuator and stop the dispatcher. macOS also reclaims the
/// actuator on process exit.
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
