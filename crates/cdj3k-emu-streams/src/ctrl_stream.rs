//! CtrlStream, the two-way connection to the `ctrl.sock` virtio-serial port.
//!
//! It connects to `{socket_dir}/ctrl.sock`, a Unix socket server that QEMU
//! creates. A background thread owns the UnixStream, reads 64-byte MOSI LED
//! frames and stores the latest in `state`. The write half is cloned and
//! shared, so `inject()` can send MISO frames from any thread.

use cdj3k_emu_platform::local_socket::LocalStream as UnixStream;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use cdj3k_emu_panel::miso_frame::MISO_SIZE;

/// The wait before retrying `connect()` when `ctrl.sock` is missing or refuses
/// the connection.
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub struct LedState {
    pub frame: [u8; 64],
}

impl Default for LedState {
    fn default() -> Self {
        Self { frame: [0u8; 64] }
    }
}

pub struct CtrlStream {
    state: Arc<Mutex<Option<LedState>>>,
    writer: Arc<Mutex<Option<UnixStream>>>,
    sock_path: PathBuf,
    /// The most recently received MOSI frame, updated whether or not the
    /// repaint gate updates `state`. Observers such as the deferred debug
    /// viewport read live wire bytes from it without depending on the main
    /// viewport's update cycle.
    latest_mosi: Arc<Mutex<[u8; 64]>>,
}

impl CtrlStream {
    /// Spawn the reader and writer thread and return a handle.
    /// `socket_dir` is the slot's runtime directory, such as
    /// `/tmp/cdj3k-emu-501/instance-1`, and the socket is
    /// `{socket_dir}/ctrl.sock`.
    pub fn new(socket_dir: &str, gate: crate::RepaintGate) -> Self {
        let sock_path = PathBuf::from(socket_dir.trim_end_matches('/')).join("ctrl.sock");
        let state: Arc<Mutex<Option<LedState>>> = Arc::new(Mutex::new(None));
        let writer: Arc<Mutex<Option<UnixStream>>> = Arc::new(Mutex::new(None));
        let latest_mosi: Arc<Mutex<[u8; 64]>> = Arc::new(Mutex::new([0u8; 64]));

        let state_clone = Arc::clone(&state);
        let writer_clone = Arc::clone(&writer);
        let latest_clone = Arc::clone(&latest_mosi);
        let path = sock_path.clone();

        thread::Builder::new()
            .name("ctrl-stream".into())
            .spawn(move || stream_loop(path, state_clone, writer_clone, latest_clone, gate))
            .expect("the ctrl-stream thread could not be started");

        Self {
            state,
            writer,
            sock_path,
            latest_mosi,
        }
    }

    /// Returns a copy of the most recently received MOSI frame. It does not
    /// affect the `take()` and repaint flow.
    pub fn peek_latest_mosi(&self) -> [u8; 64] {
        *self.latest_mosi.lock().unwrap()
    }

    /// A shared handle to the live MOSI buffer, for callers such as deferred
    /// viewports that cannot borrow `&self`.
    pub fn latest_mosi_arc(&self) -> Arc<Mutex<[u8; 64]>> {
        Arc::clone(&self.latest_mosi)
    }

    pub fn addr_str(&self) -> &str {
        self.sock_path.to_str().unwrap_or("")
    }

    pub fn is_ready(&self) -> bool {
        self.writer.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    /// Returns the latest LED frame if one has arrived since the last call.
    pub fn take(&self) -> Option<LedState> {
        self.state.lock().unwrap().take()
    }

    /// Send one MISO frame to the guest. Returns false if the socket is not
    /// connected.
    pub fn inject(&self, frame: &[u8; MISO_SIZE]) -> bool {
        puffin::profile_function!();
        let Ok(mut guard) = self.writer.lock() else {
            return false;
        };
        let Some(w) = guard.as_mut() else {
            return false;
        };
        puffin::profile_scope!("write_all_miso");
        match w.write_all(frame) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("[ctrl] Writing a MISO frame failed: {e}");
                *guard = None;
                false
            }
        }
    }
}

fn stream_loop(
    sock_path: PathBuf,
    state: Arc<Mutex<Option<LedState>>>,
    writer: Arc<Mutex<Option<UnixStream>>>,
    latest_mosi: Arc<Mutex<[u8; 64]>>,
    gate: crate::RepaintGate,
) {
    // Limit the connect-failure log. The socket is absent until QEMU starts
    // and after a guest restart, so log the first failure and then every 10th
    // attempt; otherwise stderr gets a line every RECONNECT_DELAY for as long
    // as the socket is absent.
    let mut failed_attempts: u32 = 0;
    loop {
        match UnixStream::connect(&sock_path) {
            Ok(stream) => {
                eprintln!("[ctrl] Connected to {}.", sock_path.display());
                failed_attempts = 0;
                if let Ok(write_half) = stream.try_clone() {
                    *writer.lock().unwrap() = Some(write_half);
                    read_loop(stream, &state, &latest_mosi, &gate);
                    *writer.lock().unwrap() = None;
                }
                eprintln!("[ctrl] The control socket closed; reconnecting.");
            }
            Err(e) => {
                failed_attempts = failed_attempts.saturating_add(1);
                if failed_attempts == 1 || failed_attempts.is_multiple_of(10) {
                    eprintln!(
                        "[ctrl] Connecting to {} failed: {} (attempt {failed_attempts}); retrying in {:?}.",
                        sock_path.display(),
                        e,
                        RECONNECT_DELAY
                    );
                }
                thread::sleep(RECONNECT_DELAY);
            }
        }
    }
}

fn read_loop(
    mut stream: UnixStream,
    state: &Arc<Mutex<Option<LedState>>>,
    latest_mosi: &Arc<Mutex<[u8; 64]>>,
    gate: &crate::RepaintGate,
) {
    let mut buf = [0u8; 64];
    let mut frames: u64 = 0;
    // The last MOSI frame that requested a repaint. The guest sends about 100
    // frames a second, and they are identical while no LED changes; comparing
    // them here keeps the main viewport asleep through those frames.
    // `latest_mosi` is updated for every frame, so the debug viewport's live
    // peek still sees all of them.
    let mut last_repaint_mosi: Option<[u8; 64]> = None;
    // With `CDJ3K_MOSI_DUMP=<file>`, append every changed frame as one hex line
    // (`<unix ms> <frame number> <64 bytes>`), to map a model's LED layout
    // against scripted presses offline.
    let mut dump = std::env::var_os("CDJ3K_MOSI_DUMP").and_then(|p| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .ok()
    });
    loop {
        let mut total = 0;
        let ok = 'frame: loop {
            match stream.read(&mut buf[total..]) {
                Ok(0) => break 'frame false,
                Ok(n) => {
                    total += n;
                    if total == 64 {
                        break 'frame true;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    eprintln!("[ctrl] Reading from the control socket failed: {}", e);
                    break 'frame false;
                }
            }
        };
        if !ok {
            eprintln!("[ctrl] The stream ended after {} frames.", frames);
            break;
        }
        puffin::profile_scope!("ctrl_rx_frame");
        frames += 1;
        // Update the live peek for every frame.
        *latest_mosi.lock().unwrap() = buf;
        // Request a repaint only when the bytes changed.
        let changed = last_repaint_mosi != Some(buf);
        if changed {
            *state.lock().unwrap() = Some(LedState { frame: buf });
            last_repaint_mosi = Some(buf);
            gate.request();
            if let Some(f) = dump.as_mut() {
                use std::io::Write as _;
                let hex: String = buf.iter().map(|b| format!("{b:02x}")).collect();
                let ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0);
                let _ = writeln!(f, "{ms} {frames} {hex}");
            }
        }
    }
}
