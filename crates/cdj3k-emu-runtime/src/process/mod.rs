//! Signalling the QEMU child by pid, and what it inherits.

#[cfg(unix)]
#[path = "unix.rs"]
mod imp;
#[cfg(windows)]
#[path = "windows.rs"]
mod imp;
#[cfg(not(any(unix, windows)))]
#[path = "unsupported.rs"]
mod imp;

pub use imp::{adopt, is_alive, keep_across_exec, kill, set_quit_channel, terminate};

/// Remove this instance's runtime files on every way out of the process that
/// eframe's `on_exit` does not see, taking the QEMU child with it.
pub fn install_exit_cleanup(instance_dir: std::path::PathBuf) {
    let _ = crate::instance::SHUTDOWN_SOCK_DIR.set(instance_dir);
    imp::install_exit_hooks();
}
