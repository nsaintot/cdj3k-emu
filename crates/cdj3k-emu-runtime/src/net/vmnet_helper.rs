//! vmnet through a root `socket_vmnet` daemon, for Macs where the entitlement
//! grants only part of vmnet.
//!
//! macOS 15 treats an entitled process that is not root as a limited client:
//! bridged mode fails with `cannot create vmnet interface`, and host-mode
//! interfaces get a MAC filter and the bridge's PRIVATE flag, so two instances
//! never reach each other. [`use_helper`] decides by starting QEMU with a
//! bridged netdev before the first slot uses vmnet. The first conclusive
//! answer holds for the rest of the process.
//!
//! The helper is socket_vmnet (github.com/lima-vm/socket_vmnet), bundled in
//! `Contents/MacOS`. It runs as root, opens one vmnet interface, and forwards
//! frames between every QEMU connected to its socket, so the instances share
//! one segment. Each bridged interface gets one daemon, `vmnet-br-<iface>`,
//! and each host-only network UUID gets one, `vmnet-host-<uuid>`.
//!
//! The socket lives in `/var/run/cdj3k-emu-<euid>/`, which the elevated script
//! creates root-owned, and is handed to this user with mode 0600. The leases
//! live in the user's runtime root, where root only reads them.

use std::io::{self, BufRead, BufReader, Read};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cdj3k_emu_platform::runtime_paths;

use crate::elevate::{run_elevated, sh_quote};

/// Set to `helper` or `inprocess` to skip the probe.
const ROUTE_ENV: &str = "CDJ3K_VMNET";

/// How long the probe waits for QEMU's QMP greeting.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// What QEMU prints when vmnet refuses the interface.
const REFUSED: &str = "cannot create vmnet interface";

/// What QEMU prints before the list of interfaces vmnet can bridge.
const OFFERED: &str = "expected one of [";

/// How long a new daemon must keep accepting before it counts as started.
/// socket_vmnet listens before it starts vmnet, and exits if vmnet fails.
const SETTLE: Duration = Duration::from_secs(1);

/// Homebrew paths searched in debug builds, which have no bundled
/// socket_vmnet. Release builds run only the bundled copy as root.
#[cfg(debug_assertions)]
const BREW_CANDIDATES: &[&str] = &[
    "/opt/homebrew/opt/socket_vmnet/bin/socket_vmnet",
    "/opt/homebrew/bin/socket_vmnet",
    "/usr/local/opt/socket_vmnet/bin/socket_vmnet",
    "/usr/local/bin/socket_vmnet",
];

/// The first conclusive probe answer in this process.
static USE_HELPER: Mutex<Option<bool>> = Mutex::new(None);

/// Whether vmnet has to go through the root helper. `iface` is the interface
/// to probe; `None` probes the default route's interface, or en0. When vmnet
/// does not offer that interface, the first one it does offer is probed.
/// `CDJ3K_VMNET` overrides the probe. An inconclusive probe answers `false`
/// and is not cached.
pub fn use_helper(iface: Option<&str>) -> bool {
    match std::env::var(ROUTE_ENV).as_deref() {
        Ok("helper") => return true,
        Ok("inprocess") => return false,
        _ => {}
    }
    let mut cached = USE_HELPER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(answer) = *cached {
        return answer;
    }
    let mut iface = iface
        .map(str::to_string)
        .or_else(default_route_iface)
        .unwrap_or_else(|| "en0".to_string());
    let mut answer = probe(&iface);
    if let Probe::Offered(other) = answer {
        iface = other;
        answer = probe(&iface);
    }
    let helper = match answer {
        Probe::Granted => false,
        Probe::Refused => true,
        Probe::Offered(_) | Probe::Unknown => return false,
    };
    eprintln!(
        "cdj3k-emu: vmnet {} this process bridged mode on {iface}.",
        if helper { "refuses" } else { "grants" }
    );
    *cached = Some(helper);
    helper
}

/// What one probe found.
#[derive(Debug, PartialEq, Eq)]
enum Probe {
    /// QMP came up: vmnet opened the interface in-process.
    Granted,
    /// vmnet refused the interface.
    Refused,
    /// vmnet does not offer the probed interface; this is the first it does.
    Offered(String),
    /// Anything else, such as a timeout.
    Unknown,
}

/// Start QEMU paused with only a bridged netdev on `iface`.
fn probe(iface: &str) -> Probe {
    let argv: Vec<String> = [
        "cdj3k-emu-qemu",
        "-M",
        "none",
        "-nodefaults",
        "-display",
        "none",
        "-monitor",
        "none",
        "-serial",
        "none",
        "-S",
        "-qmp",
        "stdio",
        "-netdev",
        &format!("vmnet-bridged,id=probe,ifname={iface}"),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let Ok(command) = crate::qemu_exec::qemu_command(None) else {
        return Probe::Unknown;
    };
    let mut command = command.with_argv(&argv);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let Ok(mut child) = command.spawn() else {
        return Probe::Unknown;
    };

    // The greeting is printed once every netdev is up; EOF means QEMU exited.
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Probe::Unknown;
    };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(line);
    });
    let greeted = rx.recv_timeout(PROBE_TIMEOUT);
    let _ = child.kill();
    let _ = child.wait();
    let Ok(line) = greeted else {
        eprintln!("cdj3k-emu: the vmnet probe on {iface} timed out.");
        return Probe::Unknown;
    };
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    let answer = classify(line.starts_with("{\"QMP\""), &stderr);
    if answer == Probe::Unknown {
        eprintln!(
            "cdj3k-emu: the vmnet probe on {iface} failed: {}",
            stderr.trim()
        );
    }
    answer
}

fn classify(greeted: bool, stderr: &str) -> Probe {
    if greeted {
        return Probe::Granted;
    }
    if stderr.contains(REFUSED) {
        return Probe::Refused;
    }
    stderr
        .split_once(OFFERED)
        .and_then(|(_, rest)| rest.split(']').next())
        .and_then(|list| list.split_whitespace().next())
        .filter(|name| cdj3k_emu_platform::net::is_valid_iface(name))
        .map_or(Probe::Unknown, |name| Probe::Offered(name.to_string()))
}

/// The interface of the default IPv4 route, from `route -n get default`.
fn default_route_iface() -> Option<String> {
    let out = std::process::Command::new("/sbin/route")
        .args(["-n", "get", "default"])
        .output()
        .ok()?;
    route_iface(&String::from_utf8_lossy(&out.stdout))
}

fn route_iface(route_output: &str) -> Option<String> {
    route_output
        .lines()
        .find_map(|l| l.trim().strip_prefix("interface:"))
        .map(|name| name.trim().to_string())
        .filter(|name| cdj3k_emu_platform::net::is_valid_iface(name))
}

/// One instance's hold on a shared socket_vmnet daemon.
///
/// The handle writes a lease, a file named `<pid>-<n>`, in the daemon's lease
/// directory; `n` distinguishes two handles in one process. A root watchdog
/// started with the daemon stops it once no lease names a live process, so a
/// crashed instance never keeps it running.
pub struct SocketVmnet {
    socket_path: PathBuf,
    lease: PathBuf,
}

impl SocketVmnet {
    /// Connect to the bridged daemon on `iface`, starting it if none runs.
    pub fn start_bridged(iface: &str) -> io::Result<Self> {
        if !cdj3k_emu_platform::net::is_valid_iface(iface) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid interface name: {iface:?}"),
            ));
        }
        Self::acquire(
            &format!("vmnet-br-{iface}"),
            &format!("--vmnet-mode=bridged --vmnet-interface={}", sh_quote(iface)),
        )
    }

    /// Connect to the host-only daemon for the network UUID, starting it if
    /// none runs. The UUID is the one in-process host mode uses, so the
    /// segment has no DHCP server.
    pub fn start_host() -> io::Result<Self> {
        let uuid = super::vmnet::host_network_id().to_ascii_lowercase();
        if !is_uuid(&uuid) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid network UUID: {uuid:?}"),
            ));
        }
        Self::acquire(
            &format!("vmnet-host-{uuid}"),
            &format!("--vmnet-mode=host --vmnet-network-identifier={uuid}"),
        )
    }

    /// Write the lease, then reuse a live daemon or start one. The lease goes
    /// first, so a starting watchdog already counts this instance. A lock on
    /// the runtime root makes the check and the start one step across app
    /// processes, so two slots never start two daemons.
    fn acquire(name: &str, mode_args: &str) -> io::Result<Self> {
        let runtime = runtime_paths::ensure_runtime_base_dir()?;
        let _lock = lock(&runtime.join("vmnet.lock"))?;
        let socket_path = socket_dir().join(format!("{name}.sock"));
        let leases = runtime.join(format!("{name}.clients"));
        std::fs::create_dir_all(&leases)?;
        prune_dead_leases(&leases);
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let lease = leases.join(format!("{}-{n}", std::process::id()));
        std::fs::write(&lease, "")?;
        let this = Self { socket_path, lease };

        if !socket_accepts(&this.socket_path) {
            launch_elevated(&find_binary()?, mode_args, &this.socket_path, &leases)?;
        }
        Ok(this)
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }
}

impl Drop for SocketVmnet {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.lease);
        if let Some(dir) = self.lease.parent() {
            // Fails while another instance still holds a lease.
            let _ = std::fs::remove_dir(dir);
        }
    }
}

/// `/var/run/cdj3k-emu-<euid>`, root-owned, so no user process can replace
/// the socket or what root does to it.
fn socket_dir() -> PathBuf {
    // SAFETY: `geteuid()` has no preconditions and cannot fail.
    PathBuf::from(format!("/var/run/cdj3k-emu-{}", unsafe { libc::geteuid() }))
}

/// Take an exclusive lock on `path`, waiting for it. Dropping the file
/// releases it.
fn lock(path: &Path) -> io::Result<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    // SAFETY: the fd is open for as long as `file` lives.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(file)
}

/// Remove the leases of processes that have exited.
fn prune_dead_leases(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for path in entries.flatten().map(|e| e.path()) {
        if !lease_pid(&path).is_some_and(crate::process::is_alive) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn lease_pid(path: &Path) -> Option<i32> {
    path.file_name()?.to_str()?.split('-').next()?.parse().ok()
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

fn find_binary() -> io::Result<PathBuf> {
    let bundled = cdj3k_emu_platform::bundled::tool("socket_vmnet");
    if bundled.is_absolute() {
        return Ok(bundled);
    }
    #[cfg(debug_assertions)]
    if let Some(found) = BREW_CANDIDATES
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
    {
        return Ok(found);
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "socket_vmnet was not found in the app bundle",
    ))
}

/// Start socket_vmnet as root with a watchdog beside it, then wait until the
/// socket accepts connections and still does [`SETTLE`] later.
fn launch_elevated(
    bin: &Path,
    mode_args: &str,
    socket_path: &Path,
    leases: &Path,
) -> io::Result<()> {
    let dir = socket_path.parent().unwrap_or(Path::new("/var/run"));
    // SAFETY: `geteuid()` has no preconditions and cannot fail.
    let uid = unsafe { libc::geteuid() };
    run_elevated(&watchdog_script(
        &sh_quote(&bin.to_string_lossy()),
        mode_args,
        &sh_quote(&dir.to_string_lossy()),
        &sh_quote(&socket_path.to_string_lossy()),
        &sh_quote(&leases.to_string_lossy()),
        uid,
    ))?;

    let deadline = Instant::now() + Duration::from_secs(5);
    while !socket_accepts(socket_path) {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "socket_vmnet did not start within 5 s",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(SETTLE);
    if !socket_accepts(socket_path) {
        return Err(io::Error::other(
            "socket_vmnet exited after it started; vmnet refused the interface",
        ));
    }
    Ok(())
}

/// The elevated shell script. Every argument is already quoted.
///
/// It creates the socket directory root-owned, starts the daemon, and gives
/// the socket to `uid` with mode 0600. The watchdog runs while the daemon
/// lives. Once a second it looks for a lease whose PID is alive; root only
/// reads the lease directory. With no live lease it removes the socket, so no
/// new client finds it, then stops the daemon.
fn watchdog_script(
    bin: &str,
    mode_args: &str,
    dir: &str,
    sock: &str,
    leases: &str,
    uid: u32,
) -> String {
    format!(
        r#"mkdir -p {dir} && chown root:wheel {dir} && chmod 0755 {dir} || exit 1
rm -f {sock}
nohup {bin} --socket-group=wheel {mode_args} {sock} </dev/null >/dev/null 2>&1 &
SV_PID=$!
( trap '' HUP
  i=0
  while [ $i -lt 50 ] && [ ! -S {sock} ]; do sleep 0.1; i=$((i+1)); done
  chown {uid} {sock}
  chmod 0600 {sock}
  while kill -0 "$SV_PID" 2>/dev/null; do
    live=0
    for l in {leases}/*; do
      pid=${{l##*/}}; pid=${{pid%%-*}}
      case "$pid" in ''|*[!0-9]*) continue ;; esac
      if kill -0 "$pid" 2>/dev/null; then live=1; break; fi
    done
    if [ "$live" -eq 0 ]; then
      rm -f {sock}
      kill "$SV_PID" 2>/dev/null
      sleep 0.3
      kill -9 "$SV_PID" 2>/dev/null
      break
    fi
    sleep 1
  done
) </dev/null >/dev/null 2>&1 &
"#
    )
}

/// Whether a daemon listens on `path`. A daemon that exited leaves the socket
/// file behind with nothing accepting on it.
fn socket_accepts(path: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_selects_the_helper_and_an_offer_names_the_next_probe() {
        assert_eq!(classify(true, ""), Probe::Granted);
        assert_eq!(
            classify(
                false,
                "-netdev vmnet-bridged,id=probe,ifname=en0: cannot create vmnet interface: unknown vmnet error"
            ),
            Probe::Refused
        );
        assert_eq!(
            classify(
                false,
                "-netdev vmnet-bridged,id=probe,ifname=utun3: unsupported ifname 'utun3', expected one of [ en0 en7 ]"
            ),
            Probe::Offered("en0".into())
        );
        assert_eq!(
            classify(false, "unsupported ifname 'en5', expected one of [ ]"),
            Probe::Unknown
        );
    }

    #[test]
    fn the_default_route_names_its_interface() {
        let out = "   route to: default\ndestination: default\n    gateway: 10.10.50.1\n  interface: en7\n      flags: <UP,GATEWAY,DONE,STATIC,PRCLONING>\n";
        assert_eq!(route_iface(out).as_deref(), Some("en7"));
        assert_eq!(
            route_iface("route: writing to routing socket: not in table\n"),
            None
        );
    }

    #[test]
    fn a_lease_is_named_by_its_pid() {
        assert_eq!(lease_pid(Path::new("/tmp/x.clients/4356-0")), Some(4356));
        assert_eq!(lease_pid(Path::new("/tmp/x.clients/passwd")), None);
    }

    #[test]
    fn only_a_uuid_names_a_host_socket() {
        assert!(is_uuid("7b3d5e2a-9c14-4f6b-b2e1-0a1c2d3e4f50"));
        assert!(!is_uuid("../../etc/passwd-0000-0000-000000000000"));
        assert!(!is_uuid("7b3d5e2a9c144f6bb2e10a1c2d3e4f50"));
    }

    /// Root only reads the lease directory, and gives the socket to the user.
    #[test]
    fn root_deletes_nothing_in_the_lease_directory() {
        let s = watchdog_script("'/b'", "--vmnet-mode=host", "'/d'", "'/d/s'", "'/l'", 501);
        assert!(s.contains("chown 501 '/d/s'"), "{s}");
        assert!(s.contains("--socket-group=wheel"), "{s}");
        assert!(!s.contains("rm -f \"$l\""), "{s}");
        assert!(!s.contains("rm -rf"), "{s}");
    }
}
