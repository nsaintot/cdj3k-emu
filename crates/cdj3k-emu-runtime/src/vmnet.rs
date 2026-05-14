//! socket_vmnet integration for bridged Pro DJ Link networking.
//!
//! socket_vmnet (github.com/lima-vm/socket_vmnet) runs as root and exposes a
//! vmnet interface over a Unix socket.  QEMU connects with:
//!   -netdev stream,id=net0,server=off,addr.type=unix,addr.path=<sock>
//!
//! Elevation uses the macOS Security framework (AuthorizationCreate +
//! AuthorizationCopyRights + AuthorizationExecuteWithPrivileges), which shows
//! the native admin dialog with TouchID / Apple Watch support.

use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const BREW_CANDIDATES: &[&str] = &[
    "/opt/homebrew/bin/socket_vmnet",
    "/usr/local/bin/socket_vmnet",
    "/opt/homebrew/opt/socket_vmnet/bin/socket_vmnet",
];

/// A running socket_vmnet daemon for one QEMU instance.
///
/// Owns its daemon via a root-side watchdog spawned alongside socket_vmnet
/// (see `launch_elevated`).  The watchdog reaps the daemon when either:
///   - the cdj3k-emu PID exits (handles SIGKILL / crash / normal quit), or
///   - the socket file is unlinked (how `stop()` / `Drop` request shutdown).
///
/// We can't kill the daemon directly because it runs as root and the host
/// app runs as the user, but the user *can* unlink the socket (parent dir
/// is user-owned), which the watchdog uses as a shutdown signal.
pub struct SocketVmnet {
    socket_path: PathBuf,
    /// True when this handle started the daemon (and the watchdog).  False
    /// when we attached to a pre-existing daemon - in that case `Drop` must
    /// not unlink the socket out from under whoever else is using it.
    owns_daemon: bool,
}

/// Validate an interface name before letting it reach a root-elevated shell
/// template.  `sh_quote` already neutralises injection, but rejecting names
/// that don't match the BSD ifname grammar is a cheap defence-in-depth check
/// against ever feeding garbage to the elevated watcher.
pub(crate) fn is_valid_iface(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 16
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl SocketVmnet {
    /// Start socket_vmnet in bridged mode on `iface`, or reuse an already-running
    /// daemon for that interface.  The socket is shared across all instances.
    ///
    /// Shows the native macOS admin dialog only when a new daemon must be started.
    pub fn start_bridged(iface: &str) -> io::Result<Self> {
        if !is_valid_iface(iface) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid interface name: {iface:?}"),
            ));
        }
        cdj3k_emu_platform::runtime_paths::ensure_runtime_base_dir()?;
        let socket_path = socket_path_for(iface);

        // If the daemon is already live, reuse it - no elevation, no restart.
        // We didn't spawn it, so we don't own it: Drop won't unlink the socket.
        if socket_accepts(&socket_path) {
            return Ok(Self {
                socket_path,
                owns_daemon: false,
            });
        }

        // Stale socket file without a live daemon behind it.
        if socket_path.exists() {
            let _ = std::fs::remove_file(&socket_path);
        }

        let bin = find_binary()?;
        launch_elevated(&bin, iface, &socket_path)?;

        Ok(Self {
            socket_path,
            owns_daemon: true,
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Signal the root-side watchdog to reap socket_vmnet by unlinking the
    /// socket file.  Returns immediately; the daemon dies shortly after on
    /// the watchdog's next poll.  Safe to call multiple times.
    pub fn stop(&self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

impl Drop for SocketVmnet {
    fn drop(&mut self) {
        if self.owns_daemon {
            self.stop();
        }
    }
}

// ── Security.framework FFI ────────────────────────────────────────────────────

type AuthorizationRef = *mut libc::c_void;
type OSStatus = i32;
type AuthorizationFlags = u32;

const K_AUTH_FLAG_DEFAULTS: AuthorizationFlags = 0;
const K_AUTH_FLAG_INTERACTION_ALLOWED: AuthorizationFlags = 1 << 0;
const K_AUTH_FLAG_EXTEND_RIGHTS: AuthorizationFlags = 1 << 1;

#[repr(C)]
struct AuthorizationItem {
    name: *const libc::c_char,
    value_length: libc::size_t,
    value: *mut libc::c_void,
    flags: u32,
}

#[repr(C)]
struct AuthorizationItemSet {
    count: u32,
    items: *mut AuthorizationItem,
}

#[link(name = "Security", kind = "framework")]
extern "C" {
    fn AuthorizationCreate(
        rights: *const AuthorizationItemSet,
        environment: *const AuthorizationItemSet,
        flags: AuthorizationFlags,
        authorization: *mut AuthorizationRef,
    ) -> OSStatus;

    fn AuthorizationCopyRights(
        authorization: AuthorizationRef,
        rights: *const AuthorizationItemSet,
        environment: *const AuthorizationItemSet,
        flags: AuthorizationFlags,
        authorized_rights: *mut *mut AuthorizationItemSet,
    ) -> OSStatus;

    fn AuthorizationExecuteWithPrivileges(
        authorization: AuthorizationRef,
        path_to_tool: *const libc::c_char,
        options: AuthorizationFlags,
        arguments: *const *const libc::c_char,
        communication_pipe: *mut *mut libc::FILE,
    ) -> OSStatus;

    fn AuthorizationFree(authorization: AuthorizationRef, flags: AuthorizationFlags) -> OSStatus;
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn socket_path_for(iface: &str) -> PathBuf {
    cdj3k_emu_platform::runtime_paths::vmnet_sock(iface)
}

fn find_binary() -> io::Result<String> {
    if let Ok(exe) = std::env::current_exe() {
        let bundled = exe.parent().unwrap_or(Path::new(".")).join("socket_vmnet");
        if bundled.exists() {
            return Ok(bundled.to_string_lossy().into_owned());
        }
    }
    for c in BREW_CANDIDATES {
        if Path::new(c).exists() {
            return Ok(c.to_string());
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "socket_vmnet not found - run bundle.sh or install via Homebrew",
    ))
}

fn launch_elevated(bin: &str, iface: &str, socket_path: &Path) -> io::Result<()> {
    // The elevated shell does two things, both backgrounded:
    //   1. Spawn socket_vmnet itself.
    //   2. Spawn a watchdog subshell that polls the cdj3k-emu PID and the
    //      socket file presence; when either goes away it kills the daemon
    //      and unlinks the socket.  The watchdog inherits root from this
    //      elevated shell, so it actually has permission to SIGTERM the
    //      daemon - something the user-level host process never could.
    //
    // The watchdog also outlives this shell: `( ... ) &` forks a subshell,
    // and AuthorizationExecuteWithPrivileges spawns us without a controlling
    // tty, so there's no SIGHUP source.  `trap '' HUP` belt-and-suspenders.
    let bin_q = sh_quote(bin);
    let iface_q = sh_quote(iface);
    let sock_q = sh_quote(&socket_path.to_string_lossy());
    let parent_pid = std::process::id();
    let cmd = format!(
        r#"nohup {bin} --vmnet-mode bridged --vmnet-interface {iface} {sock} >/dev/null 2>&1 &
SV_PID=$!
( trap '' HUP
  i=0
  while [ $i -lt 40 ] && [ ! -S {sock} ]; do sleep 0.1; i=$((i+1)); done
  while kill -0 {ppid} 2>/dev/null && [ -S {sock} ]; do sleep 1; done
  kill "$SV_PID" 2>/dev/null
  sleep 0.3
  kill -9 "$SV_PID" 2>/dev/null
  rm -f {sock}
) </dev/null >/dev/null 2>&1 &
"#,
        bin = bin_q,
        iface = iface_q,
        sock = sock_q,
        ppid = parent_pid,
    );
    run_elevated(&cmd)?;

    // Poll until socket_vmnet is actually accepting connections (not just that the
    // socket file exists - a crashed process leaves the file behind).
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "socket_vmnet did not start within 5 s",
            ));
        }
        thread::sleep(Duration::from_millis(100));
        if socket_accepts(socket_path) {
            break;
        }
    }

    Ok(())
}

/// Try to connect to a Unix socket and immediately close.  Returns true if
/// something is actually listening - distinguishes a live daemon from a stale
/// socket file left behind by a crashed process.
fn socket_accepts(path: &std::path::Path) -> bool {
    use std::os::unix::net::UnixStream;
    UnixStream::connect(path).is_ok()
}

/// Run an arbitrary shell command as root via macOS Authorization Services.
/// Shows the native admin dialog (TouchID / Apple Watch eligible).
/// Shared by `vmnet` and `tapbridge`.
pub fn run_elevated(sh_cmd: &str) -> io::Result<()> {
    use std::ffi::CString;
    use std::ptr;

    let sh_path = CString::new("/bin/sh").unwrap();
    let sh_flag = CString::new("-c").unwrap();
    let cmd = CString::new(sh_cmd).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "shell command contains null byte",
        )
    })?;
    let right_name = CString::new("system.privilege.admin").unwrap();

    unsafe {
        let mut auth: AuthorizationRef = ptr::null_mut();
        let st = AuthorizationCreate(ptr::null(), ptr::null(), K_AUTH_FLAG_DEFAULTS, &mut auth);
        if st != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("AuthorizationCreate failed: {st}"),
            ));
        }

        let mut right_item = AuthorizationItem {
            name: right_name.as_ptr(),
            value_length: 0,
            value: ptr::null_mut(),
            flags: 0,
        };
        let rights = AuthorizationItemSet {
            count: 1,
            items: &mut right_item as *mut _,
        };
        let st = AuthorizationCopyRights(
            auth,
            &rights,
            ptr::null(),
            K_AUTH_FLAG_INTERACTION_ALLOWED | K_AUTH_FLAG_EXTEND_RIGHTS,
            ptr::null_mut(),
        );
        if st != 0 {
            AuthorizationFree(auth, K_AUTH_FLAG_DEFAULTS);
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "admin elevation cancelled or failed",
            ));
        }

        let args: [*const libc::c_char; 3] = [sh_flag.as_ptr(), cmd.as_ptr(), ptr::null()];
        let st = AuthorizationExecuteWithPrivileges(
            auth,
            sh_path.as_ptr(),
            K_AUTH_FLAG_DEFAULTS,
            args.as_ptr(),
            ptr::null_mut(),
        );
        AuthorizationFree(auth, K_AUTH_FLAG_DEFAULTS);

        if st != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("AuthorizationExecuteWithPrivileges failed: {st}"),
            ));
        }
    }
    Ok(())
}

/// Wrap a string in single quotes for /bin/sh, escaping any embedded `'`.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}
