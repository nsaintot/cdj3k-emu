//! The stream socket QEMU's chardevs listen on, as a path in the runtime
//! directory.

#[cfg(unix)]
#[path = "unix.rs"]
mod imp;
#[cfg(not(unix))]
#[path = "unsupported.rs"]
mod imp;

pub use imp::LocalStream;
