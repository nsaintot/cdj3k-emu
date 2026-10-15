//! `DevClient` runs shell commands in the guest through `cdj3k-devd`, which a
//! boot carries only in Developer Mode. It talks to devd on
//! `{sock_dir}/dev.sock`, the `cdj3k.dev` virtio-serial port.
//!
//! The wire protocol uses ASCII lines that end in `\n`; `guest/devd/devd.c`
//! describes it:
//!
//!   Host to guest
//!     exec <id> <command>   - run a command as job <id>
//!     kill <id>             - kill the job's process group
//!     ping                  - ask whether devd is there
//!
//!   Guest to host
//!     hello 1               - devd opened the port
//!     pong                  - the answer to `ping`
//!     out <id> <text>       - one line of the job's output
//!     exit <id> <status>    - the job ended
//!     error <id> <why>      - the job did not start
//!
//! The client connects in the background and reconnects after a drop, like
//! [`crate::cfg::CfgClient`]. It sends `ping` on each connection, because a
//! devd that was blocked writing while the host was away does not say hello
//! again. When the connection drops, or devd says hello again, every running
//! job ends with [`JobEvent::Lost`]. Output that is not UTF-8 arrives with
//! U+FFFD in place of the invalid bytes.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use cdj3k_emu_platform::local_socket::LocalStream as UnixStream;

/// How long the reader waits before it connects again.
const RECONNECT_DELAY: Duration = Duration::from_secs(1);
/// The longest line devd reads; it drops a longer one unanswered.
const DEVD_LINE_MAX: usize = 4095;

/// What a job reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobEvent {
    /// One line of output.
    Out(String),
    /// The job ended with this exit code, or 128 plus a signal number.
    Exit(i32),
    /// The job did not start.
    Error(String),
    /// The connection to devd closed, or devd restarted, while the job ran.
    Lost,
}

#[derive(Default)]
struct Shared {
    writer: Option<UnixStream>,
    jobs: HashMap<u32, Sender<JobEvent>>,
}

/// The host's side of devd, for one slot. Dropping it stops its reader.
pub struct DevClient {
    shared: Arc<Mutex<Shared>>,
    /// Whether devd has answered on the current connection, with `hello` or
    /// `pong`.
    up: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    next_id: AtomicU32,
}

/// A running job. Dropping it kills the job unless it has ended.
pub struct Job {
    id: u32,
    rx: Receiver<JobEvent>,
    shared: Arc<Mutex<Shared>>,
    ended: bool,
}

impl DevClient {
    /// Connect to `{sock_dir}/dev.sock` in the background, and again after
    /// each disconnect.
    pub fn new(sock_dir: PathBuf) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let up = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let sock = sock_dir.join("dev.sock");
        {
            let (shared, up, stop) = (Arc::clone(&shared), Arc::clone(&up), Arc::clone(&stop));
            thread::Builder::new()
                .name("dev-client".into())
                .spawn(move || reader_loop(&sock, &shared, &up, &stop))
                .expect("the dev-client thread could not be started");
        }
        Self {
            shared,
            up,
            stop,
            next_id: AtomicU32::new(1),
        }
    }

    /// Whether devd answers on the current connection.
    pub fn is_up(&self) -> bool {
        self.up.load(Ordering::Relaxed)
    }

    /// Run `command` with `/bin/sh -c` in the guest.
    pub fn start(&self, command: &str) -> io::Result<Job> {
        if command.contains('\n') {
            return Err(io::Error::other("a guest command is one line"));
        }
        let id = self.next_id.load(Ordering::Relaxed);
        let line = format!("exec {id} {command}\n");
        if line.len() > DEVD_LINE_MAX {
            return Err(io::Error::other(format!(
                "a guest command is at most {} bytes",
                DEVD_LINE_MAX - (line.len() - command.len())
            )));
        }
        if !self.is_up() {
            return Err(io::Error::other(
                "the guest's devd is not up; it starts with the first boot in Developer Mode",
            ));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let line = format!("exec {id} {command}\n");
        let (tx, rx) = mpsc::channel();
        let mut s = self.shared.lock().unwrap();
        let writer = s
            .writer
            .as_mut()
            .ok_or_else(|| io::Error::other("the guest's devd is not connected"))?;
        writer.write_all(line.as_bytes())?;
        s.jobs.insert(id, tx);
        drop(s);
        Ok(Job {
            id,
            rx,
            shared: Arc::clone(&self.shared),
            ended: false,
        })
    }
}

impl Drop for DevClient {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let mut s = self.shared.lock().unwrap();
        if let Some(w) = s.writer.take() {
            let _ = w.shutdown(std::net::Shutdown::Both);
        }
    }
}

impl Job {
    /// The events that have arrived, oldest first.
    pub fn take(&mut self) -> Vec<JobEvent> {
        let events: Vec<JobEvent> = self.rx.try_iter().collect();
        if events.iter().any(|e| !matches!(e, JobEvent::Out(_))) {
            self.ended = true;
        }
        events
    }

    /// Kill the job's process group.
    pub fn kill(&mut self) {
        if self.ended {
            return;
        }
        let mut s = self.shared.lock().unwrap();
        if let Some(w) = s.writer.as_mut() {
            let _ = w.write_all(format!("kill {}\n", self.id).as_bytes());
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.kill();
        self.shared.lock().unwrap().jobs.remove(&self.id);
    }
}

fn reader_loop(sock: &Path, shared: &Mutex<Shared>, up: &AtomicBool, stop: &AtomicBool) {
    while !stop.load(Ordering::Acquire) {
        let Ok(stream) = UnixStream::connect(sock) else {
            thread::sleep(RECONNECT_DELAY);
            continue;
        };
        let Ok(write_half) = stream.try_clone() else {
            thread::sleep(RECONNECT_DELAY);
            continue;
        };
        let mut writer = write_half;
        if writer.write_all(b"ping\n").is_err() {
            thread::sleep(RECONNECT_DELAY);
            continue;
        }
        shared.lock().unwrap().writer = Some(writer);
        let mut reader = BufReader::new(stream);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf);
                    on_line(line.trim_end_matches('\n'), shared, up);
                }
            }
        }
        up.store(false, Ordering::Relaxed);
        let mut s = shared.lock().unwrap();
        s.writer = None;
        lose_all(&mut s);
        drop(s);
        thread::sleep(RECONNECT_DELAY);
    }
}

fn lose_all(s: &mut Shared) {
    for (_, tx) in s.jobs.drain() {
        let _ = tx.send(JobEvent::Lost);
    }
}

fn on_line(line: &str, shared: &Mutex<Shared>, up: &AtomicBool) {
    let mut s = shared.lock().unwrap();
    match parse(line) {
        Some(Line::Hello) => {
            lose_all(&mut s);
            up.store(true, Ordering::Relaxed);
        }
        Some(Line::Pong) => up.store(true, Ordering::Relaxed),
        Some(Line::Job(id, event)) => {
            let done = !matches!(event, JobEvent::Out(_));
            if let Some(tx) = s.jobs.get(&id) {
                let _ = tx.send(event);
            }
            if done {
                s.jobs.remove(&id);
            }
        }
        None => eprintln!("cdj3k-emu: devd sent a line that is not understood: {line}"),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Line {
    Hello,
    Pong,
    Job(u32, JobEvent),
}

fn parse(line: &str) -> Option<Line> {
    if line.starts_with("hello ") || line == "hello" {
        return Some(Line::Hello);
    }
    if line == "pong" {
        return Some(Line::Pong);
    }
    let (verb, rest) = line.split_once(' ')?;
    let (id, text) = rest.split_once(' ').unwrap_or((rest, ""));
    let id: u32 = id.parse().ok()?;
    let event = match verb {
        "out" => JobEvent::Out(text.to_string()),
        "exit" => JobEvent::Exit(text.trim().parse().ok()?),
        "error" => JobEvent::Error(text.to_string()),
        _ => return None,
    };
    Some(Line::Job(id, event))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_parse_into_job_events() {
        assert_eq!(parse("hello 1"), Some(Line::Hello));
        assert_eq!(parse("pong"), Some(Line::Pong));
        assert_eq!(
            parse("out 3 a b  c"),
            Some(Line::Job(3, JobEvent::Out("a b  c".into())))
        );
        assert_eq!(
            parse("out 3 "),
            Some(Line::Job(3, JobEvent::Out("".into())))
        );
        assert_eq!(parse("exit 3 137"), Some(Line::Job(3, JobEvent::Exit(137))));
        assert_eq!(
            parse("error 4 16 jobs are already running"),
            Some(Line::Job(
                4,
                JobEvent::Error("16 jobs are already running".into())
            ))
        );
        assert_eq!(parse("exit x 0"), None);
        assert_eq!(parse("ping"), None);
    }

    #[test]
    fn a_command_longer_than_devd_reads_is_refused() {
        let client = DevClient::new(std::env::temp_dir().join("cdj3k-dev-test-none"));
        let err = client.start(&"x".repeat(DEVD_LINE_MAX)).err().unwrap();
        assert!(err.to_string().contains("at most"), "{err}");
    }
}
