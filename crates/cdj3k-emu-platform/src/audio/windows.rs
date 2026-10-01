//! WASAPI render endpoints, for the per-instance Audio Output picker.
//!
//! QEMU's `wasapi` audiodev takes `out.dev=<endpoint id>`, the string
//! `IMMDevice::GetId` returns, so that string is the uid. The rate is left at
//! 0: the mix format is only known by opening the endpoint.

use windows::core::PWSTR;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    eConsole, eRender, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM_READ,
};

use super::AudioOutDevice;

/// Every active render endpoint, in name order.
pub fn enumerate_output_devices() -> Vec<AudioOutDevice> {
    // SAFETY: balanced below, and only when this call initialised COM (a
    // thread already in another apartment mode keeps its own).
    let initialised = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    let mut out = enumerate().unwrap_or_default();
    if initialised {
        // SAFETY: pairs the successful `CoInitializeEx` above.
        unsafe { CoUninitialize() };
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.uid == b.uid);
    out
}

fn enumerate() -> windows::core::Result<Vec<AudioOutDevice>> {
    // SAFETY: COM calls on interfaces this frame owns; every returned string
    // is freed by `take_string`.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let default = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .ok()
            .and_then(|d| device_id(&d));
        let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        let mut out = Vec::new();
        for i in 0..devices.GetCount()? {
            let device = devices.Item(i)?;
            let Some(uid) = device_id(&device) else {
                continue;
            };
            out.push(AudioOutDevice {
                name: friendly_name(&device).unwrap_or_else(|| uid.clone()),
                is_default: default.as_ref() == Some(&uid),
                uid,
                sample_rate_hz: 0,
            });
        }
        Ok(out)
    }
}

unsafe fn device_id(device: &IMMDevice) -> Option<String> {
    take_string(device.GetId().ok()?)
}

unsafe fn friendly_name(device: &IMMDevice) -> Option<String> {
    let store = device.OpenPropertyStore(STGM_READ).ok()?;
    let value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
    take_string(PropVariantToStringAlloc(&value).ok()?)
}

/// A COM-allocated wide string as a `String`, freeing it.
unsafe fn take_string(s: PWSTR) -> Option<String> {
    let text = s.to_string().ok();
    CoTaskMemFree(Some(s.as_ptr().cast()));
    text.filter(|t| !t.is_empty())
}
