//! `CfgClient` is a two-way bridge to the guest's `cdj3k-cfgd` daemon.
//!
//! One Unix socket (`{sock_dir}/cfg.sock`) carries USB attach/detach commands,
//! `set`/`get` for the whitelisted virtio_snd sysfs parameters, an
//! unsolicited 3-second latency push, and the boot's mods report.
//!
//! The wire protocol uses ASCII lines that end in `\n`:
//!
//!   Host to guest
//!     usb attach              - run /usr/sbin/usb-external-attach.sh
//!     usb detach              - notify EP122 through /proc/udev_usb1 and
//!                               lazily unmount the drive
//!     set <name> <value>      - write to a whitelisted sysfs param
//!     get <name>              - request a `param` response
//!     pc_link on|off          - start or stop the PC Link bridge service
//!     mods status             - request the mods report
//!                               (sent on every connection)
//!     mods log <name>         - request a mod's journal for this boot
//!     ping                    - request a `pong`
//!
//!   Guest to host
//!     usb_state <0|1>         - emitted by the in-guest USB hooks
//!     param <name> <value>    - response to `get` or unsolicited push
//!     latency <g>,<h>,<t>     - pushed every 3s by cdj3k-cfgd
//!     pc_link on|off          - confirms a `pc_link` command
//!     pc_link on_failed|off_failed - systemctl returned non-zero
//!     pong                    - the answer to `ping`
//!     mods off|pending        - the boot has no mods / the runner has not finished
//!     mods begin, mod <name> …, mods end
//!                             - the runner's report ([`crate::mods_report::ModOutcome`])
//!     log_begin <name>, log <name> <line>, log_end <name>
//!                             - a mod's journal, in answer to each `mods log`
//!
//! The connection is lazy and self-healing. The reader thread reconnects
//! after an EOF or a failed connect, and the writer methods queue commands
//! until cfgd answers.

use cdj3k_emu_platform::local_socket::LocalStream as UnixStream;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::mods_report::{ModsReport, Partial};

mod messages;

use messages::handle_line;

/// The delay between reconnect attempts of the reader thread. It is long
/// enough not to flood QEMU while it brings up its virtio-serial port, and
/// short enough that the latency label and the USB state appear promptly.
const RECONNECT_DELAY: Duration = Duration::from_millis(500);
const PING_PERIOD: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug)]
pub struct Latency {
    pub guest_ms: u32,
    pub host_ms: u32,
    pub total_ms: u32,
    pub at: Instant,
}

#[derive(Default)]
struct Shared {
    /// The latest USB mount state from the guest; `true` means mounted.
    usb_state: Option<bool>,
    /// The latest latency push.
    latency: Option<Latency>,
    /// The latest `param <name> <value>` response for each name.
    params: HashMap<String, String>,
    /// The last PC Link bridge state that the guest confirmed: `Some(true)`
    /// when the bridge runs, `Some(false)` when it is stopped, and `None`
    /// until the first `pc_link` response.
    pc_link_state: Option<bool>,
    /// `pc_link` replies that reported a failure rather than a state.  An
    /// unanswered command means the guest has not opened the port yet; a
    /// failure reply means cfgd ran systemctl and it did not work.
    pc_link_failures: u32,
    /// The writer half of the stream, or `None` while disconnected.
    writer: Option<UnixStream>,
    /// Set once the guest has sent a line, which proves cfgd has the port
    /// open. QEMU accepts the host connection whether or not the guest is
    /// reading, so a command written before that is discarded with no error.
    guest_ready: bool,
    /// Commands held until `guest_ready`.
    pending: Vec<String>,
    /// The mods report for this boot; cfgd sends it again on every connection.
    mods: ModsReport,
    /// The `mod` lines received after `mods begin`, until `mods end`.
    mods_partial: Option<Partial>,
    /// Requested journals: how many answers are still due, and the lines of
    /// the answer being received.
    mod_logs_partial: HashMap<String, (u32, Vec<String>)>,
    /// Complete journals: the answer to the latest request for each mod.
    mod_logs: HashMap<String, Vec<String>>,
}

#[derive(Clone)]
pub struct CfgClient {
    shared: Arc<Mutex<Shared>>,
}

impl CfgClient {
    /// Start the reader and ping threads and return a handle.
    /// `sock_dir` is the instance directory; the socket is `{sock_dir}/cfg.sock`.
    pub fn new(sock_dir: &std::path::Path) -> Self {
        let sock_path = sock_dir.join("cfg.sock");
        let shared = Arc::new(Mutex::new(Shared::default()));

        let path = sock_path.clone();
        let weak = Arc::downgrade(&shared);
        thread::Builder::new()
            .name("cfg-stream".into())
            .spawn(move || reader_loop(path, weak))
            .expect("the cfg-stream thread should start");

        let weak = Arc::downgrade(&shared);
        thread::Builder::new()
            .name("cfg-ping".into())
            .spawn(move || ping_loop(weak))
            .expect("the cfg-ping thread should start");

        Self { shared }
    }

    /// Most recently received guest USB mount state (`Some(true/false)`),
    /// or `None` if no state has been observed yet.
    pub fn usb_state(&self) -> Option<bool> {
        self.shared.lock().ok()?.usb_state
    }

    /// Take the latest USB state. This consumes the cached value, so callers
    /// can use it in a poll loop.
    pub fn poll_usb_state(&self) -> Option<bool> {
        let mut s = self.shared.lock().ok()?;
        s.usb_state.take()
    }

    /// The latest latency triple, or `None` until the first push arrives.
    pub fn latency(&self) -> Option<Latency> {
        self.shared.lock().ok()?.latency
    }

    /// Most recent `param <name> <value>` response, if any.
    pub fn param(&self, name: &str) -> Option<String> {
        self.shared.lock().ok()?.params.get(name).cloned()
    }

    /// The latest confirmed PC Link bridge state, or `None` until a
    /// `pc_link` response arrives.
    pub fn pc_link_state(&self) -> Option<bool> {
        self.shared.lock().ok()?.pc_link_state
    }

    /// Forget the last `pc_link` reply, so that the reconcile loop sends the
    /// command again when the state changes.
    pub fn reset_pc_link_state(&self) {
        if let Ok(mut s) = self.shared.lock() {
            s.pc_link_state = None;
            s.pc_link_failures = 0;
        }
    }

    /// The number of `pc_link *_failed` and `?` replies since the last reset.
    pub fn pc_link_failures(&self) -> u32 {
        self.shared.lock().map(|s| s.pc_link_failures).unwrap_or(0)
    }

    /// `true` once the reader thread has connected to the cfg Unix socket and
    /// holds a writer half. This proves only that the QEMU chardev accepted
    /// the connection; [`Self::guest_heard`] tells whether cfgd is reading.
    pub fn is_connected(&self) -> bool {
        self.shared
            .lock()
            .map(|s| s.writer.is_some())
            .unwrap_or(false)
    }

    /// `true` once cfgd has said anything on the current connection, so a
    /// `usb attach` would reach the guest rather than QEMU's buffer.
    pub fn guest_heard(&self) -> bool {
        self.shared.lock().map(|s| s.guest_ready).unwrap_or(false)
    }

    /// Forget the mods report and journals of the boot that ended, so the
    /// next boot's report counts as new even when it reads the same.
    pub fn new_boot(&self) {
        if let Ok(mut s) = self.shared.lock() {
            s.mods = ModsReport::Unknown;
            s.mods_partial = None;
            s.mod_logs_partial.clear();
            s.mod_logs.clear();
        }
    }

    /// The guest's mods report for this boot.
    pub fn mods_report(&self) -> ModsReport {
        self.shared
            .lock()
            .map(|s| s.mods.clone())
            .unwrap_or_default()
    }

    /// Ask cfgd for `name`'s journal; [`Self::mod_log`] returns it once it has
    /// arrived.
    pub fn request_mod_log(&self, name: &str) -> std::io::Result<()> {
        if name.is_empty() || name.contains(char::is_whitespace) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "a mod name cannot contain spaces",
            ));
        }
        if let Ok(mut s) = self.shared.lock() {
            s.mod_logs.remove(name);
            s.mod_logs_partial.entry(name.to_string()).or_default().0 += 1;
        }
        self.send_line(&format!("mods log {name}\n"))
    }

    /// `name`'s journal in answer to the latest [`Self::request_mod_log`],
    /// once it is complete.
    pub fn mod_log(&self, name: &str) -> Option<Vec<String>> {
        self.shared.lock().ok()?.mod_logs.get(name).cloned()
    }

    /// Send `usb attach\n`. The command waits in the queue until cfgd answers.
    pub fn usb_attach(&self) -> std::io::Result<()> {
        self.send_line("usb attach\n")
    }

    /// Send `usb detach\n`. cfgd tells EP122 through `/proc/udev_usb1` to
    /// close its files, then lazily unmounts the drive.
    pub fn usb_detach(&self) -> std::io::Result<()> {
        self.send_line("usb detach\n")
    }

    /// Turn the PC Link bridge in the guest on or off. When `on` is true, this
    /// starts `cdj3k-pc-link-bridge.service`, which forwards the gadget's HID
    /// and MIDI endpoints over the `cdj3k.usb-link` virtio-serial port. When
    /// `on` is false, it stops the service, and the host sees a socket EOF.
    pub fn pc_link(&self, on: bool) -> std::io::Result<()> {
        self.send_line(if on { "pc_link on\n" } else { "pc_link off\n" })
    }

    /// Write a sysfs param. The guest will respond with a `param` line that
    /// updates [`Self::param`] asynchronously.
    pub fn set_param(&self, name: &str, value: &str) -> std::io::Result<()> {
        if name.contains(' ') || value.contains('\n') {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "a param name cannot contain spaces, and a param value cannot contain newlines",
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
                "a param name cannot contain spaces",
            ));
        }
        self.send_line(&format!("get {}\n", name))
    }

    fn send_line(&self, line: &str) -> std::io::Result<()> {
        let Ok(mut s) = self.shared.lock() else {
            return Err(std::io::Error::other("the cfg client state is poisoned"));
        };
        if s.guest_ready {
            if let Some(w) = s.writer.as_mut() {
                if w.write_all(line.as_bytes()).is_ok() && w.flush().is_ok() {
                    return Ok(());
                }
                s.writer = None;
                s.guest_ready = false;
            }
        }
        s.pending.push(line.to_string());
        Ok(())
    }
}

/// Connect to the socket, read cfgd's lines, and reconnect after QEMU exits.
/// The loop ends once the client is dropped, because QEMU serves one
/// connection on the socket.
fn reader_loop(sock_path: PathBuf, shared: std::sync::Weak<Mutex<Shared>>) {
    loop {
        if shared.strong_count() == 0 {
            return;
        }
        let stream = match UnixStream::connect(&sock_path) {
            Ok(s) => s,
            Err(_) => {
                thread::sleep(RECONNECT_DELAY);
                continue;
            }
        };

        // Keep a clone of the stream for the writer methods.
        {
            let Some(shared) = shared.upgrade() else {
                return;
            };
            if let Ok(write_clone) = stream.try_clone() {
                if let Ok(mut s) = shared.lock() {
                    s.writer = Some(write_clone);
                    s.guest_ready = false;
                    // cfgd sends the mods report again on request, so a report
                    // pushed while nothing was connected is not lost.
                    let status = "mods status\n".to_string();
                    if !s.pending.contains(&status) {
                        s.pending.push(status);
                    }
                }
            }
        }

        let mut reader = BufReader::new(stream);
        let mut buf = Vec::new();
        while let Ok(Some(line)) = next_line(&mut reader, &mut buf) {
            let Some(shared) = shared.upgrade() else {
                return;
            };
            handle_line(&line, &shared);
        }

        // The connection closed. Drop the writer and clear the pc_link reply,
        // so that a QEMU restart sends the command again. The mods report belongs to the boot
        // that ended; the next connection requests it again.
        let Some(shared) = shared.upgrade() else {
            return;
        };
        if let Ok(mut s) = shared.lock() {
            s.writer = None;
            s.guest_ready = false;
            s.pc_link_state = None;
            s.pc_link_failures = 0;
            s.mods = ModsReport::Unknown;
            s.mods_partial = None;
            s.mod_logs_partial.clear();
            s.mod_logs.clear();
        }
        drop(shared);
        thread::sleep(RECONNECT_DELAY);
    }
}

/// Send `ping` until cfgd answers on the current connection, because cfgd's
/// own pushes need the audio driver. The pings bypass the queue, and the loop
/// ends when the client is dropped.
fn ping_loop(shared: std::sync::Weak<Mutex<Shared>>) {
    while let Some(shared) = shared.upgrade() {
        if let Ok(mut s) = shared.lock() {
            if !s.guest_ready {
                if let Some(w) = s.writer.as_mut() {
                    let _ = w.write_all(b"ping\n").and_then(|()| w.flush());
                }
            }
        }
        drop(shared);
        thread::sleep(PING_PERIOD);
    }
}

/// A line from the guest proves that cfgd is reading. Mark the guest ready
/// and write the commands queued in the meantime.
fn flush_pending(shared: &Arc<Mutex<Shared>>) {
    let Ok(mut s) = shared.lock() else { return };
    if s.guest_ready {
        return;
    }
    s.guest_ready = true;
    let queued = std::mem::take(&mut s.pending);
    let Some(w) = s.writer.as_mut() else { return };
    for line in queued {
        if w.write_all(line.as_bytes()).is_err() || w.flush().is_err() {
            break;
        }
    }
}

/// The maximum length of a line from the guest, in bytes. A longer line is
/// read to its end and dropped.
const LINE_MAX: usize = 64 << 10;

/// Read the next line from the guest, without its line ending; `None` when
/// the stream ends. A line longer than [`LINE_MAX`] is returned empty.
fn next_line(reader: &mut impl BufRead, buf: &mut Vec<u8>) -> std::io::Result<Option<String>> {
    buf.clear();
    let mut over = false;
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            if buf.is_empty() && !over {
                return Ok(None);
            }
            break;
        }
        let end = chunk.iter().position(|&b| b == b'\n');
        let part = &chunk[..end.unwrap_or(chunk.len())];
        if over || buf.len() + part.len() > LINE_MAX {
            over = true;
            buf.clear();
        } else {
            buf.extend_from_slice(part);
        }
        let used = end.map_or(chunk.len(), |i| i + 1);
        reader.consume(used);
        if end.is_some() {
            break;
        }
    }
    let line = String::from_utf8_lossy(buf);
    Ok(Some(line.trim_end_matches('\r').to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_oversize_line_is_dropped_and_reading_goes_on() {
        let long = "x".repeat(LINE_MAX + 1);
        let text = format!("a\r\n{long}\nb\nc");
        let mut reader = std::io::Cursor::new(text.into_bytes());
        let mut buf = Vec::new();
        let mut lines = Vec::new();
        while let Some(line) = next_line(&mut reader, &mut buf).unwrap() {
            lines.push(line);
        }
        assert_eq!(lines, ["a", "", "b", "c"]);
    }
}
