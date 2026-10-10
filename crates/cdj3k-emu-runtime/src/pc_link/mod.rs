//! The PC Link bridge, which is the host end of the `cdj3k.usb-link`
//! virtio-serial channel.
//!
//! It presents the emulated deck's USB HID and USB MIDI interfaces as virtual
//! endpoints on the host, so that HID clients (rekordbox, hidapi) and DAWs
//! see the guest as a USB device on a cable. Both endpoints carry the
//! identity that the guest gadget reports ([`gadget`]), which is the
//! firmware's own.
//!
//! Modules:
//!   - `frame`: the wire format shared with the guest daemon
//!   - `gadget`: the gadget identity that the guest reports on connect
//!   - `transport`: the Unix socket I/O threads and framing (macOS builds)
//!
//! [`PcLink`] holds the endpoints and the currently connected transport.
//! [`PcLink::start`] registers the endpoints once; [`PcLink::reconnect`]
//! replaces only the transport, so a QEMU respawn re-dials the socket
//! underneath the same endpoints.
//!
//! Only macOS has endpoints: an `IOHIDUserDevice` and a CoreMIDI driver
//! plugin (see `macos.rs`). On other hosts, [`PcLink::start`] fails as
//! unsupported.

pub mod frame;
pub mod gadget;
/// The transport talks to QEMU over a Unix socket, which needs
/// `std::os::unix::net`, so it exists only on macOS builds.
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
