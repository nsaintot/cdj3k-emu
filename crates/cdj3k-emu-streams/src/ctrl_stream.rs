//! CtrlStream - bidirectional connection to the `ctrl.sock` virtio-serial port.
//!
//! Connects to `{socket_dir}/ctrl.sock` (created by QEMU as a Unix socket server).
//! A background thread owns the UnixStream and reads 64-byte MOSI LED frames,
//! storing the latest in `state`.  The write half is cloned and shared so
//! `inject()` can send MISO frames from any thread.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use cdj3k_emu_subucom::miso_frame::MISO_SIZE;
use cdj3k_emu_subucom::mosi_frame::MosiFrame;

/// Backoff before retrying `connect()` after `ctrl.sock` is gone or refuses.
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

impl LedState {
    /// Wraps the raw frame bytes in a `MosiFrame` accessor.
    pub fn mosi(&self) -> MosiFrame {
        MosiFrame::from_bytes(self.frame)
    }
}

pub struct CtrlStream {
    state: Arc<Mutex<Option<LedState>>>,
    writer: Arc<Mutex<Option<UnixStream>>>,
    sock_path: PathBuf,
    /// Always reflects the most recently-received MOSI frame, regardless of
    /// the repaint-gating that controls `state`. Lets observers (e.g. the
    /// deferred debug viewport) display live wire bytes without being coupled
    /// to the main viewport's update cycle.
    latest_mosi: Arc<Mutex<[u8; 64]>>,
}

impl CtrlStream {
    /// Spawn the reader/writer thread and return a handle.
    /// `socket_dir` is e.g. `/tmp/cdj3k-0`; the socket is `{socket_dir}/ctrl.sock`.
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
            .expect("spawn ctrl-stream thread");

        Self {
            state,
            writer,
            sock_path,
            latest_mosi,
        }
    }

    /// Snapshot the most recently received MOSI frame. Always returns the
    /// latest wire bytes - does not interact with the take()/repaint flow.
    pub fn peek_latest_mosi(&self) -> [u8; 64] {
        *self.latest_mosi.lock().unwrap()
    }

    /// Shared handle to the live MOSI buffer for callers (e.g. deferred
    /// viewports) that need to read it from a context that can't borrow `&self`.
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

    /// Send one MISO frame to the guest.  Returns false if not connected.
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
                eprintln!("[ctrl] inject: write failed: {e}");
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
    // Rate-limit connect-failure logging: the socket is absent until QEMU is
    // spawned and after a guest restart.  Log the first failure, then every
    // 10th attempt thereafter, so stderr doesn't grow at RECONNECT_DELAY⁻¹
    // for the entire pre-spawn or post-crash window.
    let mut failed_attempts: u32 = 0;
    loop {
        match UnixStream::connect(&sock_path) {
            Ok(stream) => {
                eprintln!("[ctrl] connected to {}", sock_path.display());
                failed_attempts = 0;
                if let Ok(write_half) = stream.try_clone() {
                    *writer.lock().unwrap() = Some(write_half);
                    read_loop(stream, &state, &latest_mosi, &gate);
                    *writer.lock().unwrap() = None;
                }
                eprintln!("[ctrl] disconnected - reconnecting");
            }
            Err(e) => {
                failed_attempts = failed_attempts.saturating_add(1);
                if failed_attempts == 1 || failed_attempts.is_multiple_of(10) {
                    eprintln!(
                        "[ctrl] connect {}: {} (attempt {failed_attempts}, retrying in {:?})",
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
    // Last MOSI frame for which we requested a repaint. The wire heartbeats
    // ~100 Hz with mostly-identical bytes when no LED state changed; gating
    // here keeps the main viewport asleep instead of waking it 100×/s for
    // visually-identical frames. `latest_mosi` is updated unconditionally so
    // the debug viewport's live peek still sees every frame.
    let mut last_repaint_mosi: Option<[u8; 64]> = None;
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
                    eprintln!("[ctrl] read error: {}", e);
                    break 'frame false;
                }
            }
        };
        if !ok {
            eprintln!("[ctrl] stream ended after {} frames", frames);
            break;
        }
        puffin::profile_scope!("ctrl_rx_frame");
        frames += 1;
        // Live peek - always.
        *latest_mosi.lock().unwrap() = buf;
        // Repaint trigger - only when bytes actually changed.
        let changed = last_repaint_mosi.map_or(true, |prev| prev != buf);
        if changed {
            *state.lock().unwrap() = Some(LedState { frame: buf });
            last_repaint_mosi = Some(buf);
            gate.request();
        }
    }
}
