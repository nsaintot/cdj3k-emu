//! A Unix domain socket.

use std::io;
use std::os::unix::net::UnixListener;
use std::path::Path;

pub use std::os::unix::net::UnixStream as LocalStream;

pub struct LocalListener(UnixListener);

impl LocalListener {
    /// Listen at `path`, removing any file already at that path. Other users
    /// cannot connect, because the runtime root is private to the current
    /// user (mode 0700).
    pub fn bind(path: &Path) -> io::Result<Self> {
        let _ = std::fs::remove_file(path);
        UnixListener::bind(path).map(Self)
    }

    pub fn accept(&self) -> io::Result<LocalStream> {
        self.0.accept().map(|(stream, _)| stream)
    }
}
