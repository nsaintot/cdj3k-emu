//! On a host without local stream sockets, binding and connecting always
//! fail.

use std::io;
use std::path::Path;
use std::time::Duration;

/// This type has no values, because [`LocalListener::bind`] always fails.
pub enum LocalListener {}

impl LocalListener {
    pub fn bind(_path: &Path) -> io::Result<Self> {
        Err(unsupported())
    }

    pub fn accept(&self) -> io::Result<LocalStream> {
        match *self {}
    }
}

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "no local stream sockets on this host",
    )
}

/// This type has no values, because [`LocalStream::connect`] always fails.
pub enum LocalStream {}

impl LocalStream {
    pub fn connect<P: AsRef<Path>>(_path: P) -> io::Result<Self> {
        Err(unsupported())
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
