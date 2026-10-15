//! The control listener. One thread accepts connections, and each connection
//! has a reader thread and a writer thread. A connection sends one request at
//! a time: the reader parses the line, hands the [`Command`] to the window
//! through [`ControlServer::next`] and queues the window's reply as one JSON
//! line. A line that does not parse is answered at once.
//!
//! The window can also push events to a connection at any time through the
//! connection's [`EventSink`]. The writer thread sends replies and events in
//! the order they were queued.
//!
//! The window learns that a connection closed through [`Incoming::Closed`],
//! so it can let go of what that connection held.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::Value;

use crate::command::Command;

/// Identifies a connection for as long as the server runs.
pub type ConnId = u64;

/// The longest request line a connection may send.
const MAX_LINE: u64 = 4096;
/// How often the accept loop looks at the stop flag.
const ACCEPT_POLL: Duration = Duration::from_millis(50);
/// How often a connection whose request is pending checks that the client is
/// still there, so that the window releases what it holds without waiting
/// for the reply.
const CLOSE_POLL: Duration = Duration::from_millis(250);

/// What a connection sends to the window.
pub enum Incoming {
    Request {
        conn: ConnId,
        command: Command,
        reply: Responder,
    },
    /// The connection closed; release what it held.
    Closed(ConnId),
}

/// The window answers a request through this, at once or later.
///
/// The reply is queued on the connection when it is sent, so an event the
/// window sends after it follows it on the wire. A request gets exactly one
/// reply: the window's, or the connection's timeout error, whichever comes
/// first. Dropping the responder unanswered sends an error.
pub struct Responder {
    answered: Arc<Answered>,
    events: EventSink,
}

impl Responder {
    pub fn send(self, reply: impl Into<Value>) {
        self.answered.answer(&self.events, reply.into());
    }

    /// Where events for the request's connection go.
    pub fn events(&self) -> EventSink {
        self.events.clone()
    }
}

impl Drop for Responder {
    fn drop(&mut self) {
        self.answered
            .answer(&self.events, crate::error("the window dropped the request"));
    }
}

/// Whether a request has its reply.
#[derive(Default)]
struct Answered {
    done: Mutex<bool>,
    cv: Condvar,
}

impl Answered {
    /// Queue `reply` unless the request already has one.
    fn answer(&self, sink: &EventSink, reply: Value) {
        let mut done = self.done.lock().unwrap();
        if !*done {
            *done = true;
            sink.send(reply);
            self.cv.notify_all();
        }
    }
}

/// Queues lines for one connection's writer thread.
#[derive(Clone)]
pub struct EventSink {
    tx: Sender<String>,
}

impl EventSink {
    /// Queue `event` as one line. Returns `false` once the connection is
    /// gone.
    pub fn send(&self, event: impl Into<Value>) -> bool {
        let mut line = event.into().to_string();
        line.push('\n');
        self.tx.send(line).is_ok()
    }
}

type Wake = Arc<dyn Fn() + Send + Sync>;
type Live = Arc<Mutex<HashMap<ConnId, TcpStream>>>;

/// Listens on the slot's control port until dropped. Dropping it closes every
/// connection.
pub struct ControlServer {
    rx: Receiver<Incoming>,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
    port: u16,
}

impl ControlServer {
    /// Listen on [`crate::port`] for `instance`, on every interface. `wake`
    /// runs after each message for the window, so the window can take it.
    pub fn bind(instance: u32, wake: impl Fn() + Send + Sync + 'static) -> io::Result<Self> {
        Self::listen(
            SocketAddr::from(([0, 0, 0, 0], crate::port(instance))),
            wake,
        )
    }

    /// Listen on `addr`; port 0 takes any free port.
    pub fn listen(addr: SocketAddr, wake: impl Fn() + Send + Sync + 'static) -> io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let wake: Wake = Arc::new(wake);
        let accept = {
            let stop = Arc::clone(&stop);
            thread::Builder::new()
                .name("control-accept".into())
                .spawn(move || accept_loop(listener, tx, wake, stop))?
        };
        Ok(Self {
            rx,
            stop,
            accept: Some(accept),
            port,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The next message for the window, if one is waiting.
    pub fn next(&self) -> Option<Incoming> {
        self.rx.try_recv().ok()
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
    }
}

fn accept_loop(listener: TcpListener, tx: Sender<Incoming>, wake: Wake, stop: Arc<AtomicBool>) {
    let live: Live = Arc::default();
    let next_id = AtomicU64::new(1);
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                let conn = next_id.fetch_add(1, Ordering::Relaxed);
                if let Err(e) = start_connection(conn, stream, &tx, &wake, &live) {
                    eprintln!("cdj3k-emu: control: a connection could not start: {e}");
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(ACCEPT_POLL),
            Err(e) => {
                eprintln!("cdj3k-emu: control: accepting a connection failed: {e}");
                thread::sleep(ACCEPT_POLL);
            }
        }
    }
    for stream in live.lock().unwrap().values() {
        let _ = stream.shutdown(Shutdown::Both);
    }
}

fn start_connection(
    conn: ConnId,
    stream: TcpStream,
    tx: &Sender<Incoming>,
    wake: &Wake,
    live: &Live,
) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    let _ = stream.set_nodelay(true);
    live.lock().unwrap().insert(conn, stream.try_clone()?);
    let (tx, wake, live) = (tx.clone(), Arc::clone(wake), Arc::clone(live));
    thread::Builder::new()
        .name(format!("control-conn-{conn}"))
        .spawn(move || {
            serve(conn, stream, &tx, &wake);
            let _ = tx.send(Incoming::Closed(conn));
            wake();
            live.lock().unwrap().remove(&conn);
        })?;
    Ok(())
}

/// Answer the connection's requests, one at a time, until it closes.
fn serve(conn: ConnId, stream: TcpStream, tx: &Sender<Incoming>, wake: &Wake) {
    let (Ok(read_half), Ok(probe)) = (stream.try_clone(), stream.try_clone()) else {
        return;
    };
    let Ok(sink) = start_writer(conn, stream) else {
        return;
    };
    let mut reader = BufReader::new(read_half);
    loop {
        let mut line = String::new();
        match (&mut reader).take(MAX_LINE).read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if !line.ends_with('\n') && line.len() as u64 >= MAX_LINE {
            sink.send(crate::error("the request line is too long"));
            return;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match Command::parse(line) {
            Err(e) => {
                if !sink.send(crate::error(e)) {
                    return;
                }
            }
            Ok(command) => {
                if !ask_window(conn, command, tx, wake, &sink, &probe) {
                    return;
                }
            }
        }
    }
}

/// Start the thread that writes the connection's queued lines. It ends when
/// a write fails or every [`EventSink`] is dropped.
fn start_writer(conn: ConnId, mut stream: TcpStream) -> io::Result<EventSink> {
    let (tx, rx) = mpsc::channel::<String>();
    thread::Builder::new()
        .name(format!("control-write-{conn}"))
        .spawn(move || {
            for line in rx {
                if stream.write_all(line.as_bytes()).is_err() {
                    let _ = stream.shutdown(Shutdown::Both);
                    return;
                }
            }
        })?;
    Ok(EventSink { tx })
}

/// Hand `command` to the window and wait until it has a reply. Returns
/// `false` when the server is gone or the client has closed the connection.
fn ask_window(
    conn: ConnId,
    command: Command,
    tx: &Sender<Incoming>,
    wake: &Wake,
    sink: &EventSink,
    probe: &TcpStream,
) -> bool {
    let wait = command.answer_wait();
    let answered = Arc::new(Answered::default());
    let reply = Responder {
        answered: Arc::clone(&answered),
        events: sink.clone(),
    };
    if tx
        .send(Incoming::Request {
            conn,
            command,
            reply,
        })
        .is_err()
    {
        return false;
    }
    wake();
    let deadline = std::time::Instant::now() + wait;
    let mut done = answered.done.lock().unwrap();
    while !*done {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            break;
        }
        done = answered
            .cv
            .wait_timeout_while(done, left.min(CLOSE_POLL), |done| !*done)
            .unwrap()
            .0;
        if !*done && peer_closed(probe) {
            return false;
        }
    }
    if !*done {
        *done = true;
        sink.send(crate::error("the window did not answer in time"));
    }
    true
}

/// Whether the client has closed the connection. Data the client has already
/// sent, such as its next request, does not count.
fn peer_closed(stream: &TcpStream) -> bool {
    if stream.set_nonblocking(true).is_err() {
        return false;
    }
    let mut byte = [0u8; 1];
    let closed = match stream.peek(&mut byte) {
        Ok(0) => true,
        Ok(_) => false,
        Err(e) => e.kind() != io::ErrorKind::WouldBlock,
    };
    let _ = stream.set_nonblocking(false);
    closed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Client;

    #[test]
    fn a_request_reaches_the_window_and_the_reply_comes_back() {
        let server = ControlServer::listen(SocketAddr::from(([127, 0, 0, 1], 0)), || {}).unwrap();
        let port = server.port();
        let worker = thread::spawn(move || {
            let mut closed = false;
            let mut answered = 0;
            while !closed {
                match server.next() {
                    Some(Incoming::Request { command, reply, .. }) => {
                        assert_eq!(command, Command::Hello);
                        reply.send(crate::ok());
                        answered += 1;
                    }
                    Some(Incoming::Closed(_)) => closed = true,
                    None => thread::sleep(Duration::from_millis(5)),
                }
            }
            answered
        });
        let mut client = Client::connect_port("127.0.0.1", port).unwrap();
        assert_eq!(client.request("hello").unwrap()["ok"], true);
        let bad = client.request("frobnicate").unwrap();
        assert_eq!(bad["ok"], false);
        drop(client);
        assert_eq!(worker.join().unwrap(), 1);
    }

    /// Events sent after a reply follow it, and a request the window drops
    /// still gets one reply.
    #[test]
    fn events_follow_the_reply_and_a_dropped_request_is_answered() {
        let server = ControlServer::listen(SocketAddr::from(([127, 0, 0, 1], 0)), || {}).unwrap();
        let port = server.port();
        let worker = thread::spawn(move || loop {
            match server.next() {
                Some(Incoming::Request {
                    command: Command::Hello,
                    reply,
                    ..
                }) => {
                    let events = reply.events();
                    reply.send(crate::ok());
                    events.send(serde_json::json!({ "event": "one" }));
                    events.send(serde_json::json!({ "event": "two" }));
                }
                Some(Incoming::Request { reply, .. }) => drop(reply),
                Some(Incoming::Closed(_)) => return,
                None => thread::sleep(Duration::from_millis(5)),
            }
        });
        let mut client = Client::connect_port("127.0.0.1", port).unwrap();
        assert_eq!(client.request("hello").unwrap()["ok"], true);
        let wait = Duration::from_secs(2);
        assert_eq!(client.next_event(wait).unwrap().unwrap()["event"], "one");
        assert_eq!(client.next_event(wait).unwrap().unwrap()["event"], "two");
        let dropped = client.request("state").unwrap();
        assert_eq!(dropped["error"], "the window dropped the request");
        assert!(client
            .next_event(Duration::from_millis(50))
            .unwrap()
            .is_none());
        drop(client);
        worker.join().unwrap();
    }

    #[test]
    fn a_client_that_leaves_during_a_request_is_noticed_at_once() {
        let server = ControlServer::listen(SocketAddr::from(([127, 0, 0, 1], 0)), || {}).unwrap();
        let port = server.port();
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let worker = thread::spawn(move || {
            let mut pending = Vec::new();
            let started = std::time::Instant::now();
            loop {
                match server.next() {
                    Some(Incoming::Request { reply, .. }) => pending.push(reply),
                    Some(Incoming::Closed(_)) => return started.elapsed(),
                    None => thread::sleep(Duration::from_millis(10)),
                }
            }
        });
        // The window never answers this wait, which could last 10 minutes.
        client.write_all(b"wait boot 600000\n").unwrap();
        thread::sleep(Duration::from_millis(200));
        drop(client);
        let noticed = worker.join().unwrap();
        assert!(noticed < Duration::from_secs(3), "{noticed:?}");
    }
}
