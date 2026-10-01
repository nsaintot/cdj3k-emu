//! The stream socket QEMU's chardevs listen on, as a path in the runtime
//! directory.

#[cfg(unix)]
#[path = "unix.rs"]
mod imp;
#[cfg(windows)]
#[path = "windows.rs"]
mod imp;
#[cfg(not(any(unix, windows)))]
#[path = "unsupported.rs"]
mod imp;

pub use imp::LocalStream;
