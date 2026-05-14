//! CfgClient - bidirectional bridge to the guest's `cdj3k-cfgd` daemon.
//!
//! One Unix socket (`{sock_dir}/cfg.sock`) carries USB attach/detach commands,
//! `set`/`get` for the whitelisted virtio_snd sysfs parameters, and an
//! unsolicited 3-second latency push.
//!
//! Wire protocol (line-based, ASCII, '\n'-terminated):
//!
//!   host → guest
//!     usb attach              - invoke /usr/sbin/usb-external-attach.sh
//!     usb detach              - no-op (EP122 handles unmount)
//!     set <name> <value>      - write to a whitelisted sysfs param
//!     get <name>              - request a `param` response
//!
//!   guest → host
//!     usb_state <0|1>         - emitted by the in-guest USB hooks
//!     param <name> <value>    - response to `get` or unsolicited push
//!     latency <g>,<h>,<t>     - pushed every 3s by cdj3k-cfgd
//!
//! Connection is lazy and self-healing: the reader thread reconnects on EOF
//! / connect failure, the writer methods retry briefly while QEMU is still
//! bringing the port up.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Backoff between reader-thread reconnect attempts. Long enough not to thrash
/// QEMU while it's spinning up its virtio-serial port, short enough that the
/// "first frame" UX (latency label, USB state) lights up promptly.
const RECONNECT_DELAY: Duration = Duration::from_millis(500);
/// Cold-path writer: max attempts to open a one-shot connection while the
/// reader thread is still bringing up the long-lived writer. `RETRY_DELAY ×
/// RETRIES = 2 s`, matching the read-side timeout below.
const COLD_WRITER_RETRIES: u32 = 20;
const COLD_WRITER_RETRY_DELAY: Duration = Duration::from_millis(100);
/// Write timeout for the cold-path one-shot connection.
const COLD_WRITER_WRITE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug)]
pub struct Latency {
    pub guest_ms: u32,
    pub host_ms: u32,
    pub total_ms: u32,
    pub at: Instant,
}

#[derive(Default)]
struct Shared {
    /// Latest USB mount state from the guest (`true` = mounted).
    usb_state: Option<bool>,
    /// Latest latency push.
    latency: Option<Latency>,
    /// Most recent `param <name> <value>` response per name.
    params: HashMap<String, String>,
    /// Writer half - `None` while disconnected.
    writer: Option<UnixStream>,
}

#[derive(Clone)]
pub struct CfgClient {
    sock_path: PathBuf,
    shared: Arc<Mutex<Shared>>,
}

impl CfgClient {
    /// Spawn the background reader thread and return a handle.
    /// `sock_dir` is e.g. `/tmp/cdj3k-emu/instance-1`; the socket is `{sock_dir}/cfg.sock`.
    pub fn new(sock_dir: &std::path::Path) -> Self {
        let sock_path = sock_dir.join("cfg.sock");
        let shared = Arc::new(Mutex::new(Shared::default()));

        let path = sock_path.clone();
        let shared_clone = Arc::clone(&shared);
        thread::Builder::new()
            .name("cfg-stream".into())
            .spawn(move || reader_loop(path, shared_clone))
            .expect("spawn cfg-stream thread");

        Self { sock_path, shared }
    }

    /// Most recently received guest USB mount state (`Some(true/false)`),
    /// or `None` if no state has been observed yet.
    pub fn usb_state(&self) -> Option<bool> {
        self.shared.lock().ok()?.usb_state
    }

    /// Take the next pending USB-state transition (consumes the cached value
    /// so callers can use this in a poll loop just like the old watcher).
    pub fn poll_usb_state(&self) -> Option<bool> {
        let mut s = self.shared.lock().ok()?;
        s.usb_state.take()
    }

    /// Latest latency triple. `None` until the first push lands.
    pub fn latency(&self) -> Option<Latency> {
        self.shared.lock().ok()?.latency
    }

    /// Most recent `param <name> <value>` response, if any.
    pub fn param(&self, name: &str) -> Option<String> {
        self.shared.lock().ok()?.params.get(name).cloned()
    }

    /// Send `usb attach\n`. Retries briefly while the port is still coming up.
    pub fn usb_attach(&self) -> std::io::Result<()> {
        self.send_line("usb attach\n")
    }

    /// Send `usb detach\n`. No-op on the guest side (EP122 handles unmount).
    pub fn usb_detach(&self) -> std::io::Result<()> {
        self.send_line("usb detach\n")
    }

    /// Write a sysfs param. The guest will respond with a `param` line that
    /// updates [`Self::param`] asynchronously.
    pub fn set_param(&self, name: &str, value: &str) -> std::io::Result<()> {
        if name.contains(' ') || value.contains('\n') {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "param name/value must not contain spaces / newlines",
            ));
        }
        self.send_line(&format!("set {} {}\n", name, value))
    }

    /// Request a sysfs param. The response arrives asynchronously and updates
    /// [`Self::param`].
    pub fn get_param(&self, name: &str) -> std::io::Result<()> {
        if name.contains(' ') {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "param name must not contain spaces",
            ));
        }
        self.send_line(&format!("get {}\n", name))
    }

    fn send_line(&self, line: &str) -> std::io::Result<()> {
        // Fast path: writer already connected by the reader thread.
        if let Ok(mut s) = self.shared.lock() {
            if let Some(ref mut w) = s.writer {
                if w.write_all(line.as_bytes()).is_ok() && w.flush().is_ok() {
                    return Ok(());
                }
                // Write failed - drop and let the reader thread reconnect.
                s.writer = None;
            }
        }
        // Cold path: open a one-shot writer connection. We don't hold this
        // open because the reader thread owns the long-lived stream.
        let mut last_err: Option<std::io::Error> = None;
        for _ in 0..COLD_WRITER_RETRIES {
            match UnixStream::connect(&self.sock_path) {
                Ok(mut s) => {
                    s.set_write_timeout(Some(COLD_WRITER_WRITE_TIMEOUT))?;
                    s.write_all(line.as_bytes())?;
                    s.flush()?;
                    return Ok(());
                }
                Err(e) => {
                    last_err = Some(e);
                    thread::sleep(COLD_WRITER_RETRY_DELAY);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| std::io::Error::other("cfg socket unavailable")))
    }
}

fn reader_loop(sock_path: PathBuf, shared: Arc<Mutex<Shared>>) {
    loop {
        let stream = match UnixStream::connect(&sock_path) {
            Ok(s) => s,
            Err(_) => {
                thread::sleep(RECONNECT_DELAY);
                continue;
            }
        };

        // Stash a clone for writers.
        if let Ok(write_clone) = stream.try_clone() {
            if let Ok(mut s) = shared.lock() {
                s.writer = Some(write_clone);
            }
        }

        let reader = BufReader::new(stream);
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            handle_line(&line, &shared);
        }

        // Disconnected - drop writer and try again.
        if let Ok(mut s) = shared.lock() {
            s.writer = None;
        }
        thread::sleep(RECONNECT_DELAY);
    }
}

fn handle_line(line: &str, shared: &Arc<Mutex<Shared>>) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }

    if let Some(rest) = line.strip_prefix("usb_state ") {
        let mounted = rest.trim() == "1";
        if let Ok(mut s) = shared.lock() {
            s.usb_state = Some(mounted);
        }
        return;
    }

    if let Some(rest) = line.strip_prefix("latency ") {
        let mut parts = rest.split(',');
        let g = parts.next().and_then(|x| x.trim().parse().ok());
        let h = parts.next().and_then(|x| x.trim().parse().ok());
        let t = parts.next().and_then(|x| x.trim().parse().ok());
        if let (Some(g), Some(h), Some(t)) = (g, h, t) {
            if let Ok(mut s) = shared.lock() {
                s.latency = Some(Latency {
                    guest_ms: g,
                    host_ms: h,
                    total_ms: t,
                    at: Instant::now(),
                });
            }
        }
        return;
    }

    if let Some(rest) = line.strip_prefix("param ") {
        if let Some(space) = rest.find(' ') {
            let (name, value) = rest.split_at(space);
            let value = value.trim_start();
            if let Ok(mut s) = shared.lock() {
                s.params.insert(name.to_string(), value.to_string());
            }
        }
        return;
    }
}
