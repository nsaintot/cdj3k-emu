//! Handing QEMU an open file through its monitor (`add-fd`), which only a
//! Unix socket can carry.

#[cfg(unix)]
#[path = "unix.rs"]
mod imp;
#[cfg(not(unix))]
#[path = "unsupported.rs"]
mod imp;

pub use imp::add_fd;
