//! Hosts with no enumeration source: the list stays empty, and the guest
//! follows the system default.

use super::AudioOutDevice;

pub fn enumerate_output_devices() -> Vec<AudioOutDevice> {
    Vec::new()
}
