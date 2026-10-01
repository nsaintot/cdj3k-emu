//! PC-link bridge: host-side termination of the `cdj3k.usb-link` virtio-
//! serial channel.
//!
//! Presents the emulated deck's USB-HID and USB-MIDI interfaces as virtual
//! endpoints on the host, so HID clients (rekordbox, hidapi) and DAWs see the
//! guest as a USB device on a cable.  Both carry the identity the guest gadget
//! reports ([`gadget`]), which is the firmware's own.
//!
//! Layout:
//!   - `frame`:       wire format shared with the guest daemon
//!   - `gadget`:      the gadget identity the guest reports on connect
//!   - `transport`:   unix-socket I/O threads + framing (macOS builds)
//!
//! [`PcLink`] is the endpoints plus the currently connected transport.
//! [`PcLink::start`] registers the endpoints once; [`PcLink::reconnect`]
//! replaces only the transport, so a QEMU respawn re-dials the socket
//! underneath the same endpoints.
//!
//! Only macOS builds endpoints ([`macos`], `IOHIDUserDevice` + a CoreMIDI
//! driver plugin); everywhere else [`PcLink::start`] fails as unsupported.

pub mod frame;
pub mod gadget;
/// macOS-only: vends the endpoints over a UNIX socket, which needs
/// `std::os::unix::net`.
#[cfg(target_os = "macos")]
mod transport;

#[cfg(target_os = "macos")]
mod hid;
#[cfg(target_os = "macos")]
mod midi_driver;

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;
#[cfg(not(target_os = "macos"))]
#[path = "unsupported.rs"]
mod imp;

pub use imp::{PcLink, PcLinkError, SUPPORTED};

/// Whether this build can publish virtual endpoints.
pub fn is_supported() -> bool {
    SUPPORTED
}
