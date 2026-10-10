//! The macOS endpoints: an `IOHIDUserDevice` for HID and a CoreMIDI driver
//! plugin for MIDI.
//!
//! Only the CoreMIDI driver plugin publishes MIDI, because apps that walk
//! from an endpoint to its entity and then its device ignore endpoints made
//! with `MIDISourceCreate`.
//!
//! The HID back end needs `com.apple.developer.hid.virtual.device` backed by a
//! provisioning profile. Without it, `HidBackend::new` fails, and the bridge
//! carries only MIDI and drops HID frames. MIDI never depends on HID.
//!
//! rekordbox names a HID device `"<Product>,<LocationID>,<n>"`, where `n` is
//! a counter taken when the device arrives in IOKit. It polls that name and
//! resolves it later from a deferred alarm. If the `IOHIDUserDevice` is
//! re-created in between, the name becomes invalid and rekordbox never finds
//! the device.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use super::gadget::GadgetIdentity;
use super::hid::HidBackend;
use super::midi_driver::MidiDriverLink;
use super::transport::{usb_link_sock_path, FrameSink, OutFrame, Transport};

/// HidBackend holds an IOHIDUserDeviceRef and a serial dispatch queue. Both
/// are refcounted IOKit and libdispatch handles that are safe to use from any
/// thread.
unsafe impl Send for HidBackend {}
unsafe impl Sync for HidBackend {}

/// The sink that the reader thread calls for each demultiplexed frame from
/// the guest. It owns the endpoints that macOS shows. `hid` is `None` when the
/// virtual HID device could not be registered; HID frames are then dropped
/// and MIDI keeps working.
///
/// Every transport that the bridge dials shares it through an `Arc`, so
/// `reconnect` cannot destroy an endpoint.
struct PcLinkSink {
    hid: Option<Arc<HidBackend>>,
    /// The CoreMIDI driver plugin, when the socket could be bound. Apps that
    /// match from an endpoint to its entity and device see only this.
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

/// An error from starting or re-dialing the PC Link bridge.
#[derive(Debug)]
pub enum PcLinkError {
    Io(std::io::Error),
    /// The outbound channel's `Receiver` is gone, so every back end's `send`
    /// fails. Only a full rebuild, with a new IOKit device, recovers from it.
    ReceiverLost,
}

impl std::fmt::Display for PcLinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "pc-link I/O: {e}"),
            Self::ReceiverLost => write!(f, "the pc-link outbound channel was lost"),
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

/// The most frames discarded per re-dial, so that a flooding client cannot
/// block the caller's thread.
const DRAIN_CAP: usize = 4096;

/// Discard the frames for the guest that were queued while the link was
/// down. A reconnect follows a QEMU exit, so they target a guest that no
/// longer exists.
fn drain_stale(rx: &Receiver<OutFrame>) -> usize {
    let mut n = 0;
    while n < DRAIN_CAP && rx.try_recv().is_ok() {
        n += 1;
    }
    n
}

/// The transport half of a `PcLink`, which the endpoints outlive.
enum Link {
    /// The transport is connected, and the writer thread holds the `Receiver`.
    Connected(Transport),
    /// The `Receiver` was reclaimed and waits for a re-dial.
    Dormant(Receiver<OutFrame>),
    /// The `Receiver` was lost with a panicking writer thread.
    Broken,
}

/// This build has endpoints to publish, so the menu offers the switch.
pub const SUPPORTED: bool = true;

/// A running PC Link bridge. It owns the endpoints that macOS shows for as
/// long as the user has PC Link on, and swaps its transport underneath them.
pub struct PcLink {
    /// The directory from `runtime_paths::instance_dir(id)`. Its path has no
    /// PID in it and does not change on respawn, so it stays correct across a
    /// re-dial.
    sock_dir: PathBuf,
    /// The identity that the endpoints were built from; a re-dial compares
    /// the new identity with it.
    identity: GadgetIdentity,
    sink: Arc<PcLinkSink>,
    link: Link,
}

impl PcLink {
    /// Connect to QEMU's Unix socket under `sock_dir` and register the
    /// virtual endpoints on macOS.
    pub fn start(sock_dir: &Path, instance_id: u32) -> Result<Self, PcLinkError> {
        let sock_path = usb_link_sock_path(sock_dir);

        // Dial before building anything. The caller retries start() at the
        // worker's poll interval while the guest boots, and every back end
        // built here is an endpoint that other processes on macOS see.
        let stream = Transport::dial(&sock_path)?;
        let identity = Transport::handshake(&stream)?;
        eprintln!(
            "cdj3k-emu: the pc-link gadget is {} {:04x}:{:04x}, with a {}-byte report descriptor.",
            identity.product,
            identity.vendor_id,
            identity.product_id,
            identity.report_descriptor.len()
        );

        let (tx, rx) = mpsc::channel::<OutFrame>();

        // HID is optional. Without the entitlement from Apple, the device
        // cannot be registered, but the bridge still works as a MIDI cable,
        // so report the error once and continue.
        let hid = match HidBackend::new(tx.clone(), instance_id, &identity) {
            Ok(h) => Some(Arc::new(h)),
            Err(e) => {
                eprintln!("cdj3k-emu: the pc-link HID device is unavailable: {e}");
                None
            }
        };

        // Install the driver plugin, on the first toggle or after an update,
        // before opening the link that MIDIServer connects back over.
        super::midi_driver::ensure_driver_installed();

        // The driver link is optional; without it, HID still works.
        let driver = match MidiDriverLink::start(sock_dir, tx.clone(), instance_id, &identity) {
            Ok(d) => Some(Arc::new(d)),
            Err(e) => {
                eprintln!("cdj3k-emu: the pc-link MIDI driver link is unavailable: {e}");
                None
            }
        };

        // With neither backend there is nothing for macOS to see, so
        // this is a failure the caller can retry.
        if hid.is_none() && driver.is_none() {
            return Err(PcLinkError::Io(std::io::Error::other(
                "neither the HID device nor the MIDI driver link could be created",
            )));
        }

        // Drop the local sender; the back ends hold the remaining senders.
        drop(tx);

        let sink = Arc::new(PcLinkSink { hid, driver });
        let transport = Transport::spawn(stream, rx, sink.clone())?;

        Ok(Self {
            sock_dir: sock_dir.to_path_buf(),
            identity,
            sink,
            link: Link::Connected(transport),
        })
    }

    /// Discard frames queued while the guest is down.  The backends keep
    /// publishing into the channel whether or not a transport is attached,
    /// so a dead transport is reclaimed first and its channel drained.
    pub fn drain_while_down(&mut self) -> usize {
        if let Link::Connected(t) = &self.link {
            if t.is_transport_alive() {
                return 0;
            }
            self.link = match std::mem::replace(&mut self.link, Link::Broken) {
                Link::Connected(mut t) => t.shutdown_into_rx().map_or(Link::Broken, Link::Dormant),
                other => other,
            };
        }
        match &self.link {
            Link::Dormant(rx) => drain_stale(rx),
            _ => 0,
        }
    }

    /// Whether the current transport's I/O threads are both running.
    /// It turns false after QEMU exits and the reader reaches EOF, which tells
    /// the caller to call [`PcLink::reconnect`].
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
            eprintln!("cdj3k-emu: the pc-link re-dial dropped {dropped} stale frames queued for the guest.");
        }

        let dialed = Transport::dial(&usb_link_sock_path(&self.sock_dir))
            .and_then(|s| Transport::handshake(&s).map(|id| (s, id)));
        let stream = match dialed {
            Ok((s, id)) => {
                // The endpoints stay as registered: re-creating them is
                // what rekordbox cannot follow (see the module doc).
                if id != self.identity {
                    eprintln!(
                        "cdj3k-emu: the pc-link gadget identity changed on re-dial \
                         (from {} to {}); the endpoints keep the first identity.",
                        self.identity.product, id.product
                    );
                }
                s
            }
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
                eprintln!("cdj3k-emu: could not start the pc-link transport: {e}");
                Err(PcLinkError::ReceiverLost)
            }
        }
    }
}

impl Drop for PcLink {
    fn drop(&mut self) {
        // Join the I/O threads before any IOKit or CoreMIDI handle is
        // released. A reader still inside sink.on_hid(), calling
        // IOHIDUserDeviceHandleReport, during CFRelease is undefined
        // behaviour.
        self.link = Link::Broken;
    }
}
