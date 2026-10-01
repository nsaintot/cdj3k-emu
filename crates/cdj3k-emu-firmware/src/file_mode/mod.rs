//! Marking a file executable, where the host's filesystem has the bit.

#[cfg(unix)]
#[path = "unix.rs"]
mod imp;
#[cfg(not(unix))]
#[path = "unsupported.rs"]
mod imp;

pub use imp::set_executable;
