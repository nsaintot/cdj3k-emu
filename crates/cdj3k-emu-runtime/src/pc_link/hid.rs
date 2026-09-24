//! Virtual HID device for the PC-link gadget HID endpoint.
//!
//! Registers an `IOHIDUserDevice` carrying the CDJ-3000's vendor-page report
//! descriptor, so anything on the Mac that opens HID by VID/PID (rekordbox,
//! `hidapi` clients, Console's HID logging) sees the emulated deck exactly as
//! it would see one on a USB-B cable.
//!
//! Two directions:
//!   - guest → host: [`HidBackend::publish_from_guest`] feeds 64-byte input
//!     reports the guest read off `/dev/hidraw0` into `IOHIDUserDeviceHandleReport`.
//!   - host → guest: the set-report callback fires on our dispatch queue for
//!     every output report a Mac-side client writes, and enqueues a
//!     `FRAME_HID` frame for the transport writer thread.
//!
//! `IOHIDUserDevice` is a private IOKit SPI.  `IOHIDUserDeviceCreateWithProperties`
//! returns NULL unless the calling binary carries
//! `com.apple.developer.hid.virtual.device` backed by a provisioning profile
//! Apple granted the entitlement on; that is what [`HidError::Create`]
//! reports.  A build without it still runs; [`crate::pc_link::PcLink`] keeps
//! MIDI and drops HID frames.

#![cfg(target_os = "macos")]

use std::os::raw::{c_char, c_void};
use std::sync::mpsc::Sender;

use crate::pc_link::frame::FRAME_HID;
use crate::pc_link::transport::OutFrame;

// ── CoreFoundation / IOKit FFI ─────────────────────────────────────────────

type CFIndex = isize;
type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFNumberRef = *const c_void;
type CFDataRef = *const c_void;
type CFAllocatorRef = *const c_void;
type CFMutableDictionaryRef = *mut c_void;
type IOReturn = i32;
type IOOptionBits = u32;
type IOHIDUserDeviceRef = *mut c_void;
type DispatchQueueRef = *mut c_void;

/// `IOReturn` success.
const K_IO_RETURN_SUCCESS: IOReturn = 0;
/// `kIOHIDReportTypeOutput`, the only direction a Mac-side client can push
/// bytes at us that the gadget's OUT endpoint carries.
const K_IOHID_REPORT_TYPE_OUTPUT: u32 = 1;
const K_CF_NUMBER_SINT32_TYPE: CFIndex = 3;
const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

type IOHIDUserDeviceReportCallback = extern "C" fn(
    refcon: *mut c_void,
    report_type: u32,
    report_id: u32,
    report: *mut u8,
    report_length: CFIndex,
) -> IOReturn;

#[link(name = "IOKit", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;

    fn CFDictionaryCreateMutable(
        alloc: CFAllocatorRef,
        capacity: CFIndex,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CFMutableDictionaryRef;
    fn CFDictionarySetValue(dict: CFMutableDictionaryRef, key: CFTypeRef, value: CFTypeRef);
    fn CFNumberCreate(
        alloc: CFAllocatorRef,
        number_type: CFIndex,
        value_ptr: *const c_void,
    ) -> CFNumberRef;
    fn CFDataCreate(alloc: CFAllocatorRef, bytes: *const u8, length: CFIndex) -> CFDataRef;
    fn CFStringCreateWithBytes(
        alloc: CFAllocatorRef,
        bytes: *const u8,
        len: CFIndex,
        encoding: u32,
        is_external_rep: bool,
    ) -> CFStringRef;
    fn CFRelease(cf: CFTypeRef);

    fn IOHIDUserDeviceCreateWithProperties(
        alloc: CFAllocatorRef,
        properties: CFMutableDictionaryRef,
        options: IOOptionBits,
    ) -> IOHIDUserDeviceRef;
    fn IOHIDUserDeviceRegisterSetReportCallback(
        device: IOHIDUserDeviceRef,
        callback: IOHIDUserDeviceReportCallback,
        refcon: *mut c_void,
    );
    fn IOHIDUserDeviceScheduleWithDispatchQueue(
        device: IOHIDUserDeviceRef,
        queue: DispatchQueueRef,
    );
    fn IOHIDUserDeviceUnscheduleFromDispatchQueue(
        device: IOHIDUserDeviceRef,
        queue: DispatchQueueRef,
    );
    fn IOHIDUserDeviceHandleReport(
        device: IOHIDUserDeviceRef,
        report: *const u8,
        report_length: CFIndex,
    ) -> IOReturn;

    fn dispatch_queue_create(label: *const c_char, attr: *const c_void) -> DispatchQueueRef;
    fn dispatch_release(object: *mut c_void);
    fn dispatch_sync_f(
        queue: DispatchQueueRef,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
}

/// Barrier body for [`HidBackend::drop`]; does nothing but occupy the queue.
extern "C" fn queue_barrier(_context: *mut c_void) {}

/// Owned CoreFoundation reference, released on drop.
struct CFRef(CFTypeRef);

impl CFRef {
    fn get(&self) -> CFTypeRef {
        self.0
    }
}

impl Drop for CFRef {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) };
    }
}

fn cf_string(s: &str) -> CFRef {
    let cf = unsafe {
        CFStringCreateWithBytes(
            std::ptr::null(),
            s.as_ptr(),
            s.len() as CFIndex,
            K_CF_STRING_ENCODING_UTF8,
            false,
        )
    };
    assert!(!cf.is_null(), "CFStringCreateWithBytes returned NULL");
    CFRef(cf)
}

fn cf_number(v: i32) -> CFRef {
    let cf = unsafe {
        CFNumberCreate(
            std::ptr::null(),
            K_CF_NUMBER_SINT32_TYPE,
            &v as *const i32 as *const c_void,
        )
    };
    assert!(!cf.is_null(), "CFNumberCreate returned NULL");
    CFRef(cf)
}

fn cf_data(bytes: &[u8]) -> CFRef {
    let cf = unsafe { CFDataCreate(std::ptr::null(), bytes.as_ptr(), bytes.len() as CFIndex) };
    assert!(!cf.is_null(), "CFDataCreate returned NULL");
    CFRef(cf)
}

// ── Device identity ────────────────────────────────────────────────────────

/// The CDJ-3000's HID report descriptor, byte-for-byte the value patch 28
/// writes to `functions/hid.usb0/report_desc` in the guest's gadget script:
/// vendor usage page 0xFFA0, one 64-byte input report and one 64-byte output
/// report, no report IDs.
const REPORT_DESCRIPTOR: [u8; 52] = [
    0x06, 0xa0, 0xff, // Usage Page (vendor 0xFFA0)
    0x09, 0x01, //       Usage (0x01)
    0xa1, 0x01, //       Collection (Application)
    0x09, 0x02, //         Usage (0x02)
    0xa1, 0x00, //         Collection (Physical)
    0x06, 0xa1, 0xff, //     Usage Page (vendor 0xFFA1)
    0x09, 0x03, //           Usage (0x03)
    0x09, 0x04, //           Usage (0x04)
    0x15, 0x80, //           Logical Minimum (-128)
    0x25, 0x7f, //           Logical Maximum (127)
    0x35, 0x00, //           Physical Minimum (0)
    0x45, 0xff, //           Physical Maximum (255)
    0x75, 0x08, //           Report Size (8)
    0x95, 0x40, //           Report Count (64)
    0x81, 0x02, //           Input (Data, Var, Abs)
    0x09, 0x05, //           Usage (0x05)
    0x09, 0x06, //           Usage (0x06)
    0x15, 0x80, //           Logical Minimum (-128)
    0x25, 0x7f, //           Logical Maximum (127)
    0x35, 0x00, //           Physical Minimum (0)
    0x45, 0xff, //           Physical Maximum (255)
    0x75, 0x08, //           Report Size (8)
    0x95, 0x40, //           Report Count (64)
    0x91, 0x02, //           Output (Data, Var, Abs)
    0xc0, //               End Collection
    0xc0, //             End Collection
];

/// Matches the gadget's `idVendor` / `idProduct` (patch 28) so Mac-side
/// clients that key on VID/PID find the emulated deck under the same numbers
/// as real hardware.
const VENDOR_ID: i32 = cdj3k_emu_platform::identity::VENDOR_ID as i32;
const PRODUCT_ID: i32 = cdj3k_emu_platform::identity::PRODUCT_ID as i32;
/// Gadget `bcdDevice` 0x0100, firmware 1.00.
const VERSION_NUMBER: i32 = 0x0100;
/// Primary usage page / usage of the descriptor's outermost collection.
const PRIMARY_USAGE_PAGE: i32 = 0xffa0;
const PRIMARY_USAGE: i32 = 0x01;
// The LocationID comes from `identity::usb_location_id` and must equal the
// CoreMIDI driver's `USBLocationID`: djay pairs a MIDI device with its HID
// sibling by matching VendorID+ProductID+LocationID, so the HID device is
// invisible to djay's `openHIDDevice` unless the two agree.

// ── Public API ─────────────────────────────────────────────────────────────

/// Errors registering the virtual HID device.
#[derive(Debug)]
pub enum HidError {
    /// `IOHIDUserDeviceCreateWithProperties` returned NULL.  In practice this
    /// is the entitlement gate: the binary needs
    /// `com.apple.developer.hid.virtual.device` backed by an embedded
    /// provisioning profile.
    Create,
}

impl std::fmt::Display for HidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Create => write!(
                f,
                "IOHIDUserDeviceCreateWithProperties returned NULL \
                 (needs com.apple.developer.hid.virtual.device + provisioning profile)"
            ),
        }
    }
}

impl std::error::Error for HidError {}

/// A live `IOHIDUserDevice` presenting the emulated CDJ-3000 to macOS.
///
/// Construction registers the device and it appears in IOReg immediately.
/// Drop unschedules it and releases the IOKit + dispatch handles.
pub struct HidBackend {
    device: IOHIDUserDeviceRef,
    queue: DispatchQueueRef,
    // The set-report callback holds a raw pointer into this box; we own it so
    // its lifetime covers every callback invocation.
    _refcon: Box<Sender<OutFrame>>,
}

extern "C" fn set_report_cb(
    refcon: *mut c_void,
    report_type: u32,
    _report_id: u32,
    report: *mut u8,
    report_length: CFIndex,
) -> IOReturn {
    if refcon.is_null() || report.is_null() || report_length <= 0 {
        return K_IO_RETURN_SUCCESS;
    }
    // Feature reports have no gadget-side destination: the guest bridge writes
    // what arrives here to /dev/hidraw0, which is the OUT endpoint.
    if report_type != K_IOHID_REPORT_TYPE_OUTPUT {
        return K_IO_RETURN_SUCCESS;
    }
    // SAFETY: refcon is the Box<Sender<OutFrame>> built in `HidBackend::new`,
    // borrowed here.  IOHIDUserDeviceUnscheduleFromDispatchQueue in Drop runs
    // before the box is released, and the queue is serial, so no callback is
    // in flight past that point.
    let tx: &Sender<OutFrame> = unsafe { &*(refcon as *const Sender<OutFrame>) };
    let payload = unsafe { std::slice::from_raw_parts(report, report_length as usize) }.to_vec();
    // Best-effort: a dead writer thread means the toggle went off or the guest
    // disconnected, and PcLink tears us down shortly after.
    let _ = tx.send((FRAME_HID, payload));
    K_IO_RETURN_SUCCESS
}

impl HidBackend {
    /// Register the virtual HID device.  `tx` is where output reports from
    /// Mac-side clients are enqueued for the guest.
    pub fn new(tx: Sender<OutFrame>, instance_id: u32) -> Result<Self, HidError> {
        let serial = cdj3k_emu_platform::identity::device_serial(instance_id);
        let location_id = cdj3k_emu_platform::identity::usb_location_id(instance_id);
        let properties = unsafe {
            CFDictionaryCreateMutable(
                std::ptr::null(),
                0,
                &kCFTypeDictionaryKeyCallBacks as *const c_void,
                &kCFTypeDictionaryValueCallBacks as *const c_void,
            )
        };
        assert!(!properties.is_null(), "CFDictionaryCreateMutable returned NULL");
        let properties = CFRef(properties as CFTypeRef);
        let dict = properties.get() as CFMutableDictionaryRef;

        let descriptor = cf_data(&REPORT_DESCRIPTOR);
        // IOHIDKeys.h names; IOHIDUserDevice reads them straight out of the
        // properties dictionary to build the IOHIDDevice's registry entry.
        // IOHIDUserDevice overwrites Transport with "Virtual" in the registry
        // entry regardless of what is passed here; the rest come through
        // unchanged.
        let string_props: [(&str, CFRef); 4] = [
            ("Transport", cf_string("USB")),
            ("Manufacturer", cf_string(cdj3k_emu_platform::identity::MANUFACTURER)),
            ("Product", cf_string(cdj3k_emu_platform::identity::PRODUCT)),
            ("SerialNumber", cf_string(&serial)),
        ];
        let number_props: [(&str, CFRef); 6] = [
            ("VendorID", cf_number(VENDOR_ID)),
            ("ProductID", cf_number(PRODUCT_ID)),
            ("VersionNumber", cf_number(VERSION_NUMBER)),
            ("PrimaryUsagePage", cf_number(PRIMARY_USAGE_PAGE)),
            ("PrimaryUsage", cf_number(PRIMARY_USAGE)),
            ("LocationID", cf_number(location_id)),
        ];

        let descriptor_key = cf_string("ReportDescriptor");
        unsafe {
            CFDictionarySetValue(dict, descriptor_key.get(), descriptor.get());
            for (name, value) in &string_props {
                let key = cf_string(name);
                CFDictionarySetValue(dict, key.get(), value.get());
            }
            for (name, value) in &number_props {
                let key = cf_string(name);
                CFDictionarySetValue(dict, key.get(), value.get());
            }
        }

        // Options 0: the device starts at create time.  Scheduling below only
        // attaches the callback delivery; a set-report that lands in the gap
        // is dropped, which costs nothing; the guest is idle until the host
        // has a device to talk to anyway.
        let device = unsafe { IOHIDUserDeviceCreateWithProperties(std::ptr::null(), dict, 0) };
        if device.is_null() {
            return Err(HidError::Create);
        }

        let refcon = Box::new(tx);
        let refcon_ptr: *const Sender<OutFrame> = &*refcon;

        // Serial queue: set-report callbacks arrive one at a time, which is
        // what the `&Sender` borrow in the callback assumes.
        let label = c"com.cdj3k.emu.pc-link.hid";
        let queue = unsafe { dispatch_queue_create(label.as_ptr(), std::ptr::null()) };
        assert!(!queue.is_null(), "dispatch_queue_create returned NULL");

        unsafe {
            IOHIDUserDeviceRegisterSetReportCallback(
                device,
                set_report_cb,
                refcon_ptr as *mut c_void,
            );
            IOHIDUserDeviceScheduleWithDispatchQueue(device, queue);
        }

        Ok(Self {
            device,
            queue,
            _refcon: refcon,
        })
    }

    /// Deliver a guest-originated input report to macOS.  `report` is the raw
    /// 64-byte payload the guest read off `/dev/hidraw0`; the descriptor has
    /// no report IDs, so there is no leading report-number byte to strip.
    pub fn publish_from_guest(&self, report: &[u8]) {
        if report.is_empty() {
            return;
        }
        let st = unsafe {
            IOHIDUserDeviceHandleReport(self.device, report.as_ptr(), report.len() as CFIndex)
        };
        if st != K_IO_RETURN_SUCCESS {
            eprintln!(
                "pc-link HID: IOHIDUserDeviceHandleReport failed (IOReturn 0x{st:08x}, {} bytes)",
                report.len()
            );
        }
    }
}

impl Drop for HidBackend {
    fn drop(&mut self) {
        unsafe {
            IOHIDUserDeviceUnscheduleFromDispatchQueue(self.device, self.queue);
            // Unschedule stops new callbacks; one already enqueued or running
            // still holds `_refcon`.  The queue is serial, so a synchronous
            // no-op runs only once that one has returned.
            dispatch_sync_f(self.queue, std::ptr::null_mut(), queue_barrier);
            CFRelease(self.device as CFTypeRef);
            dispatch_release(self.queue);
        }
        // _refcon drops here, after the queue drained.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The descriptor must stay byte-identical to the gadget's `report_desc`
    /// in `initramfs-patch/patch-rootfs.d/28-usb-gadget.sh`; a mismatch means
    /// macOS parses a different report layout than the guest emits.
    #[test]
    fn descriptor_matches_gadget() {
        let script = include_str!("../../../../initramfs-patch/patch-rootfs.d/28-usb-gadget.sh");
        let line = script
            .lines()
            .find(|l| l.contains("report_desc"))
            .expect("gadget script writes report_desc");
        let escaped = line
            .split_whitespace()
            .find(|tok| tok.starts_with("\\\\x"))
            .expect("report_desc payload is a \\x-escaped literal");
        let bytes: Vec<u8> = escaped
            .split("\\\\x")
            .filter(|s| !s.is_empty())
            .map(|s| u8::from_str_radix(s, 16).expect("hex byte"))
            .collect();
        assert_eq!(bytes, REPORT_DESCRIPTOR);
    }
}
