//! An AF_UNIX stream socket (Windows 10 1803 and later), which is what
//! QEMU's `-chardev socket,path=` listens on here as on every other host.

pub use uds_windows::UnixStream as LocalStream;
