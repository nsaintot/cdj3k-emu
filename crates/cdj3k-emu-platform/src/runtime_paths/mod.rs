//! Single source of truth for the host runtime / socket directory layout.
//!
//! Each host names its own root; the shape below is what every root must
//! preserve.
//!
//! ```text
//! <root>/instance-{id}/
//!   {cfg,ctrl,usb-link,midi-driver}.sock   per-stream UNIX sockets
//!   window.sock                            commands from other windows
//!   main.shm  jog.shm                      guest framebuffer + jog LCD
//!   ram.shm                                guest RAM, hosts without /dev/shm
//!   usb.placeholder                        always-present USB slot backing
//!   serial.log | serial.sock               QEMU serial
//!   tapbridge.* | linuxnet.*               bridge marker files
//! ```
//!
//! The root is per-user on every host, so no one else can create or replace
//! state under it: `/tmp` takes an euid suffix and mode 0700, `%TEMP%` is
//! per-user by ACL. A killed process leaves its `instance-{id}`
//! behind; whether a slot is open is the slot claim's to say
//! (`cdj3k_emu_storage::slot_in_use`).
//!
//! Socket paths stay under the host's UNIX-socket path limit, which C
//! truncates at silently. [`SUN_PATH_LIMIT`] and its test enforce it;
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
/// macOS and Linux both enforce the BSD `sun_path` of 104 for the *string*, and
/// exceed it by failing or truncating depending on who is asking. Windows'
/// AF_UNIX allows 108. A host with no AF_UNIX at all places no limit.
pub use imp::SUN_PATH_LIMIT;

fn cached_base_dir() -> &'static PathBuf {
    static BASE: OnceLock<PathBuf> = OnceLock::new();
    BASE.get_or_init(imp::base_dir)
}

/// This host's runtime root, e.g. `/tmp/cdj3k-emu-501`. Created on demand by
/// [`ensure_runtime_base_dir`].
pub fn runtime_base_dir() -> PathBuf {
    cached_base_dir().clone()
}

/// `<root>/instance-{id}` — per-instance socket + state directory.
pub fn instance_dir(id: u32) -> PathBuf {
    runtime_base_dir().join(format!("instance-{id}"))
}

/// Return `<root>/instance-{id}/window.sock`, where the window that holds slot
/// `id` listens for commands from other windows ([`crate::window_socket`]).
pub fn window_sock_path(id: u32) -> PathBuf {
    instance_dir(id).join("window.sock")
}

/// Ensure the runtime root exists and carries whatever isolation this host
/// needs. Idempotent. Call once at startup before placing any sockets, shm
/// files, or marker files inside.
///
/// This is the only place the root is created, so that the host's isolation
/// step cannot be skipped by a caller that made the directory itself.
pub fn ensure_runtime_base_dir() -> io::Result<PathBuf> {
    let base = runtime_base_dir();
    std::fs::create_dir_all(&base)?;
    imp::tighten(&base)?;
    Ok(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The longest socket basename any host writes into an instance dir, and
    /// the widest instance id: the worst case the layout can produce.
    #[test]
    fn a_socket_path_leaves_headroom_under_the_host_limit() {
        let longest = instance_dir(u32::MAX).join("midi-driver.sock");
        let bytes = longest.to_string_lossy().len();
        assert!(
            bytes < SUN_PATH_LIMIT,
            "{bytes} bytes does not fit {SUN_PATH_LIMIT}: {longest:?}"
        );
    }

    /// `instance_dir` keeps the leaf name `boot.sh` and the MIDI driver build.
    #[test]
    fn the_instance_dir_is_named_by_slot() {
        assert_eq!(
            instance_dir(3).file_name().unwrap().to_string_lossy(),
            "instance-3"
        );
        assert_eq!(instance_dir(3).parent(), Some(runtime_base_dir().as_path()));
    }
}
