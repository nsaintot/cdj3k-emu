//! The host's audio outputs, for the per-instance "Audio Output" picker.
//!
//! The guest is pinned to one device with `-audiodev`, and how a device is
//! addressed differs per backend — so [`AudioOutDevice::uid`] is whatever the
//! host's own backend takes, and the two sources do not name devices alike.

pub mod pipewire;

#[cfg(target_os = "macos")]
pub mod coreaudio;

#[cfg(windows)]
#[path = "windows.rs"]
pub mod wasapi;

/// One output the guest can be pinned to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioOutDevice {
    /// What this host's audio backend addresses the device by: a CoreAudio
    /// UID (`"AppleHDAEngineOutput:1B,0,1,1:0"`), a PipeWire node name, or a
    /// WASAPI endpoint id.
    /// Persisted in `InstanceSettings`, so it has to survive a reboot and a
    /// replug.
    pub uid: String,
    /// Human label, as the desktop's own sound control shows it. Never
    /// persisted — the uid is the identifier.
    pub name: String,
    pub is_default: bool,
    /// Current rate in Hz, or 0 when the source does not report one. The guest
    /// plays 96 kHz S32 and both backends resample, so a sink running at
    /// anything else is audible and otherwise invisible.
    pub sample_rate_hz: u32,
}

#[cfg(target_os = "macos")]
pub use coreaudio::enumerate_output_devices;
#[cfg(target_os = "linux")]
pub use pipewire::enumerate_output_devices;
#[cfg(windows)]
pub use wasapi::enumerate_output_devices;

/// Hosts with no enumeration source. The list stays empty, and the guest
/// follows the system default.
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn enumerate_output_devices() -> Vec<AudioOutDevice> {
    Vec::new()
}
