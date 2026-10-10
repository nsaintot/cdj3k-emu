//! The virtual HID device for the PC Link gadget's HID endpoint.
//!
//! It registers an `IOHIDUserDevice` with the guest gadget's identity and
//! report descriptor ([`GadgetIdentity`]), so that anything on the Mac that
//! opens HID by VID and PID (rekordbox, `hidapi` clients, Console's HID
//! logging) sees the emulated deck as it would see one on a USB-B cable.
//!
//! The device works in both directions:
//!   - From the guest to the host: [`HidBackend::publish_from_guest`] passes
//!     the input reports that the guest read from `/dev/hidraw0` to
//!     `IOHIDUserDeviceHandleReport`.
//!   - From the host to the guest: the set-report callback runs on the
//!     device's dispatch queue for every output report a Mac-side client
//!     writes, and queues a `FRAME_HID` frame for the transport writer thread.
//!
//! `IOHIDUserDevice` is a private IOKit SPI.
//! `IOHIDUserDeviceCreateWithProperties` returns NULL unless the calling
//! binary has the `com.apple.developer.hid.virtual.device` entitlement, backed
//! by a provisioning profile that grants it; [`HidError::Create`] reports
//! that case. A build without the entitlement still runs:
//! [`crate::pc_link::PcLink`] keeps MIDI and drops HID frames.

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
/// `kIOHIDReportTypeOutput`, the only report type that a Mac-side client can
/// send and that the gadget's OUT endpoint carries.
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

// The two attributes link two different frameworks.
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

/// The barrier for [`HidBackend::drop`]. It does nothing except wait its turn
/// on the queue.
extern "C" fn queue_barrier(_context: *mut c_void) {}

/// An owned CoreFoundation reference, released on drop.
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

/// The top-level usage for a descriptor that does not state one.
const DEFAULT_PRIMARY_USAGE: (u32, u32) = (0xff00, 0x01);
// The LocationID comes from `identity::usb_location_id` and must equal the
// CoreMIDI driver's `USBLocationID`. djay pairs a MIDI device with its HID
// sibling by matching VendorID, ProductID and LocationID, so djay's
// `openHIDDevice` does not see the HID device unless the two agree.

// ── Public API ─────────────────────────────────────────────────────────────

/// An error from registering the virtual HID device.
#[derive(Debug)]
pub enum HidError {
    /// `IOHIDUserDeviceCreateWithProperties` returned NULL. This happens
    /// when the binary lacks `com.apple.developer.hid.virtual.device` backed
    /// by an embedded provisioning profile.
    Create,
}

impl std::fmt::Display for HidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Create => write!(
                f,
                "IOHIDUserDeviceCreateWithProperties returned NULL \
                 (the binary needs com.apple.developer.hid.virtual.device \
                 and a provisioning profile)"
            ),
        }
    }
}

impl std::error::Error for HidError {}

/// A live `IOHIDUserDevice` presenting the emulated deck to macOS.
///
/// Creating it registers the device, which appears in IOReg at once.
/// Dropping it unschedules the device and releases the IOKit and dispatch
/// handles.
pub struct HidBackend {
    device: IOHIDUserDeviceRef,
    queue: DispatchQueueRef,
    // The set-report callback holds a raw pointer into this box. The backend
    // owns the box, so it outlives every callback.
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
    // Forward only output reports. The guest bridge writes them to
    // /dev/hidraw0, the OUT endpoint, which has no place for feature reports.
    if report_type != K_IOHID_REPORT_TYPE_OUTPUT {
        return K_IO_RETURN_SUCCESS;
    }
    // SAFETY: refcon is the Box<Sender<OutFrame>> built in `HidBackend::new`,
    // borrowed here. Drop unschedules the device and waits for the serial
    // queue to drain before it releases the box, so no callback outlives it.
    let tx: &Sender<OutFrame> = unsafe { &*(refcon as *const Sender<OutFrame>) };
    let payload = unsafe { std::slice::from_raw_parts(report, report_length as usize) }.to_vec();
    // The send is best-effort. A dead writer thread means that PC Link was
    // turned off or the guest disconnected, and PcLink drops this device
    // shortly after.
    let _ = tx.send((FRAME_HID, payload));
    K_IO_RETURN_SUCCESS
}

/// Build the properties dictionary for `IOHIDUserDeviceCreateWithProperties`:
/// `gadget`'s identity and report descriptor, at the instance's LocationID.
fn device_properties(instance_id: u32, gadget: &GadgetIdentity) -> CFRef {
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

    // The keys are names from IOHIDKeys.h. IOHIDUserDevice reads them from
    // the properties dictionary to build the IOHIDDevice's registry entry. It
    // always sets Transport to "Virtual" in the registry entry; the other
    // values pass through unchanged.
    let props: [(&str, CFRef); 11] = [
        ("ReportDescriptor", cf_data(&gadget.report_descriptor)),
        ("Transport", cf_string("USB")),
        ("Manufacturer", cf_string(&gadget.manufacturer)),
        ("Product", cf_string(&gadget.product)),
        ("SerialNumber", cf_string(&gadget.serial)),
        ("VendorID", cf_number(gadget.vendor_id as i32)),
        ("ProductID", cf_number(gadget.product_id as i32)),
        ("VersionNumber", cf_number(gadget.bcd_device as i32)),
        ("PrimaryUsagePage", cf_number(usage_page as i32)),
        ("PrimaryUsage", cf_number(usage as i32)),
        ("LocationID", cf_number(location_id)),
    ];
    for (name, value) in &props {
        let key = cf_string(name);
        // SAFETY: `dict` is the live dictionary above; it retains key and
        // value.
        unsafe { CFDictionarySetValue(dict, key.get(), value.get()) };
    }
    properties
}

impl HidBackend {
    /// Register the virtual HID device with the guest gadget's identity.
    /// Output reports from Mac-side clients are queued on `tx` for the guest.
    pub fn new(
        tx: Sender<OutFrame>,
        instance_id: u32,
        gadget: &GadgetIdentity,
    ) -> Result<Self, HidError> {
        let properties = device_properties(instance_id, gadget);

        // With options 0, the device starts when it is created, and the
        // scheduling below only attaches callback delivery. A set-report that
        // arrives in between is dropped at no cost, because the guest is idle
        // until the host has a device to talk to.
        let device = unsafe {
            IOHIDUserDeviceCreateWithProperties(
                std::ptr::null(),
                properties.get() as CFMutableDictionaryRef,
                0,
            )
        };
        if device.is_null() {
            return Err(HidError::Create);
        }

        let refcon = Box::new(tx);
        let refcon_ptr: *const Sender<OutFrame> = &*refcon;

        // Use a serial queue, so that set-report callbacks arrive one at a
        // time, as the `&Sender` borrow in the callback assumes.
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

    /// Deliver an input report from the guest to macOS. `report` is the raw
    /// payload that the guest read from `/dev/hidraw0`, as long as the
    /// descriptor's input report. Pioneer's descriptors have no report IDs, so
    /// there is no leading report-number byte to strip.
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
            // Unscheduling stops new callbacks, but one already queued or
            // running still holds `_refcon`. The queue is serial, so a
            // synchronous no-op runs only after that callback has returned.
            dispatch_sync_f(self.queue, std::ptr::null_mut(), queue_barrier);
            CFRelease(self.device as CFTypeRef);
            dispatch_release(self.queue);
        }
        // _refcon drops here, after the queue has drained.
    }
}
