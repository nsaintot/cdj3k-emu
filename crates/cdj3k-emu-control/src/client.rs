//! A connection to a slot's control port.

use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde_json::Value;

use crate::command::{ANSWER_WAIT, MAX_WAIT};

const CONNECT_WAIT: Duration = Duration::from_secs(2);
/// How long a read waits for a reply.
const READ_WAIT: Duration = Duration::from_secs(MAX_WAIT.as_secs() + 2 * ANSWER_WAIT.as_secs());

/// A connection to one slot. It sends one request at a time.
pub struct Client {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    /// The bytes of a line a timed-out read left unfinished.
    partial: Vec<u8>,
    /// Events that arrived while a request waited for its reply.
    events: VecDeque<Value>,
}

impl Client {
    /// Connect to slot `instance` on `host`.
    pub fn connect(host: &str, instance: u32) -> io::Result<Self> {
        Self::connect_port(host, crate::port(instance)).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("{e}; is slot {instance} open with Developer Mode on?"),
            )
        })
    }

    /// Connect to `port` on `host`.
    pub fn connect_port(host: &str, port: u16) -> io::Result<Self> {
        let addr = (host, port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::other(format!("{host} has no address")))?;
        let stream = TcpStream::connect_timeout(&addr, CONNECT_WAIT).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("{host}:{port} did not accept the connection ({e})"),
            )
        })?;
        stream.set_read_timeout(Some(READ_WAIT))?;
        let _ = stream.set_nodelay(true);
        Ok(Self {
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
            partial: Vec::new(),
            events: VecDeque::new(),
        })
    }

    /// Send one request line and return the reply. Events that arrive first
    /// are kept for [`Self::next_event`].
    pub fn request(&mut self, line: &str) -> io::Result<Value> {
        self.writer.write_all(line.trim().as_bytes())?;
        self.writer.write_all(b"\n")?;
        loop {
            let msg = self.read_message()?;
            if msg.get("event").is_some() {
                self.events.push_back(msg);
            } else {
                return Ok(msg);
            }
        }
    }

    /// Return the next event, waiting at most `wait`, or `None` when no event
    /// arrives in that time.
    pub fn next_event(&mut self, wait: Duration) -> io::Result<Option<Value>> {
        if let Some(e) = self.events.pop_front() {
            return Ok(Some(e));
        }
        self.writer
            .set_read_timeout(Some(wait.max(Duration::from_millis(1))))?;
        let msg = self.read_message();
        self.writer.set_read_timeout(Some(READ_WAIT))?;
        match msg {
            Ok(m) => Ok(Some(m)),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    /// Read one JSON line. A read that times out keeps what it got.
    fn read_message(&mut self) -> io::Result<Value> {
        self.reader.read_until(b'\n', &mut self.partial)?;
        if !self.partial.ends_with(b"\n") {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the window closed the connection",
            ));
        }
        let line = std::mem::take(&mut self.partial);
        serde_json::from_slice(&line).map_err(io::Error::other)
    }
}
