//! The window that holds a slot listens on `instance-{id}/window.sock` in the
//! runtime root ([`runtime_paths::window_sock_path`]) for as long as the
//! window is open. Other windows send commands to it, for example to restart
//! or empty the slot.
//!
//! The protocol is line-based ASCII, and each line ends with '\n'. A client
//! connects, writes one command and reads one reply line; then the window
//! closes the connection.
//!
//!   client → window
//!     delete            - stop the emulation and empty the slot
//!     quit              - stop the emulation and close, as Quit does
//!     restart-mods      - restart the emulation with the slot's mods
//!     restart-install   - restart the emulation into the finished install
//!     install-ready     - apply the finished install in the slot's staging
//!                         dir now if the emulation is stopped, otherwise when
//!                         it stops
//!     restart-pending   - ask whether a restart would boot different mods;
//!                         the reply is yes or no
//!
//!   window → client
//!     ok                - the command is done or in progress
//!     yes, no           - the window's answer to `restart-pending`
//!     busy              - the window is installing, stopping or closing, or
//!                         did not take the command within 5 s; ask again later
//!     not-running       - the command needs a running emulation
//!     error <why>       - the line is not a command
//!
//! If the socket file is missing, or a connection to it is refused, no window
//! holds the slot.
//!
//! ```text
//! printf 'restart-mods\n' | nc -U /tmp/cdj3k-emu-$(id -u)/instance-2/window.sock
//! ```

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::AcqRel};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use crate::local_socket::{LocalListener, LocalStream};
use crate::runtime_paths;

/// How long the window has to take a command before the client gets `busy`.
/// The window takes commands on each frame, and each command wakes it.
const ANSWER_WAIT: Duration = Duration::from_secs(5);
/// How long a client waits for a reply. It is longer than [`ANSWER_WAIT`], so
/// a client whose command the window did not take receives `busy` before its
/// read times out.
const REPLY_WAIT: Duration = Duration::from_secs(10);
/// A connection that sends no command for this long is closed.
const COMMAND_WAIT: Duration = Duration::from_secs(10);
/// Each side reads at most this many bytes per line. Commands and replies are
/// much shorter.
const MAX_LINE: u64 = 256;
/// How long the accept loop waits after a failed accept before it tries again.
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Delete,
    Quit,
    RestartMods,
    RestartInstall,
    InstallReady,
    RestartPending,
}

impl Command {
    pub const ALL: [Command; 6] = [
        Command::Delete,
        Command::Quit,
        Command::RestartMods,
        Command::RestartInstall,
        Command::InstallReady,
        Command::RestartPending,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Delete => "delete",
            Self::Quit => "quit",
            Self::RestartMods => "restart-mods",
            Self::RestartInstall => "restart-install",
            Self::InstallReady => "install-ready",
            Self::RestartPending => "restart-pending",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    Ok,
    Yes,
    No,
    Busy,
    NotRunning,
}

impl Reply {
    pub const ALL: [Reply; 5] = [
        Reply::Ok,
        Reply::Yes,
        Reply::No,
        Reply::Busy,
        Reply::NotRunning,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Yes => "yes",
            Self::No => "no",
            Self::Busy => "busy",
            Self::NotRunning => "not-running",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == s)
    }
}

/// Send `command` to the window that holds slot `id` and wait for its reply.
/// The result is `None` if no window holds the slot.
pub fn send(id: u32, command: Command) -> io::Result<Option<Reply>> {
    send_at(&runtime_paths::window_sock_path(id), command)
}

fn send_at(path: &Path, command: Command) -> io::Result<Option<Reply>> {
    // No window has run the slot since the runtime folder was removed. On
    // Windows, connecting into a missing folder fails with WSAENETDOWN, which
    // is not one of the error kinds below.
    if path.parent().is_some_and(|dir| !dir.is_dir()) {
        return Ok(None);
    }
    let stream = match LocalStream::connect(path) {
        Ok(stream) => stream,
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound
                    | io::ErrorKind::ConnectionRefused
                    | io::ErrorKind::Unsupported
            ) =>
        {
            return Ok(None)
        }
        Err(e) => return Err(e),
    };
    stream.set_read_timeout(Some(REPLY_WAIT))?;
    stream.set_write_timeout(Some(REPLY_WAIT))?;
    writeln!(stream.try_clone()?, "{}", command.as_str())?;
    let mut line = String::new();
    BufReader::new(stream).take(MAX_LINE).read_line(&mut line)?;
    let line = line.trim();
    if let Some(why) = line.strip_prefix("error ") {
        return Err(io::Error::other(why.to_string()));
    }
    Reply::parse(line).map(Some).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected reply {line:?}"),
        )
    })
}

/// `PendingReply` holds the reply to a command sent on its own thread, so that
/// a window that is slow to answer does not block the sender's frame loop.
/// Poll it with [`PendingReply::take`].
pub struct PendingReply {
    rx: Receiver<io::Result<Option<Reply>>>,
}

impl PendingReply {
    /// Send `command` to the window that holds slot `id`. `wake` is called
    /// when the reply arrives.
    pub fn send(id: u32, command: Command, wake: impl FnOnce() + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        // If the thread cannot start, `tx` is dropped and `take` returns an
        // error.
        let _ = std::thread::Builder::new()
            .name("cdj3k-emu-window-send".into())
            .spawn(move || {
                let _ = tx.send(send(id, command));
                wake();
            });
        Self { rx }
    }

    /// Return the reply if it has arrived, and `None` otherwise.
    pub fn take(&self) -> Option<io::Result<Option<Reply>>> {
        match self.rx.try_recv() {
            Ok(reply) => Some(reply),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err(io::Error::other("the sending thread ended")))
            }
        }
    }
}

/// `Pending` holds a command that a client sent and the window has not
/// answered yet. The window and the connection thread both try to set
/// `taken`. If the window sets it first, the window answers. If
/// [`ANSWER_WAIT`] passes first, the connection thread sets it and answers
/// `busy`.
struct Pending {
    command: Command,
    taken: Arc<AtomicBool>,
    reply: Sender<Reply>,
}

/// The window uses a `Responder` to reply to a command from
/// [`WindowSocket::next`]. Dropping it without a reply sends `busy`.
pub struct Responder(Sender<Reply>);

impl Responder {
    pub fn reply(self, reply: Reply) {
        let _ = self.0.send(reply);
    }
}

/// A `WindowSocket` listens on `instance-{id}/window.sock` for the window that
/// holds slot `id`. It lasts as long as the window: dropping it removes the
/// socket file, and its accept thread ends with the process.
pub struct WindowSocket {
    path: PathBuf,
    commands: Receiver<Pending>,
}

impl WindowSocket {
    /// Listen on the window socket of slot `id`. Only the process that holds
    /// the slot's claim may call this. The connection thread calls `wake` for
    /// each command, so that the window calls [`next`](Self::next) on its next
    /// frame.
    pub fn bind(id: u32, wake: impl Fn() + Send + Sync + 'static) -> io::Result<Self> {
        runtime_paths::ensure_runtime_base_dir()?;
        std::fs::create_dir_all(runtime_paths::instance_dir(id))?;
        Self::bind_at(
            runtime_paths::window_sock_path(id),
            ANSWER_WAIT,
            Arc::new(wake),
        )
    }

    fn bind_at(
        path: PathBuf,
        answer_wait: Duration,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<Self> {
        let listener = LocalListener::bind(&path)?;
        let (tx, commands) = mpsc::channel();
        std::thread::Builder::new()
            .name("cdj3k-emu-window-socket".into())
            .spawn(move || accept_loop(&listener, &tx, answer_wait, &wake))?;
        Ok(Self { path, commands })
    }

    /// Return the next command that a client is waiting on, if any. Commands
    /// older than [`ANSWER_WAIT`] already got `busy` and are skipped.
    pub fn next(&self) -> Option<(Command, Responder)> {
        while let Ok(pending) = self.commands.try_recv() {
            if !pending.taken.swap(true, AcqRel) {
                return Some((pending.command, Responder(pending.reply)));
            }
        }
        None
    }
}

impl Drop for WindowSocket {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn accept_loop(
    listener: &LocalListener,
    tx: &Sender<Pending>,
    answer_wait: Duration,
    wake: &Arc<dyn Fn() + Send + Sync>,
) {
    loop {
        match listener.accept() {
            Ok(stream) => {
                let (tx, wake) = (tx.clone(), wake.clone());
                let _ = std::thread::Builder::new()
                    .name("cdj3k-emu-window-conn".into())
                    .spawn(move || {
                        let _ = serve(stream, &tx, answer_wait, &*wake);
                    });
            }
            Err(_) => std::thread::sleep(ACCEPT_ERROR_BACKOFF),
        }
    }
}

fn serve(
    stream: LocalStream,
    tx: &Sender<Pending>,
    answer_wait: Duration,
    wake: &(dyn Fn() + Send + Sync),
) -> io::Result<()> {
    stream.set_read_timeout(Some(COMMAND_WAIT))?;
    let mut writer = stream.try_clone()?;
    let mut line = String::new();
    if BufReader::new(stream).take(MAX_LINE).read_line(&mut line)? == 0 {
        return Ok(());
    }
    let word = line.trim();
    match Command::parse(word) {
        Some(command) => writeln!(writer, "{}", ask(command, tx, answer_wait, wake).as_str())?,
        None => writeln!(writer, "error unknown command {word:?}")?,
    }
    // Close the connection now, so that the client sees the end of the
    // stream even while a duplicate of this socket's handle is still open.
    writer.shutdown(std::net::Shutdown::Both)
}

/// Pass `command` to the window and wait for its reply.
fn ask(
    command: Command,
    tx: &Sender<Pending>,
    answer_wait: Duration,
    wake: &(dyn Fn() + Send + Sync),
) -> Reply {
    let (reply, reply_rx) = mpsc::channel();
    let taken = Arc::new(AtomicBool::new(false));
    let pending = Pending {
        command,
        taken: taken.clone(),
        reply,
    };
    if tx.send(pending).is_err() {
        // The window is closing.
        return Reply::Busy;
    }
    wake();
    match reply_rx.recv_timeout(answer_wait) {
        Ok(r) => r,
        Err(RecvTimeoutError::Disconnected) => Reply::Busy,
        // Withdraw the command, unless the window has just taken it.
        Err(RecvTimeoutError::Timeout) if !taken.swap(true, AcqRel) => Reply::Busy,
        Err(RecvTimeoutError::Timeout) => reply_rx.recv().unwrap_or(Reply::Busy),
    }
}

#[cfg(all(test, any(unix, windows)))]
mod tests {
    use super::*;
    use std::time::Instant;

    fn test_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("cdj3k-{tag}-{}.sock", std::process::id()))
    }

    fn server(tag: &str, answer_wait: Duration) -> WindowSocket {
        WindowSocket::bind_at(test_path(tag), answer_wait, Arc::new(|| {})).unwrap()
    }

    /// Poll `server` the way a window does on each frame.
    fn take(server: &WindowSocket) -> (Command, Responder) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(taken) = server.next() {
                return taken;
            }
            assert!(Instant::now() < deadline, "no command arrived");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_command_gets_the_windows_reply() {
        let server = server("ws-reply", ANSWER_WAIT);
        let path = server.path.clone();
        let client = std::thread::spawn(move || send_at(&path, Command::RestartMods));
        let (command, responder) = take(&server);
        assert_eq!(command, Command::RestartMods);
        responder.reply(Reply::NotRunning);
        assert_eq!(client.join().unwrap().unwrap(), Some(Reply::NotRunning));
    }

    #[test]
    fn a_command_the_window_does_not_take_is_answered_busy_and_dropped() {
        let server = server("ws-busy", Duration::from_millis(100));
        assert_eq!(
            send_at(&server.path, Command::Quit).unwrap(),
            Some(Reply::Busy)
        );
        assert!(server.next().is_none(), "a lapsed command is not acted on");
    }

    #[test]
    fn an_unanswered_responder_answers_busy() {
        let server = server("ws-dropped", ANSWER_WAIT);
        let path = server.path.clone();
        let client = std::thread::spawn(move || send_at(&path, Command::Delete));
        drop(take(&server));
        assert_eq!(client.join().unwrap().unwrap(), Some(Reply::Busy));
    }

    #[test]
    fn no_socket_or_a_stale_one_means_no_window() {
        let path = test_path("ws-stale");
        let _ = std::fs::remove_file(&path);
        assert_eq!(send_at(&path, Command::Quit).unwrap(), None);
        // A listener that goes away leaves its socket file behind.
        drop(LocalListener::bind(&path).unwrap());
        assert!(path.exists());
        assert_eq!(send_at(&path, Command::Quit).unwrap(), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_socket_goes_with_the_server() {
        let server = server("ws-drop", ANSWER_WAIT);
        let path = server.path.clone();
        assert!(path.exists());
        drop(server);
        assert!(!path.exists());
        assert_eq!(send_at(&path, Command::Quit).unwrap(), None);
    }

    #[test]
    fn an_unknown_command_is_an_error_and_the_window_closes_the_connection() {
        let server = server("ws-unknown", ANSWER_WAIT);
        let stream = LocalStream::connect(&server.path).unwrap();
        stream.set_read_timeout(Some(REPLY_WAIT)).unwrap();
        writeln!(stream.try_clone().unwrap(), "frobnicate").unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "error unknown command \"frobnicate\"");
        line.clear();
        assert_eq!(
            reader.read_line(&mut line).unwrap(),
            0,
            "the window hung up"
        );
        assert!(server.next().is_none());
    }

    #[test]
    fn a_socket_in_a_missing_folder_means_no_window() {
        let path = test_path("ws-missing").join("window.sock");
        assert_eq!(send_at(&path, Command::Quit).unwrap(), None);
    }

    #[test]
    fn a_pending_reply_arrives_on_a_later_poll() {
        let (woken_tx, woken) = mpsc::channel();
        // No window holds slot u32::MAX, so the reply is `None`.
        let pending = PendingReply::send(u32::MAX, Command::Quit, move || {
            let _ = woken_tx.send(());
        });
        woken.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(pending.take().unwrap().unwrap(), None);
    }
}
