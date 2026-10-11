//! The link between `PcLink` and the CoreMIDI driver plugin.
//!
//! The driver runs inside `MIDIServer`, so it cannot share the emulator's
//! virtio-serial socket. It connects here instead, over a Unix socket that
//! carries a raw MIDI byte stream in both directions: the same bytes the guest reads
//! from and writes to the gadget's rawmidi device, with no extra framing.
//!
//! A driver is needed because apps that walk from an endpoint to its entity
//! and device, such as djay Pro, ignore endpoints made with
//! `MIDISourceCreate`: a virtual endpoint has no entity, and
//! `MIDIDeviceCreate` is refused outside a driver. See `docs/pc-link.md`.
//!
//! The socket lives at `{instance_dir}/midi-driver.sock`, beside the instance's
//! other channels. Binding it tells the plugin that PC Link is on for the
//! instance: the plugin scans `instance-*` and dials each one it finds, so every
//! instance gets its own connection and its own `MIDIDevice`, and no instance
//! can disturb another's.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use crate::pc_link::frame::FRAME_MIDI;
use crate::pc_link::gadget::GadgetIdentity;
use crate::pc_link::transport::OutFrame;

/// The longest a write to the attached driver may take. A driver that stops
/// reading is dropped.
const WRITE_TIMEOUT: Duration = Duration::from_millis(250);

/// How often the accept thread checks the stop flag. It also limits how long
/// `drop` waits to join that thread.
const ACCEPT_POLL: Duration = Duration::from_millis(200);

/// The pause after a failed accept (EMFILE, ENFILE, ...), so that a lasting
/// error does not spin the accept thread.
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(100);

/// The number of messages from the guest held for a slow driver. Beyond this,
/// the oldest are dropped, because late MIDI is of no use.
const OUTBOX_CAP: usize = 1024;

/// The queue between the transport reader and the driver writer thread. The
/// reader also dispatches HID, and `FrameSink` must not block.
#[derive(Default)]
struct Outbox {
    queue: Mutex<VecDeque<Vec<u8>>>,
    ready: Condvar,
}

impl Outbox {
    fn push(&self, bytes: &[u8]) {
        let mut q = self.queue.lock().unwrap();
        if q.len() >= OUTBOX_CAP {
            q.pop_front();
        }
        q.push_back(bytes.to_vec());
        self.ready.notify_one();
    }

    /// Return the next message, or `None` once `stop` is set and the queue is
    /// empty.
    fn pop(&self, stop: &AtomicBool) -> Option<Vec<u8>> {
        let mut q = self.queue.lock().unwrap();
        loop {
            if let Some(v) = q.pop_front() {
                return Some(v);
            }
            if stop.load(Ordering::Acquire) {
                return None;
            }
            let (next, _) = self.ready.wait_timeout(q, WRITE_TIMEOUT).unwrap();
            q = next;
        }
    }
}

/// The identity sent to the driver plugin when it connects. It carries every
/// string and ID that the plugin puts on its `MIDIDevice`, so the plugin holds
/// no identity of its own and a rebrand needs no new plugin binary. Its size
/// is fixed, because the raw MIDI byte stream follows it with no framing.
#[repr(C)]
struct Identity {
    magic: u32,
    version: u16,
    instance: u16,
    vid: u16,
    pid: u16,
    location: u32,
    product: [u8; 32],
    serial: [u8; 32],
    manufacturer: [u8; 32],
}

const IDENTITY_MAGIC: u32 = 0x4344_4a31; // 'CDJ1'

/// The plugin parses this layout byte for byte. A size change here without a
/// matching change in tools/midi-driver breaks the protocol silently.
const _: () = assert!(std::mem::size_of::<Identity>() == 112);
const IDENTITY_VERSION: u16 = 1;

impl Identity {
    fn new(instance_id: u32, gadget: &GadgetIdentity) -> Self {
        let mut product = [0u8; 32];
        let mut serial = [0u8; 32];
        let mut manufacturer = [0u8; 32];
        copy_field(&mut product, &gadget.product);
        copy_field(&mut serial, &gadget.serial);
        copy_field(&mut manufacturer, &gadget.manufacturer);
        Self {
            magic: IDENTITY_MAGIC.to_le(),
            version: IDENTITY_VERSION.to_le(),
            instance: (instance_id as u16).to_le(),
            vid: gadget.vendor_id.to_le(),
            pid: gadget.product_id.to_le(),
            location: (cdj3k_emu_platform::identity::usb_location_id(instance_id) as u32).to_le(),
            product,
            serial,
            manufacturer,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        // SAFETY: `Identity` is `#[repr(C)]` plain data with no padding, and
        // the driver parses the same layout.
        unsafe {
            std::slice::from_raw_parts(
                self as *const Self as *const u8,
                std::mem::size_of::<Self>(),
            )
        }
    }
}

/// Copy `src` into the zeroed `dst`, truncated so that at least one NUL byte
/// ends it.
fn copy_field(dst: &mut [u8; 32], src: &str) {
    let n = src.len().min(dst.len() - 1);
    dst[..n].copy_from_slice(&src.as_bytes()[..n]);
}

/// The socket the driver plugin dials for this instance.
pub fn midi_driver_sock_path(sock_dir: &Path) -> PathBuf {
    sock_dir.join("midi-driver.sock")
}

/// Bind `path`, removing a socket left behind by a crash. A socket that still
/// answers `connect` belongs to a live process and is left alone.
fn bind_clearing_stale(path: &Path) -> std::io::Result<UnixListener> {
    match UnixListener::bind(path) {
        Ok(l) => return Ok(l),
        Err(e) if e.kind() != std::io::ErrorKind::AddrInUse => return Err(e),
        Err(e) => {
            if UnixStream::connect(path).is_ok() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    format!("{} is owned by a live process", path.display()),
                ));
            }
            let _ = std::fs::remove_file(path);
            eprintln!("pc-link: removed the stale socket {}: {e}", path.display());
        }
    }
    UnixListener::bind(path)
}

const PLUGIN_NAME: &str = "CDJ3KEmuMIDI.plugin";

/// Install or refresh the CoreMIDI driver plugin in the per-user MIDI Drivers
/// directory, where MIDIServer loads it from. The plugin ships in the app
/// bundle's Resources. An unbundled development build does nothing; install
/// the plugin by hand with `make -C tools/midi-driver install`. MIDIServer
/// picks up a new or changed plugin on its next client connection.
pub fn ensure_driver_installed() {
    let Some(src) = shipped_plugin() else { return };
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let dest_dir = PathBuf::from(home).join("Library/Audio/MIDI Drivers");
    let dest = dest_dir.join(PLUGIN_NAME);

    if installed_plugin_version(&dest) == Some(IDENTITY_VERSION) {
        return;
    }

    let _ = std::fs::create_dir_all(&dest_dir);
    // Each process stages its own copy outside the directory that MIDIServer
    // scans for bundles, because instances can install at the same time.
    let staged = dest_dir
        .parent()
        .unwrap_or(&dest_dir)
        .join(format!("{PLUGIN_NAME}.{}.staging", std::process::id()));
    let _ = std::fs::remove_dir_all(&staged);
    // ditto preserves the bundle's code signature; a plain recursive copy does not.
    match std::process::Command::new("/usr/bin/ditto")
        .arg(&src)
        .arg(&staged)
        .status()
    {
        Ok(s) if s.success() => {
            let _ = std::fs::remove_dir_all(&dest);
            if let Err(e) = std::fs::rename(&staged, &dest) {
                let _ = std::fs::remove_dir_all(&staged);
                eprintln!("pc-link: could not install the MIDI driver: {e}");
                return;
            }
            eprintln!("pc-link: installed the MIDI driver at {}.", dest.display());
            // A running MIDIServer keeps serving the previous binary, which it
            // has mapped, until it exits. Ending it drops every MIDI app's
            // connection, and the apps do not reconnect.
            if midi_server_running() {
                cdj3k_emu_platform::menu_state::lock().midi_driver_replaced = true;
            }
        }
        Ok(s) => {
            let _ = std::fs::remove_dir_all(&staged);
            eprintln!("pc-link: could not install the MIDI driver (ditto {s})");
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staged);
            eprintln!("pc-link: could not install the MIDI driver: {e}");
        }
    }
}

/// Return the plugin shipped in the running app bundle
/// (`<app>.app/Contents/Resources/CDJ3KEmuMIDI.plugin`), or `None` when the
/// app runs unbundled or the executable path cannot be resolved.
fn shipped_plugin() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // The executable is <app>/Contents/MacOS/<exe>, and the plugin is
    // <app>/Contents/Resources/<plugin>.
    let plugin = exe.parent()?.parent()?.join("Resources").join(PLUGIN_NAME);
    plugin.is_dir().then_some(plugin)
}

/// Return the wire format version that an installed plugin declares in
/// `CDJ3KEmuMIDIVersion` in its `Info.plist`, or `None` when the key is
/// missing or cannot be parsed.
fn installed_plugin_version(plugin: &Path) -> Option<u16> {
    let plist = std::fs::read_to_string(plugin.join("Contents/Info.plist")).ok()?;
    let key = "<key>CDJ3KEmuMIDIVersion</key><string>";
    let rest = plist.split(key).nth(1)?;
    rest.split('<').next()?.trim().parse().ok()
}

/// Whether a `MIDIServer` process exists.
fn midi_server_running() -> bool {
    std::process::Command::new("/usr/bin/pgrep")
        .args(["-x", "MIDIServer"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Accepts the driver plugin and carries MIDI both ways.
pub struct MidiDriverLink {
    /// The write half of the connected driver, if one has attached.
    client: Client,
    outbox: Arc<Outbox>,
    stop: Arc<AtomicBool>,
    path: PathBuf,
    accept_thread: Option<thread::JoinHandle<()>>,
    write_thread: Option<thread::JoinHandle<()>>,
}

impl MidiDriverLink {
    /// Bind the socket and start accepting. `tx` receives the MIDI that the
    /// driver sends to the guest, and each driver that attaches receives
    /// `gadget`'s identity.
    pub fn start(
        sock_dir: &Path,
        tx: Sender<OutFrame>,
        instance_id: u32,
        gadget: &GadgetIdentity,
    ) -> std::io::Result<Self> {
        let path = midi_driver_sock_path(sock_dir);
        let identity = Identity::new(instance_id, gadget);
        let listener = bind_clearing_stale(&path)?;

        let client: Client = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let outbox = Arc::new(Outbox::default());

        let write_client = client.clone();
        let write_outbox = outbox.clone();
        let write_stop = stop.clone();
        let write_thread = thread::Builder::new()
            .name("pc-link-midi-writer".into())
            .spawn(move || write_loop(&write_client, &write_outbox, &write_stop))
            .expect("the pc-link-midi-writer thread should start");

        let accept_client = client.clone();
        let accept_stop = stop.clone();
        let accept_thread = thread::Builder::new()
            .name("pc-link-midi-driver".into())
            .spawn(move || accept_loop(listener, identity, &accept_client, &tx, &accept_stop))
            .expect("the pc-link-midi-driver thread should start");

        Ok(Self {
            client,
            outbox,
            stop,
            path,
            accept_thread: Some(accept_thread),
            write_thread: Some(write_thread),
        })
    }

    /// Queue MIDI from the guest for the driver, which publishes it on the
    /// device's source endpoint. This returns immediately, because it runs on
    /// the transport reader thread, which also dispatches HID.
    pub fn publish_from_guest(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.outbox.push(bytes);
    }
}

/// The attached driver's write half, shared by the accept and writer threads.
type Client = Arc<Mutex<Option<UnixStream>>>;

/// Write queued guest MIDI to the attached driver until `stop` is set and the
/// queue is drained.
fn write_loop(client: &Client, outbox: &Outbox, stop: &AtomicBool) {
    while let Some(msg) = outbox.pop(stop) {
        let mut guard = client.lock().unwrap();
        if let Some(stream) = guard.as_mut() {
            if stream.write_all(&msg).is_err() {
                // A failed write leaves a partial message on the wire, so
                // the link is dropped. Shut the stream down as well, because
                // read_loop holds a clone of the fd and only a shutdown ends
                // its read().
                let _ = stream.shutdown(std::net::Shutdown::Both);
                *guard = None;
            }
        }
    }
}

/// Serve one driver at a time until `stop` is set: greet it with `identity`,
/// publish its write half in `client`, and forward what it sends to `tx`.
fn accept_loop(
    listener: UnixListener,
    identity: Identity,
    client: &Client,
    tx: &Sender<OutFrame>,
    stop: &AtomicBool,
) {
    let _ = listener.set_nonblocking(true);
    loop {
        if stop.load(Ordering::Acquire) {
            return;
        }
        let Some(stream) = accept_driver(&listener) else {
            continue;
        };
        let Some((stream, reader)) = greet(stream, &identity) else {
            continue;
        };
        {
            // Check `stop` under the lock that `drop` takes after setting
            // `stop`. Either `drop` finds this stream and shuts it down, or
            // this thread sees `stop` and never reads.
            let mut guard = client.lock().unwrap();
            if stop.load(Ordering::Acquire) {
                let _ = stream.shutdown(std::net::Shutdown::Both);
                return;
            }
            *guard = Some(stream);
        }
        eprintln!("pc-link: the MIDI driver attached.");

        // Serve one driver at a time. Read until it goes away, then wait for
        // it to reconnect.
        read_loop(reader, tx, stop);
        *client.lock().unwrap() = None;
        eprintln!("pc-link: the MIDI driver detached.");
    }
}

/// Return the next driver connection, or `None` after a pause when there is
/// none yet or the accept failed.
fn accept_driver(listener: &UnixListener) -> Option<UnixStream> {
    match listener.accept() {
        Ok((s, _)) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
            thread::sleep(ACCEPT_POLL);
            None
        }
        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => None,
        Err(_) => {
            thread::sleep(ACCEPT_ERROR_BACKOFF);
            None
        }
    }
}

/// Make a new driver connection blocking, with a write timeout, and send it
/// `identity`. Return the stream and a read half, or `None` when the driver
/// cannot be served.
fn greet(mut stream: UnixStream, identity: &Identity) -> Option<(UnixStream, UnixStream)> {
    // accept() may hand back the listener's non-blocking flag.
    let _ = stream.set_nonblocking(false);
    let reader = stream.try_clone().ok()?;
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    // Send the identity first; the raw MIDI stream follows.
    stream.write_all(identity.as_bytes()).ok()?;
    Some((stream, reader))
}

fn read_loop(mut reader: UnixStream, tx: &Sender<OutFrame>, stop: &AtomicBool) {
    let mut buf = [0u8; 1024];
    while !stop.load(Ordering::Acquire) {
        match reader.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                if tx.send((FRAME_MIDI, buf[..n].to_vec())).is_err() {
                    // The writer thread has exited, so PcLink is shutting down.
                    return;
                }
            }
            Err(_) => return,
        }
    }
}

impl Drop for MidiDriverLink {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Unblock read_loop if a driver is connected.
        if let Some(stream) = self.client.lock().unwrap().take() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        self.outbox.ready.notify_all();
        if let Some(h) = self.write_thread.take() {
            let _ = h.join();
        }
        if let Some(h) = self.accept_thread.take() {
            let _ = h.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}
