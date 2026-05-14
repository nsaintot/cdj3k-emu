//! Enumerate CoreAudio output devices for the per-instance "Audio Output"
//! picker. UID is the stable identifier across reboots / USB replug; the
//! display name is shown to the user but never persisted.
//!
//! Raw HAL FFI to keep the dep set small (no `coreaudio-sys`). The HAL
//! property API is stable and these constants haven't moved in 15 years.

#![cfg(target_os = "macos")]

use std::ffi::{c_char, c_void, CStr};

// ── Types ────────────────────────────────────────────────────────────────────

type AudioObjectID = u32;
type OSStatus = i32;
type CFStringRef = *const c_void;

#[repr(C)]
struct AudioObjectPropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

#[repr(C)]
struct AudioBuffer {
    number_channels: u32,
    data_byte_size: u32,
    data: *mut c_void,
}

#[repr(C)]
struct AudioBufferList {
    number_buffers: u32,
    buffers: [AudioBuffer; 1], // variable-length tail; we only read `number_buffers`
}

// ── Constants (FourCC) ───────────────────────────────────────────────────────

const fn fourcc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | (s[3] as u32)
}

const K_AUDIO_OBJECT_SYSTEM_OBJECT: AudioObjectID = 1;
const K_AUDIO_HARDWARE_PROPERTY_DEVICES: u32 = fourcc(b"dev#");
const K_AUDIO_HARDWARE_PROPERTY_DEFAULT_OUTPUT_DEVICE: u32 = fourcc(b"dOut");
const K_AUDIO_DEVICE_PROPERTY_DEVICE_UID: u32 = fourcc(b"uid ");
const K_AUDIO_OBJECT_PROPERTY_NAME: u32 = fourcc(b"lnam");
const K_AUDIO_DEVICE_PROPERTY_STREAM_CONFIGURATION: u32 = fourcc(b"slay");
const K_AUDIO_DEVICE_PROPERTY_NOMINAL_SAMPLE_RATE: u32 = fourcc(b"nsrt");

const K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL: u32 = fourcc(b"glob");
const K_AUDIO_OBJECT_PROPERTY_SCOPE_OUTPUT: u32 = fourcc(b"outp");
const K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN: u32 = 0;

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

// ── FFI ──────────────────────────────────────────────────────────────────────

#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioObjectGetPropertyDataSize(
        in_object_id: AudioObjectID,
        in_address: *const AudioObjectPropertyAddress,
        in_qualifier_data_size: u32,
        in_qualifier_data: *const c_void,
        out_data_size: *mut u32,
    ) -> OSStatus;

    fn AudioObjectGetPropertyData(
        in_object_id: AudioObjectID,
        in_address: *const AudioObjectPropertyAddress,
        in_qualifier_data_size: u32,
        in_qualifier_data: *const c_void,
        io_data_size: *mut u32,
        out_data: *mut c_void,
    ) -> OSStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringGetCString(
        the_string: CFStringRef,
        buffer: *mut c_char,
        buffer_size: isize,
        encoding: u32,
    ) -> bool;
    fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
    fn CFStringGetLength(the_string: CFStringRef) -> isize;
    fn CFRelease(cf: *const c_void);
}

// ── Public surface ───────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct AudioOutDevice {
    /// Stable identifier (e.g. `"AppleHDAEngineOutput:1B,0,1,1:0"` or
    /// `"BuiltInSpeakerDevice"`). Persisted in InstanceSettings.
    pub uid: String,
    /// Human label as shown in macOS Sound Preferences.
    pub name: String,
    /// True when this device matches the current system default output.
    pub is_default: bool,
    /// Current nominal sample rate in Hz, or 0 if the property is
    /// unavailable. Shown in the menu so the user can spot devices that
    /// aren't on the guest-expected 96 kHz without opening AMS.
    pub sample_rate_hz: u32,
}

/// Enumerate every device that exposes at least one output stream.
/// Returns the devices sorted by display name for a stable menu order.
pub fn enumerate_output_devices() -> Vec<AudioOutDevice> {
    let mut out = Vec::new();

    let ids = match list_device_ids() {
        Some(v) => v,
        None => return out,
    };
    let default_id = default_output_device_id().unwrap_or(0);

    for id in ids {
        if !device_has_output_streams(id) {
            continue;
        }
        let uid = match read_cf_string(id, K_AUDIO_DEVICE_PROPERTY_DEVICE_UID) {
            Some(s) => s,
            None => continue,
        };
        let name = read_cf_string(id, K_AUDIO_OBJECT_PROPERTY_NAME).unwrap_or_else(|| uid.clone());
        let sample_rate_hz = read_nominal_sample_rate(id).unwrap_or(0);
        out.push(AudioOutDevice {
            uid,
            name,
            is_default: id == default_id,
            sample_rate_hz,
        });
    }

    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

// ── Internals ────────────────────────────────────────────────────────────────

fn list_device_ids() -> Option<Vec<AudioObjectID>> {
    let addr = AudioObjectPropertyAddress {
        selector: K_AUDIO_HARDWARE_PROPERTY_DEVICES,
        scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
        element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut size: u32 = 0;
    let st = unsafe {
        AudioObjectGetPropertyDataSize(
            K_AUDIO_OBJECT_SYSTEM_OBJECT,
            &addr,
            0,
            std::ptr::null(),
            &mut size,
        )
    };
    if st != 0 || size == 0 {
        return None;
    }
    let count = size as usize / std::mem::size_of::<AudioObjectID>();
    let mut ids = vec![0 as AudioObjectID; count];
    let st = unsafe {
        AudioObjectGetPropertyData(
            K_AUDIO_OBJECT_SYSTEM_OBJECT,
            &addr,
            0,
            std::ptr::null(),
            &mut size,
            ids.as_mut_ptr() as *mut c_void,
        )
    };
    if st != 0 {
        return None;
    }
    Some(ids)
}

fn default_output_device_id() -> Option<AudioObjectID> {
    let addr = AudioObjectPropertyAddress {
        selector: K_AUDIO_HARDWARE_PROPERTY_DEFAULT_OUTPUT_DEVICE,
        scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
        element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut id: AudioObjectID = 0;
    let mut size: u32 = std::mem::size_of::<AudioObjectID>() as u32;
    let st = unsafe {
        AudioObjectGetPropertyData(
            K_AUDIO_OBJECT_SYSTEM_OBJECT,
            &addr,
            0,
            std::ptr::null(),
            &mut size,
            &mut id as *mut _ as *mut c_void,
        )
    };
    if st != 0 || id == 0 {
        return None;
    }
    Some(id)
}

fn device_has_output_streams(id: AudioObjectID) -> bool {
    let addr = AudioObjectPropertyAddress {
        selector: K_AUDIO_DEVICE_PROPERTY_STREAM_CONFIGURATION,
        scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_OUTPUT,
        element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut size: u32 = 0;
    let st = unsafe { AudioObjectGetPropertyDataSize(id, &addr, 0, std::ptr::null(), &mut size) };
    if st != 0 || size < std::mem::size_of::<u32>() as u32 {
        return false;
    }
    let mut buf = vec![0u8; size as usize];
    let st = unsafe {
        AudioObjectGetPropertyData(
            id,
            &addr,
            0,
            std::ptr::null(),
            &mut size,
            buf.as_mut_ptr() as *mut c_void,
        )
    };
    if st != 0 {
        return false;
    }
    // AudioBufferList layout: u32 mNumberBuffers + variable AudioBuffer
    // tail. AudioBuffer holds a pointer so it is 8-byte aligned, which means
    // there are 4 padding bytes after mNumberBuffers before mBuffers[0].
    // Walk the buffers via the struct field offset (don't hand-compute from
    // sizeof(u32), or you read 4 bytes early and every channel count looks
    // like garbage, filtering out every device).
    let list = unsafe { &*(buf.as_ptr() as *const AudioBufferList) };
    if list.number_buffers == 0 {
        return false;
    }
    let nb = list.number_buffers as usize;
    let first_buf: *const AudioBuffer = std::ptr::addr_of!(list.buffers) as *const AudioBuffer;
    let mut total_ch = 0u32;
    for i in 0..nb {
        let b = unsafe { &*first_buf.add(i) };
        total_ch = total_ch.saturating_add(b.number_channels);
    }
    total_ch > 0
}

fn read_nominal_sample_rate(id: AudioObjectID) -> Option<u32> {
    let addr = AudioObjectPropertyAddress {
        selector: K_AUDIO_DEVICE_PROPERTY_NOMINAL_SAMPLE_RATE,
        scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
        element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut rate: f64 = 0.0;
    let mut size: u32 = std::mem::size_of::<f64>() as u32;
    let st = unsafe {
        AudioObjectGetPropertyData(
            id,
            &addr,
            0,
            std::ptr::null(),
            &mut size,
            &mut rate as *mut _ as *mut c_void,
        )
    };
    if st != 0 || !(rate > 0.0) {
        return None;
    }
    Some(rate.round() as u32)
}

fn read_cf_string(id: AudioObjectID, selector: u32) -> Option<String> {
    let addr = AudioObjectPropertyAddress {
        selector,
        scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
        element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut size: u32 = std::mem::size_of::<CFStringRef>() as u32;
    let mut cf: CFStringRef = std::ptr::null();
    let st = unsafe {
        AudioObjectGetPropertyData(
            id,
            &addr,
            0,
            std::ptr::null(),
            &mut size,
            &mut cf as *mut _ as *mut c_void,
        )
    };
    if st != 0 || cf.is_null() {
        return None;
    }
    let out = cf_string_to_rust(cf);
    unsafe { CFRelease(cf as *const c_void) };
    out
}

fn cf_string_to_rust(s: CFStringRef) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let len = unsafe { CFStringGetLength(s) };
    if len <= 0 {
        return Some(String::new());
    }
    let max = unsafe { CFStringGetMaximumSizeForEncoding(len, K_CF_STRING_ENCODING_UTF8) } + 1;
    let mut buf = vec![0u8; max as usize];
    let ok = unsafe {
        CFStringGetCString(
            s,
            buf.as_mut_ptr() as *mut c_char,
            max,
            K_CF_STRING_ENCODING_UTF8,
        )
    };
    if !ok {
        return None;
    }
    let cstr = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) };
    Some(cstr.to_string_lossy().into_owned())
}
