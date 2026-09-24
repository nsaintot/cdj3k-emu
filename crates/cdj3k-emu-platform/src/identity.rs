//! Per-instance device identity, shared by everything that has to agree on it.
//!
//! The emmc's U-Boot environment carries the serial the deck reports about
//! itself; the gadget reads it back from there, and the macOS endpoints take
//! it from the gadget.  The rest of the USB identity is the firmware's own
//! (see `cdj3k_emu_runtime::pc_link::gadget`); only what separates one
//! instance from another is derived here.

/// `DJMP{instance_id:06}EH`, the deck's serial number, in Pioneer's format.
pub fn device_serial(instance_id: u32) -> String {
    format!("DJMP{instance_id:06}EH")
}

/// USB location of the emulated deck's PC-link port, one per instance.  DJ
/// apps pair a MIDI device with its HID sibling on VendorID + ProductID +
/// LocationID, so this is what separates two otherwise identical decks.
pub fn usb_location_id(instance_id: u32) -> i32 {
    const BASE: i32 = 19070976; // 0x0123_0000
    BASE + instance_id as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_matches_the_uboot_env_format() {
        assert_eq!(device_serial(0), "DJMP000000EH");
        assert_eq!(device_serial(3), "DJMP000003EH");
    }

    #[test]
    fn each_instance_gets_its_own_usb_location() {
        assert_eq!(usb_location_id(0), 19070976);
        assert_ne!(usb_location_id(0), usb_location_id(1));
    }
}
