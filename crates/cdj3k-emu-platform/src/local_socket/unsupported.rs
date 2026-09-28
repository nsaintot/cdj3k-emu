//! A host with no local stream socket: nothing connects, so no stream exists.

use std::io;
use std::path::Path;
use std::time::Duration;

/// Uninhabited: [`LocalStream::connect`] never returns one.
pub enum LocalStream {}

impl LocalStream {
    pub fn connect<P: AsRef<Path>>(_path: P) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no local stream sockets on this host",
        ))
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        match *self {}
    }

    pub fn set_read_timeout(&self, _dur: Option<Duration>) -> io::Result<()> {
        match *self {}
    }

    pub fn set_write_timeout(&self, _dur: Option<Duration>) -> io::Result<()> {
        match *self {}
    }
}

impl io::Read for LocalStream {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        match *self {}
    }
}

impl io::Write for LocalStream {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        match *self {}
    }

    fn flush(&mut self) -> io::Result<()> {
        match *self {}
    }
}
