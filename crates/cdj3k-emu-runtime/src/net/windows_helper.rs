//! The elevated half of Windows bridging. The app re-runs itself with
//! [`HELPER_FLAG`] through `run_elevated`, and that copy:
//!
//! 1. makes the slot's TAP-Windows6 adapter when there is none (SetupAPI, the
//!    steps `devcon install` takes, with the driver bound to that device
//!    alone) and names its connection;
//! 2. joins the tap and the NIC to the host's Network Bridge, creating it when
//!    there is none and recording that it did. Windows has at most one, so an
//!    existing bridge is joined, never replaced;
//! 3. starts a copy of itself with [`WATCH_FLAG`], which stays elevated and
//!    takes the link down when the slot lets it go.
//!
//! The verdict goes to [`RESULT_FILE`] in the request's directory.
//!
//! The watcher waits for the app to exit or for its lease ([`super::lease`])
//! to go. It then takes the tap out of the bridge, or destroys the bridge
//! when it was the app's and no other slot's tap is in it, writes the lease's
//! answer, and removes the tap adapter once QEMU has let go of it. Setup and
//! teardown hold one named mutex, so slots starting and stopping together see
//! each other's links.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{fs, thread};

use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    CreateMutexW, OpenProcess, ReleaseMutex, WaitForSingleObject, PROCESS_SYNCHRONIZE,
};

use super::lease::{CLAIM_FILE, RELEASED_FILE};
use super::winbridge::{self, Outcome, Request, HELPER_FLAG, RESULT_FILE, WATCH_FLAG};
use super::windows_tap::{
    configure_tap, create_tap, is_connected, remove_tap, set_alias, taps_named, wait_openable,
};

/// How often the watcher looks at the app and the claim.
const WATCH_POLL: Duration = Duration::from_millis(500);
/// How long the watcher waits for QEMU to close the tap before it leaves the
/// adapter in place.
const RELEASE_WAIT: Duration = Duration::from_secs(120);

/// When `args` ask for the helper or the watcher, do its work and exit this
/// process; otherwise return.
pub fn run_helper_if_asked(args: &[String]) {
    let flag = args.get(1).map(String::as_str);
    if flag != Some(HELPER_FLAG) && flag != Some(WATCH_FLAG) {
        return;
    }
    let Some(request) = Request::parse(&args[2..]) else {
        eprintln!(
            "cdj3k-emu: {}: the request is malformed.",
            flag.unwrap_or_default()
        );
        std::process::exit(2);
    };
    if flag == Some(WATCH_FLAG) {
        watch(&request);
        std::process::exit(0);
    }
    let outcome = {
        let _held = BridgeLock::take();
        run(&request)
    };
    let wrote = fs::write(request.dir.join(RESULT_FILE), outcome.encode());
    std::process::exit(if wrote.is_ok() && matches!(outcome, Outcome::Done) {
        0
    } else {
        1
    });
}

/// The file that records the bridge the app made: its GUID.
fn owned_bridge_file() -> PathBuf {
    let base = std::env::var_os("ProgramData")
        .map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from);
    base.join("cdj3k-emu").join("bridge-owned")
}

fn run(req: &Request) -> Outcome {
    match set_up(req) {
        Ok(()) => Outcome::Done,
        Err(outcome) => outcome,
    }
}

/// Bring the slot's link up: its tap, configured and named, in the host's
/// bridge beside the NIC, and a watcher holding it.
fn set_up(req: &Request) -> Result<(), Outcome> {
    let guid = slot_tap(req)?;
    prepare_tap(req, &guid)?;
    let bridge = host_bridge(req)?;
    join_bridge(req, &bridge)?;
    if !wait_openable(&guid) {
        return Err(Outcome::TapFailed(format!(
            "{} never became openable",
            req.tap
        )));
    }
    start_watcher(req)
}

/// The interface GUID of the slot's tap adapter, made when there is none.
fn slot_tap(req: &Request) -> Result<String, Outcome> {
    let mut named = taps_named(&req.tap);
    // Remove the disconnected copies that an earlier failed setup left under
    // the slot's name.
    for extra in named.iter().skip(1) {
        if !is_connected(extra) {
            let _ = remove_tap(extra);
        }
    }
    named.truncate(1);
    if let Some(guid) = named.pop() {
        return Ok(guid);
    }
    let Some(inf) = &req.inf else {
        return Err(Outcome::TapFailed(
            "the TAP-Windows6 driver is not installed".into(),
        ));
    };
    create_tap(inf, &req.tap).map_err(|e| Outcome::TapFailed(e.to_string()))
}

/// Give the tap its parameters and the slot's name.
fn prepare_tap(req: &Request, guid: &str) -> Result<(), Outcome> {
    let Some(mac) = winbridge::tap_mac(&req.mac) else {
        return Err(Outcome::TapFailed(format!(
            "cannot derive a tap MAC from {}",
            req.mac
        )));
    };
    configure_tap(guid, &mac)
        .map_err(|e| Outcome::TapFailed(format!("could not configure {}: {e}", req.tap)))?;
    // `netsh bridge` names adapters by their alias.
    if !set_alias(guid, &req.tap) {
        return Err(Outcome::TapFailed(format!(
            "the adapter {guid} could not be named {}",
            req.tap
        )));
    }
    Ok(())
}

/// The GUID of the host's Network Bridge, created from the tap and the NIC
/// when there is none.
fn host_bridge(req: &Request) -> Result<String, Outcome> {
    if let Some(bridge) = first_bridge() {
        return Ok(bridge);
    }
    // The bridge takes the MAC of the adapter it is created with:
    // the tap's, fixed per slot ([`winbridge::tap_mac`]).
    netsh(&["bridge", "create", &req.tap, &req.nic]).map_err(Outcome::BridgeFailed)?;
    let Some(bridge) = first_bridge() else {
        return Err(Outcome::BridgeFailed("netsh made no bridge".into()));
    };
    record_owned_bridge(&bridge);
    Ok(bridge)
}

/// The first bridge `netsh` lists.
fn first_bridge() -> Option<String> {
    winbridge::guids(&netsh(&["bridge", "list"]).unwrap_or_default())
        .into_iter()
        .next()
}

/// Record `bridge` as the app's in [`owned_bridge_file`].
fn record_owned_bridge(bridge: &str) {
    let owned = owned_bridge_file();
    let recorded = owned
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::write(&owned, bridge));
    if let Err(e) = recorded {
        eprintln!(
            "cdj3k-emu: could not record the bridge in {}: {e}",
            owned.display()
        );
    }
}

/// Add the tap and the NIC to `bridge`, and check both are in it.
fn join_bridge(req: &Request, bridge: &str) -> Result<(), Outcome> {
    // `create` reports success with only its first adapter joined; `add`
    // joins the rest.
    for name in [&req.tap, &req.nic] {
        let shown = netsh(&["bridge", "show", "adapter"]).unwrap_or_default();
        if winbridge::is_bridged(&shown, name) != Some(true) {
            netsh(&["bridge", "add", name, "to", bridge]).map_err(Outcome::BridgeFailed)?;
        }
    }
    let shown = netsh(&["bridge", "show", "adapter"]).unwrap_or_default();
    for name in [&req.tap, &req.nic] {
        if winbridge::is_bridged(&shown, name) != Some(true) {
            return Err(Outcome::BridgeFailed(format!(
                "{name} did not join the bridge:\n{shown}"
            )));
        }
    }
    Ok(())
}

/// Start the elevated copy of the app that takes the link down
/// ([`WATCH_FLAG`]).
fn start_watcher(req: &Request) -> Result<(), Outcome> {
    let exe = std::env::current_exe().map_err(|e| {
        Outcome::BridgeFailed(format!("cannot find the app to watch the link: {e}"))
    })?;
    let watcher = Request {
        inf: None,
        ..req.clone()
    };
    cdj3k_emu_platform::child::command(&exe)
        .arg(WATCH_FLAG)
        .args(watcher.args())
        .spawn()
        .map_err(|e| Outcome::BridgeFailed(format!("cannot start the link's watcher: {e}")))?;
    Ok(())
}

/// Wait for the slot to let its link go, then take it down.
fn watch(req: &Request) {
    let claim_path = req.dir.join(CLAIM_FILE);
    let claimed = || fs::read_to_string(&claim_path).is_ok_and(|c| c.trim() == req.claim);
    // SAFETY: the handle is closed below; a failed open reads as an app that
    // has already gone.
    let app = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, req.pid) }.ok();
    let app_alive = || {
        app.is_some_and(
            |h| unsafe { WaitForSingleObject(h, WATCH_POLL.as_millis() as u32) } != WAIT_OBJECT_0,
        )
    };
    while claimed() && app_alive() {}
    if let Some(h) = app {
        // SAFETY: opened above, closed once.
        let _ = unsafe { CloseHandle(h) };
    }

    {
        let _held = BridgeLock::take();
        if !reclaimed(&claim_path, &req.claim) {
            take_down(&req.tap);
        }
        let _ = fs::write(req.dir.join(RELEASED_FILE), &req.claim);
    }

    let deadline = Instant::now() + RELEASE_WAIT;
    while Instant::now() < deadline {
        thread::sleep(Duration::from_secs(1));
        let _held = BridgeLock::take();
        if reclaimed(&claim_path, &req.claim) {
            return;
        }
        let Some(guid) = taps_named(&req.tap).into_iter().next() else {
            return;
        };
        if !is_connected(&guid) {
            if let Err(e) = remove_tap(&guid) {
                eprintln!("cdj3k-emu: could not remove {}: {e}", req.tap);
            }
            return;
        }
    }
}

/// Whether a later setup claimed the slot's link under another claim.
fn reclaimed(claim_path: &Path, ours: &str) -> bool {
    fs::read_to_string(claim_path).is_ok_and(|c| c.trim() != ours)
}

/// Take `tap` out of the bridge, or destroy the bridge when it was the app's
/// and no other slot's tap is in it.
fn take_down(tap: &str) {
    let shown = netsh(&["bridge", "show", "adapter"]).unwrap_or_default();
    let list = netsh(&["bridge", "list"]).unwrap_or_default();
    let owned_path = owned_bridge_file();
    let owned = fs::read_to_string(&owned_path).ok();
    let plan = winbridge::teardown(&shown, &list, owned.as_deref(), tap);
    if let Some(bridge) = &plan.remove_from {
        if let Err(e) = netsh(&["bridge", "remove", tap, "from", bridge]) {
            eprintln!("cdj3k-emu: could not take {tap} out of the bridge: {e}");
        }
    }
    if let Some(bridge) = &plan.destroy {
        match netsh(&["bridge", "destroy", bridge]) {
            Ok(_) => {
                let _ = fs::remove_file(&owned_path);
            }
            Err(e) => eprintln!("cdj3k-emu: could not destroy the bridge: {e}"),
        }
    }
}

/// The named mutex setup and teardown hold, released when dropped.
struct BridgeLock(Option<HANDLE>);

impl BridgeLock {
    /// Wait up to a minute for the mutex; the caller then goes on without it.
    fn take() -> Self {
        // SAFETY: no security attributes, a static NUL-terminated name.
        let Ok(h) = (unsafe { CreateMutexW(None, false, w!("Global\\cdj3k-emu-bridge")) }) else {
            return Self(None);
        };
        // SAFETY: `h` is the mutex just opened. An abandoned mutex is owned
        // all the same.
        let waited = unsafe { WaitForSingleObject(h, 60_000) };
        if waited == WAIT_OBJECT_0 || waited.0 == 0x80 {
            Self(Some(h))
        } else {
            let _ = unsafe { CloseHandle(h) };
            Self(None)
        }
    }
}

impl Drop for BridgeLock {
    fn drop(&mut self) {
        if let Some(h) = self.0 {
            // SAFETY: this thread owns the mutex; the handle is closed once.
            unsafe {
                let _ = ReleaseMutex(h);
                let _ = CloseHandle(h);
            }
        }
    }
}

/// Run `netsh`, returning its output, or its output as the error when it
/// exits non-zero.
pub(super) fn netsh(args: &[&str]) -> Result<String, String> {
    let out = cdj3k_emu_platform::child::command("netsh")
        .args(args)
        .output()
        .map_err(|e| format!("cannot run netsh: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() {
        Ok(text)
    } else {
        Err(text)
    }
}
