//! Virtual HID device for the PC-link gadget HID endpoint.
//!
//! Registers an `IOHIDUserDevice` carrying the guest gadget's identity and
//! report descriptor ([`GadgetIdentity`]), so anything on the Mac that opens HID by VID/PID (rekordbox,
//! `hidapi` clients, Console's HID logging) sees the emulated deck exactly as
//! it would see one on a USB-B cable.
//!
//! Two directions:
//!   - guest → host: [`HidBackend::publish_from_guest`] feeds the input
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

use std::os::raw::{c_char, c_void};
use std::sync::mpsc::Sender;

use crate::pc_link::frame::FRAME_HID;
use crate::pc_link::gadget::GadgetIdentity;
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

// Two frameworks, not one attribute twice.
#[allow(clippy::duplicated_attributes)]
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

/// Top-level usage for a descriptor that does not state one.
const DEFAULT_PRIMARY_USAGE: (u32, u32) = (0xff00, 0x01);
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

/// A live `IOHIDUserDevice` presenting the emulated deck to macOS.
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
    /// Register the virtual HID device with the guest gadget's identity.
    /// `tx` is where output reports from Mac-side clients are enqueued for
    /// the guest.
    pub fn new(
        tx: Sender<OutFrame>,
        instance_id: u32,
        gadget: &GadgetIdentity,
    ) -> Result<Self, HidError> {
        let (usage_page, usage) = gadget.primary_usage().unwrap_or(DEFAULT_PRIMARY_USAGE);
        let location_id = cdj3k_emu_platform::identity::usb_location_id(instance_id);
        let properties = unsafe {
            CFDictionaryCreateMutable(
                std::ptr::null(),
                0,
                &kCFTypeDictionaryKeyCallBacks as *const c_void,
                &kCFTypeDictionaryValueCallBacks as *const c_void,
            )
        };
        assert!(
            !properties.is_null(),
            "CFDictionaryCreateMutable returned NULL"
        );
        let properties = CFRef(properties as CFTypeRef);
        let dict = properties.get() as CFMutableDictionaryRef;

        let descriptor = cf_data(&gadget.report_descriptor);
        // IOHIDKeys.h names; IOHIDUserDevice reads them straight out of the
        // properties dictionary to build the IOHIDDevice's registry entry.
        // IOHIDUserDevice overwrites Transport with "Virtual" in the registry
        // entry regardless of what is passed here; the rest come through
        // unchanged.
        let string_props: [(&str, CFRef); 4] = [
            ("Transport", cf_string("USB")),
            ("Manufacturer", cf_string(&gadget.manufacturer)),
            ("Product", cf_string(&gadget.product)),
            ("SerialNumber", cf_string(&gadget.serial)),
        ];
        let number_props: [(&str, CFRef); 6] = [
            ("VendorID", cf_number(gadget.vendor_id as i32)),
            ("ProductID", cf_number(gadget.product_id as i32)),
            ("VersionNumber", cf_number(gadget.bcd_device as i32)),
            ("PrimaryUsagePage", cf_number(usage_page as i32)),
            ("PrimaryUsage", cf_number(usage as i32)),
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
    /// payload the guest read off `/dev/hidraw0`, as long as the descriptor's
    /// input report; Pioneer's descriptors have no report IDs, so there is no
    /// leading report-number byte to strip.
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
