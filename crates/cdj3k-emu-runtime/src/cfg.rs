//! CfgClient - bidirectional bridge to the guest's `cdj3k-cfgd` daemon.
//!
//! One Unix socket (`{sock_dir}/cfg.sock`) carries USB attach/detach commands,
//! `set`/`get` for the whitelisted virtio_snd sysfs parameters, an
//! unsolicited 3-second latency push, and the boot's mods report.
//!
//! Wire protocol (line-based, ASCII, '\n'-terminated):
//!
//!   host → guest
//!     usb attach              - invoke /usr/sbin/usb-external-attach.sh
//!     usb detach              - no-op (EP122 handles unmount)
//!     set <name> <value>      - write to a whitelisted sysfs param
//!     get <name>              - request a `param` response
//!     mods status             - request the mods report
//!                               (sent on every connection)
//!     mods log <name>         - request a mod's journal for this boot
//!
//!   guest → host
//!     usb_state <0|1>         - emitted by the in-guest USB hooks
//!     param <name> <value>    - response to `get` or unsolicited push
//!     latency <g>,<h>,<t>     - pushed every 3s by cdj3k-cfgd
//!     pc_link on|off          - confirms a host-issued pc_link toggle
//!     pc_link on_failed|off_failed - systemctl returned non-zero
//!     mods off|pending        - the boot has no mods / the runner has not finished
//!     mods begin, mod <name> …, mods end
//!                             - the runner's report ([`crate::mods_report::ModOutcome`])
//!     log_begin <name>, log <name> <line>, log_end <name>
//!                             - a mod's journal, in answer to each `mods log`
//!
//! Connection is lazy and self-healing: the reader thread reconnects on EOF
//! / connect failure, the writer methods retry briefly while QEMU is still
//! bringing the port up.

use cdj3k_emu_platform::local_socket::LocalStream as UnixStream;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::mods_report::{ModsReport, Partial};

/// Backoff between reader-thread reconnect attempts. Long enough not to thrash
/// QEMU while it's spinning up its virtio-serial port, short enough that the
/// "first frame" UX (latency label, USB state) lights up promptly.
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
    /// Latest USB mount state from the guest (`true` = mounted).
    usb_state: Option<bool>,
    /// Latest latency push.
    latency: Option<Latency>,
    /// Most recent `param <name> <value>` response per name.
    params: HashMap<String, String>,
    /// Last confirmed PC-link bridge state from the guest.
    /// `Some(true)` = bridge running, `Some(false)` = bridge stopped,
    /// `None` = no `pc_link` response observed yet.
    pc_link_state: Option<bool>,
    /// `pc_link` replies that reported a failure rather than a state.  An
    /// unanswered command means the guest has not opened the port yet; a
    /// failure reply means cfgd ran systemctl and it did not work.
    pc_link_failures: u32,
    /// Writer half - `None` while disconnected.
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
    /// Spawn the background reader thread and return a handle.
    /// `sock_dir` is e.g. `/tmp/cdj3k-emu/instance-1`; the socket is `{sock_dir}/cfg.sock`.
    pub fn new(sock_dir: &std::path::Path) -> Self {
        let sock_path = sock_dir.join("cfg.sock");
        let shared = Arc::new(Mutex::new(Shared::default()));

        let path = sock_path.clone();
        let weak = Arc::downgrade(&shared);
        thread::Builder::new()
            .name("cfg-stream".into())
            .spawn(move || reader_loop(path, weak))
            .expect("spawn cfg-stream thread");

        let weak = Arc::downgrade(&shared);
        thread::Builder::new()
            .name("cfg-ping".into())
            .spawn(move || ping_loop(weak))
            .expect("spawn cfg-ping thread");

        Self { shared }
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

    /// Latest confirmed PC-link bridge state.  `None` if the host hasn't
    /// toggled it yet (or no response has come back).
    pub fn pc_link_state(&self) -> Option<bool> {
        self.shared.lock().ok()?.pc_link_state
    }

    /// Forget the last pc_link echo so the reconcile re-issues on a state change.
    pub fn reset_pc_link_state(&self) {
        if let Ok(mut s) = self.shared.lock() {
            s.pc_link_state = None;
            s.pc_link_failures = 0;
        }
    }

    /// Count of `pc_link *_failed` / `?` replies since the last reset.
    pub fn pc_link_failures(&self) -> u32 {
        self.shared.lock().map(|s| s.pc_link_failures).unwrap_or(0)
    }

    /// `true` once the reader thread has connected to the cfg unix socket
    /// (and thus stashed a live writer half for the fast send_line path).
    /// Independent of any guest-side activity; proves only that the QEMU
    /// chardev is reachable and cfgd has accepted the host's connection on
    /// its end.  Used as an audio-independent "cfg channel is live" gate
    /// for one-shot cold-start commands like `pc_link on`.
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

    /// The guest's mods report for this boot.
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

    /// Send `usb attach\n`. Retries briefly while the port is still coming up.
    pub fn usb_attach(&self) -> std::io::Result<()> {
        self.send_line("usb attach\n")
    }

    /// Send `usb detach\n`. No-op on the guest side (EP122 handles unmount).
    pub fn usb_detach(&self) -> std::io::Result<()> {
        self.send_line("usb detach\n")
    }

    /// Toggle the PC-link bridge in the guest.  `on=true` starts
    /// `cdj3k-pc-link-bridge.service` which begins pumping the gadget's
    /// HID + MIDI endpoints out the `cdj3k.usb-link` virtio-serial port;
    /// `on=false` stops it, which the host side observes as a socket EOF.
    pub fn pc_link(&self, on: bool) -> std::io::Result<()> {
        self.send_line(if on { "pc_link on\n" } else { "pc_link off\n" })
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
        let Ok(mut s) = self.shared.lock() else {
            return Err(std::io::Error::other("cfg state poisoned"));
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

/// Connect, read cfgd's lines, reconnect after QEMU goes; ends once the client
/// is dropped, since QEMU serves one connection on the socket.
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

        // Stash a clone for writers.
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

        // Disconnected: drop the writer and clear the pc_link echo so a QEMU
        // restart re-issues the command. The mods report belongs to the boot
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

/// `ping` until cfgd answers on the current connection; cfgd's own pushes
/// need the audio driver. Bypasses the queue. Ends when the client is dropped.
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

/// A line from the guest proves cfgd is reading: mark it ready and write
/// whatever was queued meanwhile.
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

fn handle_line(line: &str, shared: &Arc<Mutex<Shared>>) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    flush_pending(shared);

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

    if handle_mods_line(line, shared) {
        return;
    }

    if let Some(rest) = line.strip_prefix("pc_link ") {
        // Guest replies: "on", "off", "on_failed", "off_failed", "?".
        // Anything else is a protocol mismatch; log and ignore.
        let state = match rest.trim() {
            "on" => Some(true),
            "off" => Some(false),
            "on_failed" | "off_failed" | "?" => {
                eprintln!("cfg: guest reported pc_link {}", rest.trim());
                if let Ok(mut s) = shared.lock() {
                    s.pc_link_failures = s.pc_link_failures.saturating_add(1);
                }
                None
            }
            other => {
                eprintln!("cfg: unrecognised pc_link response '{other}'");
                return;
            }
        };
        if let Some(v) = state {
            if let Ok(mut s) = shared.lock() {
                s.pc_link_state = Some(v);
            }
        }
    }
}

/// The maximum number of lines kept from a mod's journal (cfgd sends at
/// most 500).
const MOD_LOG_LINES: usize = 600;
/// The maximum length of a journal line, in bytes; a longer line is cut.
const MOD_LOG_LINE_MAX: usize = 1024;
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

/// `s` truncated to at most `max` bytes, at a character boundary.
fn clip(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Handle a `mods`, `mod` or `log*` line; `false` for any other line.
/// Journal lines are kept only for a mod whose journal
/// [`CfgClient::request_mod_log`] requested, and only the answer to the
/// latest request is kept.
fn handle_mods_line(line: &str, shared: &Arc<Mutex<Shared>>) -> bool {
    let Ok(mut s) = shared.lock() else {
        return true;
    };
    if let Some(name) = line.strip_prefix("log_begin ") {
        if let Some((_, lines)) = s.mod_logs_partial.get_mut(name) {
            lines.clear();
        }
    } else if let Some(rest) = line.strip_prefix("log ") {
        let (name, text) = rest.split_once(' ').unwrap_or((rest, ""));
        if let Some((_, lines)) = s.mod_logs_partial.get_mut(name) {
            if lines.len() < MOD_LOG_LINES {
                lines.push(clip(text, MOD_LOG_LINE_MAX).to_string());
            }
        }
    } else if let Some(name) = line.strip_prefix("log_end ") {
        if let Some((due, _)) = s.mod_logs_partial.get_mut(name) {
            *due -= 1;
            if *due == 0 {
                let (_, lines) = s.mod_logs_partial.remove(name).unwrap_or_default();
                s.mod_logs.insert(name.to_string(), lines);
            }
        }
    } else if let Some(rest) = line.strip_prefix("mods ") {
        match rest {
            "off" => s.mods = ModsReport::Off,
            "pending" => s.mods = ModsReport::Pending,
            "begin" => s.mods_partial = Some(Partial::default()),
            // An end without its begin is the tail of a report sent before
            // the host connected.
            "end" => match s.mods_partial.take() {
                Some(partial) => s.mods = partial.done(),
                None => eprintln!("cfg: mods end without mods begin, ignored"),
            },
            other => eprintln!("cfg: unrecognised mods line '{other}'"),
        }
    } else if let Some(rest) = line.strip_prefix("mod ") {
        if !s.mods_partial.as_mut().is_some_and(|p| p.push(rest)) {
            eprintln!("cfg: unexpected mod line '{rest}'");
        }
    } else {
        return false;
    }
    true
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

    #[test]
    fn a_journal_asked_twice_keeps_the_last_answer_whole() {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let feed = |lines: &[&str]| {
            for l in lines {
                assert!(handle_mods_line(l, &shared));
            }
        };
        let log = |name: &str| shared.lock().unwrap().mod_logs.get(name).cloned();
        shared
            .lock()
            .unwrap()
            .mod_logs_partial
            .insert("a".into(), (1, Vec::new()));
        feed(&["log_begin a", "log a one"]);
        // Requested again while the first answer is still arriving.
        shared
            .lock()
            .unwrap()
            .mod_logs_partial
            .get_mut("a")
            .unwrap()
            .0 += 1;
        feed(&["log a two", "log_end a", "log b stray", "log_end b"]);
        assert_eq!(log("a"), None);
        assert_eq!(log("b"), None, "never asked for");
        feed(&[
            "log_begin a",
            "log a one",
            "log a",
            "log a two",
            "log_end a",
        ]);
        assert_eq!(log("a").unwrap(), ["one", "", "two"]);
    }
}
