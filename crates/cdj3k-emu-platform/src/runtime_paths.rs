//! Single source of truth for the host runtime / socket directory layout.
//!
//! Layout (where `<UID>` is the current effective UID):
//! ```text
//! /tmp/cdj3k-emu-<UID>/                       (RUNTIME_BASE_DIR, mode 0700)
//!   instance-{id}/                            (instance_dir)
//!     {cfg,ctrl,main,jog,sub,led}.sock        (per-stream UNIX sockets)
//!     ram.shm                                 (shared-mem guest RAM)
//!     serial.log                              (QEMU serial log)
//!     tapbridge                               (tap-bridge marker)
//!   vmnet-{iface}.sock                        (per-iface socket_vmnet sockets)
//! ```
//!
//! Per-UID suffix + mode 0700 prevents squatting on a shared host: only the
//! invoking user can create or replace state under here.  The path stays
//! short enough (<25 bytes before the instance subdir) to leave headroom
//! under the macOS UNIX-socket path limit (~104 bytes).
//!
//! Mirrors the path convention used by `boot.sh` (CDJ3K_SOCK_DIR).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn current_uid() -> u32 {
    // SAFETY: `geteuid()` has no preconditions and cannot fail.
    unsafe { libc::geteuid() as u32 }
}

fn cached_base_dir() -> &'static PathBuf {
    static BASE: OnceLock<PathBuf> = OnceLock::new();
    BASE.get_or_init(|| PathBuf::from(format!("/tmp/cdj3k-emu-{}", current_uid())))
}

/// `/tmp/cdj3k-emu-<UID>`.  Created on demand by [`ensure_runtime_base_dir`].
pub fn runtime_base_dir() -> PathBuf {
    cached_base_dir().clone()
}

/// `/tmp/cdj3k-emu-<UID>/instance-{id}` — per-instance socket + state directory.
pub fn instance_dir(id: u32) -> PathBuf {
    runtime_base_dir().join(format!("instance-{id}"))
}

/// Shared **DJPL** runtime socket dir (`/tmp/djpl-<UID>`, mode 0700). A
/// product-NEUTRAL namespace shared with djx-emu so both emulators connect to
/// the *same* socket_vmnet daemon → one DJ-Link L2 (players ↔ mixer see each
/// other). Override with `DJPL_NET_DIR` (set identically in both apps). Distinct
/// from [`runtime_base_dir`] (which stays per-app for instance/eMMC state).
pub fn djpl_net_dir() -> PathBuf {
    if let Ok(d) = std::env::var("DJPL_NET_DIR") {
        return PathBuf::from(d);
    }
    PathBuf::from(format!("/tmp/djpl-{}", current_uid()))
}

/// `/tmp/djpl-<UID>/vmnet-{iface}.sock` — socket_vmnet daemon socket for host
/// iface `iface`, shared across every emulator bound to that iface (the naming
/// MUST match djx-emu's `net_vmnet`).
pub fn vmnet_sock(iface: &str) -> PathBuf {
    djpl_net_dir().join(format!("vmnet-{iface}.sock"))
}

/// Ensure the shared DJPL net dir exists with mode 0700. Idempotent.
pub fn ensure_djpl_net_dir() -> io::Result<PathBuf> {
    let base = djpl_net_dir();
    std::fs::create_dir_all(&base)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(base)
}

/// Helper for callers that already have an `&Path` to an instance dir and
/// want to attach a known per-stream socket basename.
pub fn join_sock(instance: &Path, basename: &str) -> PathBuf {
    instance.join(basename)
}

/// Ensure the runtime base dir exists with mode 0700.  Idempotent.
/// Call once at startup (e.g. from `vmnet::start_*`) before placing any
/// sockets, shm files, or marker files inside.
pub fn ensure_runtime_base_dir() -> io::Result<PathBuf> {
    let base = runtime_base_dir();
    std::fs::create_dir_all(&base)?;
    // Tighten perms even if the dir was created by an older build with the
    // default umask.  No-op when already 0700.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o700);
        std::fs::set_permissions(&base, perms)?;
    }
    Ok(base)
}
