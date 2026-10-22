//! The layout of the host's runtime and socket directory, defined in one
//! place.
//!
//! Each host names its own root, and every root keeps the layout below.
//!
//! ```text
//! <root>/instance-{id}/
//!   {cfg,ctrl,usb-link,midi-driver}.sock   per-stream UNIX sockets
//!   window.sock                            commands from other windows
//!   main.shm  jog.shm                      guest framebuffer and jog LCD
//!   ram.shm                                guest RAM, hosts without /dev/shm
//!   usb.placeholder                        always-present USB slot backing
//!   serial.log | serial.sock               QEMU serial
//!   tapbridge.* | linuxnet.*               bridge marker files
//! <root>/vmnet.lock                        macOS vmnet helper start lock
//! <root>/vmnet-{br-iface|host-uuid}.clients/  its leases, one per slot
//! ```
//!
//! The root is per-user on every host, so no other user can create or replace
//! state under it: under `/tmp` it takes an euid suffix and mode 0700, and
//! `%TEMP%` is per-user by ACL. A killed process leaves its `instance-{id}`
//! behind; the slot claim (`cdj3k_emu_storage::slot_in_use`) decides whether
//! a slot is open.
//!
//! Socket paths stay under the host's UNIX socket path limit, at which C
//! truncates silently. [`SUN_PATH_LIMIT`] and its test enforce it;
//! `tools/midi-driver/link.c` reimplements this layout in C without the check.

#[cfg(unix)]
#[path = "unix.rs"]
mod imp;
#[cfg(windows)]
#[path = "windows.rs"]
mod imp;
#[cfg(not(any(unix, windows)))]
#[path = "unsupported.rs"]
mod imp;

use std::io;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Bytes a socket path may occupy on this host, including the NUL.
///
/// macOS limits `sun_path` to 104 bytes and Linux to 108; the Unix arm uses 104
/// for both. A longer path fails or is truncated, depending on the API.
/// Windows' AF_UNIX allows 108, and a host without AF_UNIX places no limit.
pub use imp::SUN_PATH_LIMIT;

fn cached_base_dir() -> &'static PathBuf {
    static BASE: OnceLock<PathBuf> = OnceLock::new();
    BASE.get_or_init(imp::base_dir)
}

/// This host's runtime root, such as `/tmp/cdj3k-emu-501`.
/// [`ensure_runtime_base_dir`] creates it.
pub fn runtime_base_dir() -> PathBuf {
    cached_base_dir().clone()
}

/// `<root>/instance-{id}`, the slot's directory for sockets and state.
pub fn instance_dir(id: u32) -> PathBuf {
    runtime_base_dir().join(format!("instance-{id}"))
}

/// Return `<root>/instance-{id}/window.sock`, where the window that holds slot
/// `id` listens for commands from other windows ([`crate::window_socket`]).
pub fn window_sock_path(id: u32) -> PathBuf {
    instance_dir(id).join("window.sock")
}

/// Ensure that the runtime root exists with the isolation this host needs. It
/// is idempotent. Call it once at startup, before placing any sockets, shm
/// files or marker files inside.
///
/// This is the only place that creates the root, so a caller cannot skip the
/// host's isolation step by making the directory itself.
pub fn ensure_runtime_base_dir() -> io::Result<PathBuf> {
    let base = runtime_base_dir();
    std::fs::create_dir_all(&base)?;
    imp::tighten(&base)?;
    Ok(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The longest path the layout can produce, `midi-driver.sock` in the
    /// directory of the widest instance id, fits under `SUN_PATH_LIMIT`.
    #[test]
    fn a_socket_path_leaves_headroom_under_the_host_limit() {
        let longest = instance_dir(u32::MAX).join("midi-driver.sock");
        let bytes = longest.to_string_lossy().len();
        assert!(
            bytes < SUN_PATH_LIMIT,
            "{bytes} bytes does not fit {SUN_PATH_LIMIT}: {longest:?}"
        );
    }

    /// `instance_dir` keeps the leaf name that the MIDI driver build expects.
    #[test]
    fn the_instance_dir_is_named_by_slot() {
        assert_eq!(
            instance_dir(3).file_name().unwrap().to_string_lossy(),
            "instance-3"
        );
        assert_eq!(instance_dir(3).parent(), Some(runtime_base_dir().as_path()));
    }
}
