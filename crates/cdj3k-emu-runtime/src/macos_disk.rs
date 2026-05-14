//! Native IOKit + DiskArbitration bindings for the USB manager.  Kept in
//! one file so all the unsafe FFI lives in one auditable spot.
//!
//! Three public entry points:
//!
//!   * [`list_removable`] — IOKit IOMedia enumeration.  Yields BSD name,
//!     size, and a human label derived from the volume name (first
//!     partition) or the media name (whole-disk fallback).
//!   * [`unmount_disk`] — DiskArbitration whole-disk unmount.  Synchronous
//!     wrapper around `DADiskUnmount`.
//!   * [`mount_disk`] — DiskArbitration whole-disk mount.  Synchronous
//!     wrapper around `DADiskMount`.
//!
//! Synchronous semantics on async DA APIs:  we schedule a fresh `DASession`
//! on the calling thread's run loop, issue the op with a small context
//! capturing the dissenter status, then pump the run loop in private-mode
//! until the callback fires (with a hard timeout).  No global state, no
//! background threads.

#![cfg(target_os = "macos")]
#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case)]

use std::ffi::{c_void, CStr, CString};
use std::io;
use std::os::raw::{c_char, c_int};
use std::time::{Duration, Instant};

use super::usb::PhysicalDisk;

// ── CoreFoundation ────────────────────────────────────────────────────────────

type CFAllocatorRef = *const c_void;
type CFTypeRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFMutableDictionaryRef = *mut c_void;
type CFStringRef = *const c_void;
type CFBooleanRef = *const c_void;
type CFNumberRef = *const c_void;
type CFRunLoopRef = *mut c_void;
type CFTimeInterval = f64;
type CFIndex = isize;
type CFTypeID = usize;
type Boolean = u8;

const kCFAllocatorDefault: CFAllocatorRef = std::ptr::null();
const kCFNumberSInt64Type: c_int = 4;
const kCFStringEncodingUTF8: u32 = 0x0800_0100;

// CFRunLoopRunInMode return codes.
const kCFRunLoopRunFinished: i32 = 1;
const kCFRunLoopRunStopped: i32 = 2;
const kCFRunLoopRunTimedOut: i32 = 3;
const kCFRunLoopRunHandledSource: i32 = 4;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopDefaultMode: CFStringRef;

    fn CFRelease(cf: CFTypeRef);
    fn CFGetTypeID(cf: CFTypeRef) -> CFTypeID;
    fn CFStringGetTypeID() -> CFTypeID;
    fn CFNumberGetTypeID() -> CFTypeID;
    fn CFBooleanGetTypeID() -> CFTypeID;

    fn CFStringCreateWithCString(
        alloc: CFAllocatorRef,
        cstr: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
    fn CFStringGetLength(s: CFStringRef) -> CFIndex;
    fn CFStringGetMaximumSizeForEncoding(len: CFIndex, encoding: u32) -> CFIndex;
    fn CFStringGetCString(s: CFStringRef, buf: *mut c_char, max: CFIndex, encoding: u32)
        -> Boolean;

    fn CFNumberGetValue(num: CFNumberRef, kind: c_int, out: *mut c_void) -> Boolean;
    fn CFBooleanGetValue(b: CFBooleanRef) -> Boolean;
    fn CFDictionaryGetValue(dict: CFDictionaryRef, key: *const c_void) -> *const c_void;

    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopRunInMode(
        mode: CFStringRef,
        seconds: CFTimeInterval,
        return_after_source_handled: Boolean,
    ) -> i32;
}

// ── IOKit ─────────────────────────────────────────────────────────────────────

type io_iterator_t = u32;
type io_object_t = u32;
type io_registry_entry_t = u32;
type mach_port_t = u32;
type kern_return_t = i32;
type IOOptionBits = u32;

const KERN_SUCCESS: kern_return_t = 0;
const kIOMainPortDefault: mach_port_t = 0;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOServiceMatching(name: *const c_char) -> CFMutableDictionaryRef;
    fn IOServiceGetMatchingServices(
        main_port: mach_port_t,
        matching: CFMutableDictionaryRef,
        existing: *mut io_iterator_t,
    ) -> kern_return_t;
    fn IOIteratorNext(iter: io_iterator_t) -> io_object_t;
    fn IORegistryEntryCreateCFProperty(
        entry: io_registry_entry_t,
        key: CFStringRef,
        allocator: CFAllocatorRef,
        options: IOOptionBits,
    ) -> CFTypeRef;
    fn IOObjectRelease(obj: io_object_t) -> kern_return_t;
}

// ── DiskArbitration ───────────────────────────────────────────────────────────

#[repr(C)]
struct DASession_(c_void);
#[repr(C)]
struct DADisk_(c_void);
#[repr(C)]
struct DADissenter_(c_void);

type DASessionRef = *mut DASession_;
type DADiskRef = *mut DADisk_;
type DADissenterRef = *const DADissenter_;
type CFURLRef = *const c_void;

type DADiskCallback =
    unsafe extern "C" fn(disk: DADiskRef, dissenter: DADissenterRef, context: *mut c_void);

const kDADiskUnmountOptionWhole: u32 = 0x0000_0001;
const kDADiskMountOptionWhole: u32 = 0x0000_0001;

#[link(name = "DiskArbitration", kind = "framework")]
extern "C" {
    static kDADiskDescriptionVolumeNameKey: CFStringRef;
    static kDADiskDescriptionMediaNameKey: CFStringRef;

    fn DASessionCreate(alloc: CFAllocatorRef) -> DASessionRef;
    fn DADiskCreateFromBSDName(
        alloc: CFAllocatorRef,
        session: DASessionRef,
        bsd_name: *const c_char,
    ) -> DADiskRef;
    fn DADiskCopyDescription(disk: DADiskRef) -> CFDictionaryRef;

    fn DASessionScheduleWithRunLoop(
        session: DASessionRef,
        run_loop: CFRunLoopRef,
        mode: CFStringRef,
    );
    fn DASessionUnscheduleFromRunLoop(
        session: DASessionRef,
        run_loop: CFRunLoopRef,
        mode: CFStringRef,
    );

    fn DADiskUnmount(
        disk: DADiskRef,
        options: u32,
        callback: Option<DADiskCallback>,
        context: *mut c_void,
    );
    fn DADiskMount(
        disk: DADiskRef,
        path: CFURLRef,
        options: u32,
        callback: Option<DADiskCallback>,
        context: *mut c_void,
    );

    fn DADissenterGetStatus(dissenter: DADissenterRef) -> i32;
}

// ── CF helpers ────────────────────────────────────────────────────────────────

/// Create a CFString from a Rust &str.  Caller must `CFRelease` the result.
unsafe fn cf_string(s: &str) -> CFStringRef {
    let cstr = CString::new(s).expect("CF key must not contain NUL");
    CFStringCreateWithCString(kCFAllocatorDefault, cstr.as_ptr(), kCFStringEncodingUTF8)
}

/// Convert a CFString to an owned Rust String.  Returns None on type mismatch
/// or encoding failure.
unsafe fn cf_string_to_rust(s: CFStringRef) -> Option<String> {
    if s.is_null() {
        return None;
    }
    if CFGetTypeID(s) != CFStringGetTypeID() {
        return None;
    }
    let len = CFStringGetLength(s);
    let max = CFStringGetMaximumSizeForEncoding(len, kCFStringEncodingUTF8) + 1;
    if max <= 0 {
        return None;
    }
    let mut buf = vec![0u8; max as usize];
    if CFStringGetCString(
        s,
        buf.as_mut_ptr() as *mut c_char,
        max,
        kCFStringEncodingUTF8,
    ) == 0
    {
        return None;
    }
    let cstr = CStr::from_ptr(buf.as_ptr() as *const c_char);
    Some(cstr.to_string_lossy().into_owned())
}

/// Read an IORegistry property by key name.  Caller owns the returned ref
/// (CFRelease on non-null).
unsafe fn ioreg_property(entry: io_registry_entry_t, key: &str) -> CFTypeRef {
    let cf_key = cf_string(key);
    if cf_key.is_null() {
        return std::ptr::null();
    }
    let val = IORegistryEntryCreateCFProperty(entry, cf_key, kCFAllocatorDefault, 0);
    CFRelease(cf_key);
    val
}

unsafe fn ioreg_bool(entry: io_registry_entry_t, key: &str) -> bool {
    let v = ioreg_property(entry, key);
    if v.is_null() {
        return false;
    }
    let out = if CFGetTypeID(v) == CFBooleanGetTypeID() {
        CFBooleanGetValue(v) != 0
    } else {
        false
    };
    CFRelease(v);
    out
}

unsafe fn ioreg_u64(entry: io_registry_entry_t, key: &str) -> Option<u64> {
    let v = ioreg_property(entry, key);
    if v.is_null() {
        return None;
    }
    let mut out: i64 = 0;
    let ok = CFGetTypeID(v) == CFNumberGetTypeID()
        && CFNumberGetValue(v, kCFNumberSInt64Type, &mut out as *mut _ as *mut c_void) != 0;
    CFRelease(v);
    if ok && out >= 0 {
        Some(out as u64)
    } else {
        None
    }
}

unsafe fn ioreg_string(entry: io_registry_entry_t, key: &str) -> Option<String> {
    let v = ioreg_property(entry, key);
    if v.is_null() {
        return None;
    }
    let out = cf_string_to_rust(v);
    CFRelease(v);
    out
}

/// Look up DAVolumeName / DAMediaName for a BSD whole-disk name.  Tries
/// `<bsd>s1` first (most USB sticks have a single partition with a label),
/// then falls back to the whole disk's media name.
unsafe fn disk_friendly_name(session: DASessionRef, bsd_name: &str) -> Option<String> {
    // Try partition 1 for the volume name.
    let part = format!("{bsd_name}s1");
    if let Some(name) = da_string(session, &part, kDADiskDescriptionVolumeNameKey) {
        if !name.trim().is_empty() {
            return Some(name);
        }
    }
    // Fall back to the whole disk's media name (typically the hardware label,
    // e.g. "JetFlash Transcend 32GB Media").
    if let Some(name) = da_string(session, bsd_name, kDADiskDescriptionMediaNameKey) {
        if !name.trim().is_empty() {
            return Some(name);
        }
    }
    None
}

/// Pull a CFString-valued key out of a DADisk description dictionary.
unsafe fn da_string(session: DASessionRef, bsd: &str, key: CFStringRef) -> Option<String> {
    let bsd_c = CString::new(bsd).ok()?;
    let disk = DADiskCreateFromBSDName(kCFAllocatorDefault, session, bsd_c.as_ptr());
    if disk.is_null() {
        return None;
    }
    let desc = DADiskCopyDescription(disk);
    let out = if desc.is_null() {
        None
    } else {
        let val = CFDictionaryGetValue(desc, key as *const c_void);
        let r = cf_string_to_rust(val);
        CFRelease(desc);
        r
    };
    CFRelease(disk as CFTypeRef);
    out
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Enumerate removable, whole-disk IOMedia entries.
pub fn list_removable() -> Vec<PhysicalDisk> {
    let mut out = Vec::new();
    unsafe {
        let matching = IOServiceMatching(b"IOMedia\0".as_ptr() as *const c_char);
        if matching.is_null() {
            return out;
        }
        // IOServiceGetMatchingServices consumes a +1 reference on `matching`.
        let mut iter: io_iterator_t = 0;
        if IOServiceGetMatchingServices(kIOMainPortDefault, matching, &mut iter) != KERN_SUCCESS {
            return out;
        }
        let session = DASessionCreate(kCFAllocatorDefault);
        loop {
            let svc = IOIteratorNext(iter);
            if svc == 0 {
                break;
            }
            // Whole-disk + removable filter.  "Whole" excludes partitions;
            // "Removable" or "Ejectable" picks USB / SD / external drives.
            let is_whole = ioreg_bool(svc, "Whole");
            let is_removable = ioreg_bool(svc, "Removable") || ioreg_bool(svc, "Ejectable");
            if is_whole && is_removable {
                if let Some(bsd) = ioreg_string(svc, "BSD Name") {
                    let size = ioreg_u64(svc, "Size").unwrap_or(0);
                    let friendly = if !session.is_null() {
                        disk_friendly_name(session, &bsd)
                    } else {
                        None
                    };
                    let gb = size as f64 / 1_000_000_000.0;
                    let display = friendly.unwrap_or_else(|| bsd.clone());
                    out.push(PhysicalDisk {
                        bsd_name: bsd.clone(),
                        label: format!("{display} ({gb:.1} GB)"),
                        bsd_path: format!("/dev/{bsd}"),
                        size_bytes: size,
                    });
                }
            }
            IOObjectRelease(svc);
        }
        IOObjectRelease(iter);
        if !session.is_null() {
            CFRelease(session as CFTypeRef);
        }
    }
    out
}

/// Unmount every volume on the given whole disk.  Returns once the unmount
/// callback fires or the timeout elapses.
pub fn unmount_disk(bsd_name: &str) -> io::Result<()> {
    sync_disk_op(bsd_name, "unmount", |disk, ctx| unsafe {
        DADiskUnmount(
            disk,
            kDADiskUnmountOptionWhole,
            Some(on_da_done),
            ctx as *mut c_void,
        );
    })
}

/// Mount every volume on the given whole disk back at its default mount
/// point.  Used after we've handed the raw device to QEMU and want it back.
pub fn mount_disk(bsd_name: &str) -> io::Result<()> {
    sync_disk_op(bsd_name, "mount", |disk, ctx| unsafe {
        DADiskMount(
            disk,
            std::ptr::null(),
            kDADiskMountOptionWhole,
            Some(on_da_done),
            ctx as *mut c_void,
        );
    })
}

// ── Synchronous DA driver ─────────────────────────────────────────────────────

struct DaCtx {
    done: bool,
    status: i32,
}

unsafe extern "C" fn on_da_done(_disk: DADiskRef, dissenter: DADissenterRef, context: *mut c_void) {
    let ctx = &mut *(context as *mut DaCtx);
    ctx.status = if dissenter.is_null() {
        0
    } else {
        DADissenterGetStatus(dissenter)
    };
    ctx.done = true;
}

/// Schedule a session, issue the op, pump the current thread's run loop in
/// the default mode until the callback fires (or until 10 s elapses).
fn sync_disk_op<F>(bsd_name: &str, label: &str, issue: F) -> io::Result<()>
where
    F: FnOnce(DADiskRef, *mut DaCtx),
{
    unsafe {
        let session = DASessionCreate(kCFAllocatorDefault);
        if session.is_null() {
            return Err(io::Error::other(format!("{label}: DASessionCreate failed")));
        }
        let bsd_c = CString::new(bsd_name).map_err(io::Error::other)?;
        let disk = DADiskCreateFromBSDName(kCFAllocatorDefault, session, bsd_c.as_ptr());
        if disk.is_null() {
            CFRelease(session as CFTypeRef);
            return Err(io::Error::other(format!(
                "{label}: DADiskCreateFromBSDName({bsd_name}) failed"
            )));
        }

        let run_loop = CFRunLoopGetCurrent();
        DASessionScheduleWithRunLoop(session, run_loop, kCFRunLoopDefaultMode);

        let mut ctx = DaCtx {
            done: false,
            status: 0,
        };
        issue(disk, &mut ctx as *mut _);

        // Pump run loop in 100 ms slices, total deadline 10 s.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ctx.done && Instant::now() < deadline {
            let rc = CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.1, 1);
            if rc == kCFRunLoopRunFinished {
                // No sources left to service - bail to avoid a tight loop.
                break;
            }
            let _ = (
                kCFRunLoopRunStopped,
                kCFRunLoopRunTimedOut,
                kCFRunLoopRunHandledSource,
            );
        }

        DASessionUnscheduleFromRunLoop(session, run_loop, kCFRunLoopDefaultMode);
        CFRelease(disk as CFTypeRef);
        CFRelease(session as CFTypeRef);

        if !ctx.done {
            return Err(io::Error::other(format!("{label}: timeout on {bsd_name}")));
        }
        if ctx.status != 0 {
            return Err(io::Error::other(format!(
                "{label}: DA error 0x{:08x} on {bsd_name}",
                ctx.status as u32
            )));
        }
        Ok(())
    }
}
