//! L2 TAP bridge for Pro DJ Link over an existing TAP interface (e.g. OpenVPN).
//!
//! # Architecture
//!
//! vmnet.framework cannot bridge to TAP interfaces, so we do it at the OS level:
//!
//!    host_tap (e.g. tap0)──┐
//!                          ├── macOS bridge (bridgeN) ── qemu_tap (tapM)  ── QEMU guest
//! (optional: other ifaces)─┘
//!
//! An elevated watcher script (run as root via Authorization Services) creates
//! the bridge and the QEMU-side TAP, then loops on a heartbeat file.  When
//! `TapBridge` is dropped, it removes the heartbeat; the watcher sees the file
//! gone, tears everything down, and exits.  This guarantees cleanup on normal
//! exit, panic, and SIGTERM.  The watcher also watches the app PID directly so
//! SIGKILL / Ctrl+C without a handler still triggers teardown.  Stale interfaces
//! from a previous unclean exit are detected and removed on the next launch.

use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{fs, thread};

use crate::vmnet::{run_elevated, sh_quote};
use cdj3k_emu_platform::runtime_paths;

fn tapbridge_base(instance_id: u32) -> String {
    runtime_paths::instance_dir(instance_id)
        .join("tapbridge")
        .to_string_lossy()
        .into_owned()
}

/// The fixed macOS bridge interface shared with djx-emu so both emulators' TAPs
/// land on the same L2 (DJ-Link). macOS bridges must be named `bridge<N>`;
/// override with `DJPL_BRIDGE` (set identically in both apps). Both apps
/// create-or-reuse this one bridge instead of an ephemeral `ifconfig bridge
/// create`.
fn djpl_bridge() -> String {
    std::env::var("DJPL_BRIDGE").unwrap_or_else(|_| "bridge99".to_string())
}

/// A live macOS bridge + QEMU TAP pair managed by an elevated watcher process.
/// Tearing down the bridge and TAP is automatic on drop.
pub struct TapBridge {
    /// The macOS bridge interface created for this session (e.g. "bridge3").
    pub bridge_iface: String,
    /// The TAP interface name (e.g. "tap1") - informational; QEMU uses `qemu_tap_fd`.
    pub qemu_tap: String,
    /// Open fd for `/dev/<qemu_tap>`.  Passed to QEMU as `tap,fd=N` so the
    /// interface is never DOWN between bridge setup and QEMU's first packet.
    pub qemu_tap_fd: RawFd,
    /// Keeps the tap interface RUNNING until this struct is dropped.
    _tap_file: fs::File,
    /// Removing this file signals the watcher to tear everything down.
    heartbeat: PathBuf,
}

impl TapBridge {
    /// Create a bridge between `host_tap` and a fresh QEMU-side TAP interface.
    ///
    /// Presents an admin elevation dialog (Authorization Services).
    /// Returns the `TapBridge` whose `qemu_tap_fd` should be passed to QEMU
    /// via `-netdev tap,fd=<qemu_tap_fd>`.
    pub fn setup(host_tap: &str, instance_id: u32) -> io::Result<Self> {
        if !crate::vmnet::is_valid_iface(host_tap) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid host tap name: {host_tap:?}"),
            ));
        }
        // Clean up any stale interfaces from a previous unclean exit first.
        cleanup_stale(instance_id);

        let app_pid = std::process::id();
        let base = tapbridge_base(instance_id);
        let heartbeat = PathBuf::from(format!("{}.alive", base));
        let names_file = PathBuf::from(format!("{}.names", base));
        let script_path = PathBuf::from(format!("{}.sh", base));

        // Remove any leftovers from a previous run.
        let _ = fs::remove_file(&heartbeat);
        let _ = fs::remove_file(&names_file);

        // Write the watcher script.
        let script =
            build_watcher_script(&heartbeat, host_tap, &names_file, app_pid, &djpl_bridge());
        fs::write(&script_path, &script).map_err(|e| {
            io::Error::new(e.kind(), format!("failed to write watcher script: {e}"))
        })?;

        // Touch the heartbeat before launching so the watcher never races.
        fs::write(&heartbeat, b"")
            .map_err(|e| io::Error::new(e.kind(), format!("failed to create heartbeat: {e}")))?;

        // Run the watcher script elevated and backgrounded.
        let cmd = format!(
            "nohup /bin/sh {} >/dev/null 2>&1 &",
            sh_quote(&script_path.to_string_lossy()),
        );
        run_elevated(&cmd)?;

        // Poll for the names file (watcher writes it after releasing the tap fd).
        let (bridge_iface, qemu_tap) = wait_for_names(&names_file, Duration::from_secs(8))?;

        // Open the tap device ourselves (watcher chmod'd it to 0666).
        // Holding this fd keeps the interface RUNNING in the bridge so there is
        // no DOWN/UP gap when QEMU starts; QEMU receives this fd directly via
        // `tap,fd=N` and never calls open() on /dev/tapN itself.
        let tap_dev = format!("/dev/{}", qemu_tap);
        let tap_file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&tap_dev)
            .map_err(|e| io::Error::new(e.kind(), format!("cannot open {tap_dev}: {e}")))?;
        let qemu_tap_fd = tap_file.as_raw_fd();
        // Clear FD_CLOEXEC so the fd survives the fork+exec into --qemu-worker.
        unsafe { libc::fcntl(qemu_tap_fd, libc::F_SETFD, 0) };

        eprintln!(
            "cdj3k-emu: tapbridge up  bridge={}  qemu_tap={}  fd={}  host_tap={}",
            bridge_iface, qemu_tap, qemu_tap_fd, host_tap
        );

        Ok(Self {
            bridge_iface,
            qemu_tap,
            qemu_tap_fd,
            _tap_file: tap_file,
            heartbeat,
        })
    }
}

impl Drop for TapBridge {
    fn drop(&mut self) {
        // Signal the watcher to tear down bridge + tap.
        let _ = fs::remove_file(&self.heartbeat);
        // Brief wait so the elevated watcher can finish before the process exits.
        thread::sleep(Duration::from_millis(600));
        eprintln!(
            "cdj3k-emu: tapbridge torn down  bridge={}  qemu_tap={}",
            self.bridge_iface, self.qemu_tap
        );
    }
}

// ── watcher script ────────────────────────────────────────────────────────────

fn build_watcher_script(
    heartbeat: &Path,
    host_tap: &str,
    names_file: &Path,
    app_pid: u32,
    bridge: &str,
) -> String {
    format!(
        r#"#!/bin/sh
set -e
HEARTBEAT={heartbeat}
HOST_TAP={host_tap}
NAMES_FILE={names_file}
APP_PID={app_pid}
BRIDGE={bridge}

# tuntap kext: tap devices only appear in ifconfig after /dev/tapN is opened.
# Claim the first free one by opening its device file on fd 3.
TAP_QEMU=""
for n in $(seq 0 15); do
    iface="tap$n"
    [ "$iface" = "$HOST_TAP" ] && continue
    [ -c "/dev/$iface" ] || continue
    # Skip if already claimed (RUNNING means another process has it open).
    flags=$(ifconfig "$iface" 2>/dev/null | head -1)
    case "$flags" in *RUNNING*) continue ;; esac
    if exec 3<>"/dev/$iface" 2>/dev/null; then
        TAP_QEMU="$iface"
        chmod 0666 "/dev/$iface" 2>/dev/null || true
        break
    fi
done
[ -z "$TAP_QEMU" ] && {{ echo "tapbridge: no free tap device found" >&2; exit 1; }}

# Create-or-reuse the SHARED fixed bridge (shared with djx-emu). If it already
# exists (another emulator made it), reuse it; otherwise create it by name.
ifconfig "$BRIDGE" >/dev/null 2>&1 || ifconfig "$BRIDGE" create >/dev/null 2>&1 \
    || {{ echo "tapbridge: cannot create $BRIDGE" >&2; exit 1; }}

# Add the host tap to the shared bridge once (skip if already a member).
if ! ifconfig "$BRIDGE" 2>/dev/null | grep -q "member: $HOST_TAP"; then
    ifconfig "$BRIDGE" addm "$HOST_TAP" 2>/dev/null || true
    ifconfig "$BRIDGE" -stp "$HOST_TAP" 2>/dev/null || true
fi
ifconfig "$BRIDGE" up               2>/dev/null || true

# Open tap1 briefly: creates the interface, brings it up, sets permissions.
# Then release so Rust can take exclusive hold and pass fd=N to QEMU.
# (tuntap kext is exclusive - only one process can hold the fd at a time.)
ifconfig "$TAP_QEMU" up             2>/dev/null || true
exec 3>&-

printf '%s:%s\n' "$BRIDGE" "$TAP_QEMU" > "$NAMES_FILE"

# Phase 1: Poll until Rust/QEMU has opened the tap (it will become RUNNING),
# then add it to the bridge with STP off.  The interface disappears when the
# watcher releases it above and reappears when Rust opens it - we must add
# the new instance, not the old one.
DEADLINE=$(( $(date +%s) + 10 ))
while [ -f "$HEARTBEAT" ] && kill -0 "$APP_PID" 2>/dev/null; do
    if ifconfig "$TAP_QEMU" 2>/dev/null | grep -q RUNNING; then
        ifconfig "$BRIDGE" addm "$TAP_QEMU" 2>/dev/null || true
        ifconfig "$BRIDGE" -stp "$TAP_QEMU" 2>/dev/null || true
        break
    fi
    [ $(date +%s) -ge $DEADLINE ] && {{ echo "tapbridge: $TAP_QEMU never became RUNNING" >&2; break; }}
    sleep 0.2
done

# Phase 2: Heartbeat loop.
while [ -f "$HEARTBEAT" ] && kill -0 "$APP_PID" 2>/dev/null; do
    sleep 0.5
done

# Tear down. The bridge is SHARED, so only remove OUR qemu tap; destroy the
# bridge itself only when no other emulator's tap remains on it (last one out).
chmod 0600 "/dev/$TAP_QEMU" 2>/dev/null || true
ifconfig "$BRIDGE"   deletem "$TAP_QEMU" 2>/dev/null || true
ifconfig "$TAP_QEMU" down    2>/dev/null || true
REMAIN=$(ifconfig "$BRIDGE" 2>/dev/null | awk '/member: tap/ {{print $2}}' | grep -v "^$HOST_TAP$" | grep -c . || true)
if [ "$REMAIN" = "0" ]; then
    ifconfig "$BRIDGE" deletem "$HOST_TAP" 2>/dev/null || true
    ifconfig "$BRIDGE" destroy 2>/dev/null || true
fi
rm -f "$NAMES_FILE" "$HEARTBEAT" {script_self}
"#,
        heartbeat = sh_quote(&heartbeat.to_string_lossy()),
        host_tap = sh_quote(host_tap),
        names_file = sh_quote(&names_file.to_string_lossy()),
        app_pid = app_pid,
        bridge = sh_quote(bridge),
        script_self = sh_quote(&heartbeat.with_extension("sh").to_string_lossy()),
    )
}

fn wait_for_names(names_file: &Path, timeout: Duration) -> io::Result<(String, String)> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(content) = fs::read_to_string(names_file) {
            let content = content.trim();
            if let Some((bridge, tap)) = content.split_once(':') {
                if !bridge.is_empty() && !tap.is_empty() {
                    return Ok((bridge.to_string(), tap.to_string()));
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "tapbridge watcher did not report interface names within {}s",
                    timeout.as_secs(),
                ),
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

// ── stale interface cleanup ───────────────────────────────────────────────────

/// On launch, check for stale bridge/tap pairs from a previous unclean exit
/// and destroy them via a single elevated call.
fn cleanup_stale(instance_id: u32) {
    let names_file = PathBuf::from(format!("{}.names", tapbridge_base(instance_id)));
    let Ok(content) = fs::read_to_string(&names_file) else {
        return;
    };
    let content = content.trim();
    let Some((bridge, tap)) = content.split_once(':') else {
        return;
    };
    if bridge.is_empty() || tap.is_empty() {
        return;
    }

    eprintln!(
        "cdj3k-emu: cleaning up stale tapbridge  bridge={}  tap={}",
        bridge, tap
    );

    let cmd = format!(
        "ifconfig {br} deletem {tap} 2>/dev/null; ifconfig {tap} destroy 2>/dev/null; rm -f {nf}",
        br = sh_quote(bridge),
        tap = sh_quote(tap),
        nf = sh_quote(&names_file.to_string_lossy()),
    );
    // Best-effort - ignore elevation cancellation.
    let _ = run_elevated(&cmd);
}
