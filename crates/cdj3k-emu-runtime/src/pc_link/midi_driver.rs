//! Link between `PcLink` and the CoreMIDI driver plugin.
//!
//! The driver runs inside `MIDIServer`, so it cannot share the emulator's
//! virtio-serial socket. It connects here instead, over a unix socket carrying
//! a raw MIDI byte stream in both directions: the same bytes the guest reads
//! from and writes to the gadget's rawmidi device, with no extra framing.
//!
//! Why a driver at all: apps that walk endpoint → entity → device (djay Pro)
//! ignore endpoints made with `MIDISourceCreate`, because a virtual endpoint
//! has no entity, and `MIDIDeviceCreate` is refused outside a driver. See
//! `docs/pc-link.md`.
//!
//! The socket lives at `{instance_dir}/midi-driver.sock`, beside the instance's
//! other channels.  Binding it is how the instance advertises that PC Link is
//! on: the plugin scans `instance-*` and dials each one it finds, so every
//! instance gets its own connection and its own `MIDIDevice`, and no instance
//! can disturb another's.

#![cfg(target_os = "macos")]

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::collections::VecDeque;
use std::thread;
use std::time::Duration;

use crate::pc_link::frame::FRAME_MIDI;
use crate::pc_link::transport::OutFrame;

/// Cap on a write to the attached driver.  A driver that stops draining is
/// dropped rather than wedging the writer thread, which `drop` joins.
const WRITE_TIMEOUT: Duration = Duration::from_millis(250);

/// How often the accept thread re-checks the stop flag.  It also bounds how
/// long `drop` waits to join that thread.
const ACCEPT_POLL: Duration = Duration::from_millis(200);

/// Guest→driver messages held while the driver is slow.  Beyond this the
/// oldest are dropped; MIDI is only useful live.
const OUTBOX_CAP: usize = 1024;

/// Queue between the transport reader and the driver writer thread.  The
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

    /// Next message, or `None` once `stop` is set and the queue is drained.
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

/// Identity handed to the driver plugin when it connects.  It carries every
/// string and id the plugin puts on its `MIDIDevice`, so the plugin holds no
/// identity of its own and a rebrand needs no new plugin binary.  Fixed size:
/// the raw MIDI byte stream follows it with no framing.
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

/// The plugin parses this layout byte for byte; a size change here without a
/// matching one in tools/midi-driver is a silent protocol break.
const _: () = assert!(std::mem::size_of::<Identity>() == 112);
const IDENTITY_VERSION: u16 = 1;

impl Identity {
    fn new(instance_id: u32) -> Self {
        let mut product = [0u8; 32];
        let mut serial = [0u8; 32];
        let mut manufacturer = [0u8; 32];
        copy_field(&mut product, cdj3k_emu_platform::identity::PRODUCT);
        copy_field(
            &mut serial,
            &cdj3k_emu_platform::identity::device_serial(instance_id),
        );
        copy_field(&mut manufacturer, cdj3k_emu_platform::identity::MANUFACTURER);
        Self {
            magic: IDENTITY_MAGIC.to_le(),
            version: IDENTITY_VERSION.to_le(),
            instance: (instance_id as u16).to_le(),
            vid: cdj3k_emu_platform::identity::VENDOR_ID.to_le(),
            pid: cdj3k_emu_platform::identity::PRODUCT_ID.to_le(),
            location: (cdj3k_emu_platform::identity::usb_location_id(instance_id) as u32).to_le(),
            product,
            serial,
            manufacturer,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        // SAFETY: #[repr(C)] POD with no padding-sensitive reads; the driver
        // parses the same layout.
        unsafe {
            std::slice::from_raw_parts(self as *const Self as *const u8, std::mem::size_of::<Self>())
        }
    }
}

/// NUL-padded copy, truncated to fit.
fn copy_field(dst: &mut [u8; 32], src: &str) {
    let n = src.len().min(dst.len() - 1);
    dst[..n].copy_from_slice(&src.as_bytes()[..n]);
}

/// The socket the driver plugin dials for this instance.
pub fn midi_driver_sock_path(sock_dir: &Path) -> PathBuf {
    sock_dir.join("midi-driver.sock")
}

/// Bind `path`, clearing a socket left behind by a crash.  A node that still
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
            eprintln!("pc-link: cleared stale {}: {e}", path.display());
        }
    }
    UnixListener::bind(path)
}

const PLUGIN_NAME: &str = "CDJ3KEmuMIDI.plugin";
const PLUGIN_EXE: &str = "CDJ3KEmuMIDI";

/// Install or refresh the CoreMIDI driver plugin into the per-user MIDI
/// Drivers directory, where MIDIServer loads it from.  The plugin ships in the
/// app bundle's Resources; an unbundled dev build is a no-op (install by hand
/// with `make -C tools/midi-driver install`).  MIDIServer picks up a new or
/// changed plugin on its next client connection.
pub fn ensure_driver_installed() {
    let Some(src) = shipped_plugin() else { return };
    let Some(home) = std::env::var_os("HOME") else { return };
    let dest_dir = PathBuf::from(home).join("Library/Audio/MIDI Drivers");
    let dest = dest_dir.join(PLUGIN_NAME);

    // Byte-compare the executable: reinstall only when missing or changed.
    let src_exe = src.join("Contents/MacOS").join(PLUGIN_EXE);
    let dest_exe = dest.join("Contents/MacOS").join(PLUGIN_EXE);
    if same_bytes(&src_exe, &dest_exe) {
        return;
    }
    if let Some(v) = installed_plugin_version(&dest) {
        if v != IDENTITY_VERSION {
            eprintln!(
                "pc-link: installed MIDI driver is v{v}, this build speaks v{IDENTITY_VERSION}"
            );
        }
    }

    let _ = std::fs::create_dir_all(&dest_dir);
    // Staging is per process and outside the directory MIDIServer scans for
    // bundles; instances install concurrently.
    let staged = dest_dir
        .parent()
        .unwrap_or(&dest_dir)
        .join(format!("{PLUGIN_NAME}.{}.staging", std::process::id()));
    let _ = std::fs::remove_dir_all(&staged);
    // ditto preserves the bundle's code signature; a plain recursive copy does not.
    match std::process::Command::new("/usr/bin/ditto").arg(&src).arg(&staged).status() {
        Ok(s) if s.success() => {
            let _ = std::fs::remove_dir_all(&dest);
            if let Err(e) = std::fs::rename(&staged, &dest) {
                let _ = std::fs::remove_dir_all(&staged);
                eprintln!("pc-link: MIDI driver install failed: {e}");
                return;
            }
            eprintln!("pc-link: installed MIDI driver -> {}", dest.display());
            // A running MIDIServer has the previous binary mapped and keeps
            // serving it; only its exit loads the new one.  Ending it drops
            // every MIDI app's connection and they do not reconnect, so the
            // menu offers the choice instead of taking it.
            if midi_server_running() {
                cdj3k_emu_platform::menu_state::lock().midi_driver_replaced = true;
            }
        }
        Ok(s) => {
            let _ = std::fs::remove_dir_all(&staged);
            eprintln!("pc-link: MIDI driver install failed (ditto {s})");
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staged);
            eprintln!("pc-link: MIDI driver install failed: {e}");
        }
    }
}

/// The plugin shipped in the running app bundle
/// (`<app>.app/Contents/Resources/CDJ3KEmuMIDI.plugin`), or `None` when running
/// unbundled or the executable path can't be resolved.
fn shipped_plugin() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // <app>/Contents/MacOS/<exe> -> <app>/Contents/Resources/<plugin>
    let plugin = exe.parent()?.parent()?.join("Resources").join(PLUGIN_NAME);
    plugin.is_dir().then_some(plugin)
}

/// The wire format an installed plugin declares, from `CDJ3KEmuMIDIVersion`
/// in its `Info.plist`.  `None` when absent or unparsable.
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

fn same_bytes(a: &Path, b: &Path) -> bool {
    matches!((std::fs::read(a), std::fs::read(b)), (Ok(x), Ok(y)) if x == y)
}

/// Accepts the driver plugin and pumps MIDI both ways.
pub struct MidiDriverLink {
    /// Write half of the connected driver, if one has attached.
    client: Arc<Mutex<Option<UnixStream>>>,
    outbox: Arc<Outbox>,
    stop: Arc<AtomicBool>,
    path: PathBuf,
    accept_thread: Option<thread::JoinHandle<()>>,
    write_thread: Option<thread::JoinHandle<()>>,
}

impl MidiDriverLink {
    /// Bind the socket and start accepting. `tx` receives driver→guest MIDI.
    pub fn start(sock_dir: &Path, tx: Sender<OutFrame>, instance_id: u32) -> std::io::Result<Self> {
        let path = midi_driver_sock_path(sock_dir);
        let listener = bind_clearing_stale(&path)?;

        let client: Arc<Mutex<Option<UnixStream>>> = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let outbox = Arc::new(Outbox::default());

        let write_client = client.clone();
        let write_outbox = outbox.clone();
        let write_stop = stop.clone();
        let write_thread = thread::Builder::new()
            .name("pc-link-midi-writer".into())
            .spawn(move || {
                while let Some(msg) = write_outbox.pop(&write_stop) {
                    let mut guard = write_client.lock().unwrap();
                    if let Some(stream) = guard.as_mut() {
                        if stream.write_all(&msg).is_err() {
                            // A failed write leaves a partial message on the
                            // wire, so the link goes rather than the parser
                            // being fed the next one.  Shutdown, not just a
                            // drop: read_loop holds a clone of the fd, and
                            // only a shutdown ends its read().
                            let _ = stream.shutdown(std::net::Shutdown::Both);
                            *guard = None;
                        }
                    }
                }
            })
            .expect("spawn pc-link-midi-writer");

        let accept_client = client.clone();
        let accept_stop = stop.clone();
        let accept_thread = thread::Builder::new()
            .name("pc-link-midi-driver".into())
            .spawn(move || {
                let _ = listener.set_nonblocking(true);
                loop {
                    if accept_stop.load(Ordering::Acquire) {
                        return;
                    }
                    let mut stream = match listener.accept() {
                        Ok((s, _)) => s,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(ACCEPT_POLL);
                            continue;
                        }
                        Err(_) => continue,
                    };
                    // accept() may hand back the listener's non-blocking flag.
                    let _ = stream.set_nonblocking(false);
                    let reader = match stream.try_clone() {
                        Ok(r) => r,
                        Err(_) => continue,
                    };
                    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
                    // Identity first, then the raw MIDI stream.
                    if stream.write_all(Identity::new(instance_id).as_bytes()).is_err() {
                        continue;
                    }
                    {
                        // Checked under the lock `drop` takes after setting
                        // `stop`: either `drop` finds this stream and shuts it
                        // down, or this sees `stop` and never reads.
                        let mut guard = accept_client.lock().unwrap();
                        if accept_stop.load(Ordering::Acquire) {
                            let _ = stream.shutdown(std::net::Shutdown::Both);
                            return;
                        }
                        *guard = Some(stream);
                    }
                    eprintln!("pc-link: MIDI driver attached");

                    // One driver at a time: read until it goes away, then
                    // loop back and wait for it to reconnect.
                    read_loop(reader, &tx, &accept_stop);
                    *accept_client.lock().unwrap() = None;
                    eprintln!("pc-link: MIDI driver detached");
                }
            })
            .expect("spawn pc-link-midi-driver");

        Ok(Self {
            client,
            outbox,
            stop,
            path,
            accept_thread: Some(accept_thread),
            write_thread: Some(write_thread),
        })
    }

    /// Queue guest-originated MIDI for the driver, which publishes it on the
    /// device's source endpoint.  Returns immediately: this runs on the
    /// transport reader thread, which also dispatches HID.
    pub fn publish_from_guest(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.outbox.push(bytes);
    }
}

fn read_loop(mut reader: UnixStream, tx: &Sender<OutFrame>, stop: &AtomicBool) {
    let mut buf = [0u8; 1024];
    while !stop.load(Ordering::Acquire) {
        match reader.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                if tx.send((FRAME_MIDI, buf[..n].to_vec())).is_err() {
                    return; // writer thread gone; PcLink is shutting down
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
