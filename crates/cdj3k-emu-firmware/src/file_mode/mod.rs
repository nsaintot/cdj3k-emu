//! A host file's mode bits, where the host's filesystem has them.

#[cfg(unix)]
#[path = "unix.rs"]
mod imp;
#[cfg(not(unix))]
#[path = "unsupported.rs"]
mod imp;

pub use imp::{permissions, set_executable};
