//! An AF_UNIX stream socket (Windows 10 1803 and later). On Windows, as on
//! every other host, QEMU's `-chardev socket,path=` listens on one, and so
//! does each window ([`LocalListener`]).

use std::io;
use std::path::Path;

pub use uds_windows::UnixStream as LocalStream;

pub struct LocalListener(uds_windows::UnixListener);

impl LocalListener {
    /// Listen at `path`, removing any file already at that path. Other users
    /// cannot connect, because the runtime root under `%TEMP%` is private to
    /// the current user.
    pub fn bind(path: &Path) -> io::Result<Self> {
        let _ = std::fs::remove_file(path);
        uds_windows::UnixListener::bind(path).map(Self)
    }

    pub fn accept(&self) -> io::Result<LocalStream> {
        self.0.accept().map(|(stream, _)| stream)
    }
}
