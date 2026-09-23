//! PC-link bridge: macOS-side termination of the `cdj3k.usb-link` virtio-
//! serial channel.
//!
//! Presents the emulated CDJ-3000's USB-HID and USB-MIDI interfaces as native
//! macOS virtual endpoints, so HID clients (rekordbox, hidapi) and DAWs see
//! the guest as a USB device on a cable.
//!
//! Both HID and MIDI carry data.  MIDI is published by the CoreMIDI driver
//! plugin alone: apps that walk endpoint → entity → device ignore endpoints
//! made with `MIDISourceCreate`.
//!
//! Layout:
//!   - `frame`:       wire format shared with the guest daemon
//!   - `transport`:   unix-socket I/O threads + framing
//!   - `midi_driver`: link to the CoreMIDI driver plugin
//!   - `hid`:         IOHIDUserDevice virtual HID device
//!
//! The HID backend needs `com.apple.developer.hid.virtual.device` backed by a
//! provisioning profile; without it `HidBackend::new` fails and the bridge
//! runs MIDI-only, dropping HID frames.  MIDI never depends on HID.
//!
//! Lifecycle: a `PcLink` is the endpoints plus the currently connected
//! transport.  [`PcLink::start`] registers the endpoints once;
//! [`PcLink::reconnect`] replaces only the transport, so a QEMU respawn
//! re-dials the socket underneath the same endpoints.
//!
//! rekordbox names a HID device `"<Product>,<LocationID>,<n>"`, where `n` is
//! a counter drawn when the device arrives in IOKit; it polls that name and
//! resolves it later from a deferred alarm.  Re-creating the `IOHIDUserDevice`
//! between those moments invalidates the name and rekordbox never converges.

pub mod frame;
pub mod transport;

#[cfg(target_os = "macos")]
pub mod hid;
#[cfg(target_os = "macos")]
pub mod midi_driver;

#[cfg(target_os = "macos")]
mod imp {
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::{self, Receiver};
    use std::sync::Arc;

    use super::hid::HidBackend;
    use super::midi_driver::MidiDriverLink;
    use super::transport::{usb_link_sock_path, FrameSink, OutFrame, Transport};

    /// Marker traits: HidBackend holds an IOHIDUserDeviceRef and a serial
    /// dispatch queue, both refcounted IOKit/libdispatch handles that are
    /// safe to use from any thread.
    unsafe impl Send for HidBackend {}
    unsafe impl Sync for HidBackend {}

    /// Sink invoked by the reader thread on each demuxed guest→host frame;
    /// owns the OS-visible endpoints.  `hid` is `None` when the virtual-HID
    /// device could not be registered; HID frames drop, MIDI carries on.
    ///
    /// Shared via `Arc` with every transport the bridge dials, so `reconnect`
    /// cannot destroy an endpoint.
    struct PcLinkSink {
        hid: Option<Arc<HidBackend>>,
        /// The CoreMIDI driver plugin, when the socket could be bound.  It is
        /// what apps matching on endpoint -> entity -> device can see.
        driver: Option<Arc<MidiDriverLink>>,
    }

    impl FrameSink for PcLinkSink {
        fn on_hid(&self, payload: &[u8]) {
            if let Some(hid) = &self.hid {
                hid.publish_from_guest(payload);
            }
        }
        fn on_midi(&self, payload: &[u8]) {
            if let Some(d) = &self.driver {
                d.publish_from_guest(payload);
            }
        }
    }

    /// Errors starting or re-dialing the PC-link bridge.
    #[derive(Debug)]
    pub enum PcLinkError {
        Io(std::io::Error),
        /// The outbound channel's `Receiver` is gone; every backend `send`
        /// now fails.  Only a full rebuild recovers (fresh IOKit device).
        ReceiverLost,
    }

    impl std::fmt::Display for PcLinkError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Io(e) => write!(f, "pc-link I/O: {e}"),
                Self::ReceiverLost => write!(f, "pc-link outbound channel lost"),
            }
        }
    }

    impl std::error::Error for PcLinkError {}

    impl From<std::io::Error> for PcLinkError {
        fn from(e: std::io::Error) -> Self {
            Self::Io(e)
        }
    }

    impl PcLinkError {
        /// Whether another attempt can succeed without rebuilding the bridge.
        /// A refused dial is retryable; a lost channel is not.
        pub fn is_retryable(&self) -> bool {
            !matches!(self, Self::ReceiverLost)
        }
    }

    /// Upper bound on frames discarded per re-dial, so a flooding client
    /// cannot wedge the caller's thread.
    const DRAIN_CAP: usize = 4096;

    /// Discard host→guest frames queued while the link was down.  A reconnect
    /// follows a QEMU exit, so these target a guest that no longer exists.
    fn drain_stale(rx: &Receiver<OutFrame>) -> usize {
        let mut n = 0;
        while n < DRAIN_CAP && rx.try_recv().is_ok() {
            n += 1;
        }
        n
    }

    /// The transport half of a `PcLink`, which the endpoints outlive.
    enum Link {
        /// Connected; the `Receiver` lives inside the writer thread.
        Connected(Transport),
        /// Reclaimed and waiting for a re-dial.
        Dormant(Receiver<OutFrame>),
        /// The `Receiver` was lost with a panicking writer thread.
        Broken,
    }

    /// Running PC-link bridge.  Owns the OS-visible endpoints for as long as
    /// the user has PC-link enabled, and swaps its transport underneath them.
    pub struct PcLink {
        /// `runtime_paths::instance_dir(id)`; carries no PID and is not
        /// rewritten on respawn, so it stays correct across a re-dial.
        sock_dir: PathBuf,
        sink: Arc<PcLinkSink>,
        link: Link,
    }

    impl PcLink {
        /// Connect to the QEMU-exposed unix socket under `sock_dir` and
        /// register macOS-side virtual endpoints.
        pub fn start(sock_dir: &Path, instance_id: u32) -> Result<Self, PcLinkError> {
            let sock_path = usb_link_sock_path(sock_dir);

            // Dial before building anything: the caller retries start() at the
            // worker poll cadence while the guest boots, and every backend
            // built here is an endpoint macOS shows to other processes.
            let stream = Transport::dial(&sock_path)?;

            let (tx, rx) = mpsc::channel::<OutFrame>();

            // HID is optional: without the Apple-granted entitlement the
            // device cannot be registered, and the bridge is still useful as
            // a MIDI cable.  Report once and carry on.
            let hid = match HidBackend::new(tx.clone(), instance_id) {
                Ok(h) => Some(Arc::new(h)),
                Err(e) => {
                    eprintln!("cdj3k-emu: pc-link HID unavailable — {e}");
                    None
                }
            };

            // Install the driver plugin (first toggle, or after an update)
            // before opening the link MIDIServer will connect back over.
            super::midi_driver::ensure_driver_installed();

            // The driver link is optional: without it HID still carries.
            let driver = match MidiDriverLink::start(sock_dir, tx.clone(), instance_id) {
                Ok(d) => Some(Arc::new(d)),
                Err(e) => {
                    eprintln!("cdj3k-emu: pc-link MIDI driver link unavailable - {e}");
                    None
                }
            };

            // With neither backend there is nothing for macOS to see, so
            // this is a failure the caller can retry.
            if hid.is_none() && driver.is_none() {
                return Err(PcLinkError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "neither the HID device nor the MIDI driver link could be created",
                )));
            }

            // Drop the local tx clone; the backends' clones are the producers.
            drop(tx);

            let sink = Arc::new(PcLinkSink { hid, driver });
            let transport = Transport::spawn(stream, rx, sink.clone())?;

            Ok(Self {
                sock_dir: sock_dir.to_path_buf(),
                sink,
                link: Link::Connected(transport),
            })
        }

        /// Discard frames queued while the link is down.  The backends keep
        /// publishing into the channel whether or not a transport is attached,
        /// and only a re-dial drains it otherwise.
        pub fn drain_if_dormant(&self) -> usize {
            match &self.link {
                Link::Dormant(rx) => drain_stale(rx),
                _ => 0,
            }
        }

        /// Whether the current transport's I/O threads are both running.
        /// False after QEMU exits and the reader takes EOF; the caller's
        /// signal to [`PcLink::reconnect`].
        pub fn is_alive(&self) -> bool {
            matches!(&self.link, Link::Connected(t) if t.is_transport_alive())
        }

        /// Re-dial the guest socket, keeping the endpoints and their channel.
        /// All errors are retryable except [`PcLinkError::ReceiverLost`].
        pub fn reconnect(&mut self) -> Result<(), PcLinkError> {
            // Join the old transport before dialing, so at most one reader
            // owns the sink at a time.
            let rx = match std::mem::replace(&mut self.link, Link::Broken) {
                Link::Connected(mut t) => t.shutdown_into_rx().ok_or(PcLinkError::ReceiverLost)?,
                Link::Dormant(rx) => rx,
                Link::Broken => return Err(PcLinkError::ReceiverLost),
            };

            let dropped = drain_stale(&rx);
            if dropped > 0 {
                eprintln!("cdj3k-emu: pc-link re-dial dropped {dropped} stale host->guest frames");
            }

            let stream = match Transport::dial(&usb_link_sock_path(&self.sock_dir)) {
                Ok(s) => s,
                Err(e) => {
                    // Hold `rx` for the next attempt.  Dropping it closes the
                    // channel and kills every backend's send() for good.
                    self.link = Link::Dormant(rx);
                    return Err(PcLinkError::Io(e));
                }
            };

            match Transport::spawn(stream, rx, self.sink.clone()) {
                Ok(t) => {
                    self.link = Link::Connected(t);
                    Ok(())
                }
                // spawn took `rx` by value and could not hand it back.
                Err(e) => {
                    eprintln!("cdj3k-emu: pc-link transport spawn failed: {e}");
                    Err(PcLinkError::ReceiverLost)
                }
            }
        }
    }

    impl Drop for PcLink {
        fn drop(&mut self) {
            // Join the I/O threads before any IOKit/CoreMIDI handle is
            // released: a reader inside sink.on_hid() ->
            // IOHIDUserDeviceHandleReport during CFRelease is UB.
            self.link = Link::Broken;
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::{PcLink, PcLinkError};

#[cfg(not(target_os = "macos"))]
mod imp_stub {
    use std::path::Path;

    #[derive(Debug)]
    pub enum PcLinkError {
        Unsupported,
    }

    impl std::fmt::Display for PcLinkError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "pc-link bridge requires macOS")
        }
    }
    impl std::error::Error for PcLinkError {}

    impl PcLinkError {
        pub fn is_retryable(&self) -> bool {
            false
        }
    }

    pub struct PcLink;

    impl PcLink {
        pub fn start(_sock_dir: &Path, _instance_id: u32) -> Result<Self, PcLinkError> {
            Err(PcLinkError::Unsupported)
        }

        pub fn is_alive(&self) -> bool {
            false
        }

        pub fn reconnect(&mut self) -> Result<(), PcLinkError> {
            Err(PcLinkError::Unsupported)
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub use imp_stub::{PcLink, PcLinkError};
