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
use std::{fs, io, thread};

use windows::core::{w, GUID, HSTRING, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupCopyOEMInfW, SetupDiBuildDriverInfoList, SetupDiCallClassInstaller,
    SetupDiCreateDeviceInfoList, SetupDiCreateDeviceInfoW, SetupDiDestroyDeviceInfoList,
    SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiOpenDevRegKey,
    SetupDiSetDeviceRegistryPropertyW, DICD_GENERATE_ID, DICS_FLAG_GLOBAL, DIF_INSTALLDEVICE,
    DIF_INSTALLINTERFACES, DIF_REGISTERDEVICE, DIF_REGISTER_COINSTALLERS, DIF_REMOVE,
    DIF_PROPERTYCHANGE, DIF_SELECTBESTCOMPATDRV, DICS_PROPCHANGE, DIGCF_PRESENT, DIREG_DRV, GUID_DEVCLASS_NET, HDEVINFO,
    SetupDiSetClassInstallParamsW, SPDIT_COMPATDRIVER, SPDRP_HARDWAREID, SP_CLASSINSTALL_HEADER, SP_PROPCHANGE_PARAMS, SPOST_PATH, SP_COPY_NOOVERWRITE, SP_DEVINFO_DATA,
};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::NetworkManagement::IpHelper::{
    ConvertInterfaceGuidToLuid, GetIfEntry2, MIB_IF_ROW2,
};
use windows::Win32::NetworkManagement::Ndis::MediaConnectStateConnected;
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE,
    KEY_READ, KEY_SET_VALUE, REG_SZ,
};
use windows::Win32::System::Threading::{
    CreateMutexW, OpenProcess, ReleaseMutex, WaitForSingleObject, PROCESS_SYNCHRONIZE,
};

use super::lease::{CLAIM_FILE, RELEASED_FILE};
use super::winbridge::{
    self, Outcome, Request, HELPER_FLAG, RESULT_FILE, WATCH_FLAG,
};

const TAP_HWID: &str = "tap0901";
/// Where the class-keyed connection names live; QEMU looks adapters up here.
const NETWORK_KEY: &str =
    r"SYSTEM\CurrentControlSet\Control\Network\{4D36E972-E325-11CE-BFC1-08002BE10318}";
/// How often the watcher looks at the app and the claim.
const WATCH_POLL: Duration = Duration::from_millis(500);
/// How long a new adapter is given to get its interface GUID, and to become
/// openable.
const NET_CFG_WAIT: Duration = Duration::from_secs(15);
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
        eprintln!("cdj3k-emu: {}: malformed request", flag.unwrap_or_default());
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
    let base = std::env::var_os("ProgramData").map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from);
    base.join("cdj3k-emu").join("bridge-owned")
}

fn run(req: &Request) -> Outcome {
    let mut named = taps_named(&req.tap);
    // Copies an earlier failed setup left under the slot's name.
    for extra in named.iter().skip(1) {
        if !is_connected(extra) {
            let _ = remove_tap(extra);
        }
    }
    named.truncate(1);
    let guid = match named.pop() {
        Some(guid) => guid,
        None => {
            let Some(inf) = &req.inf else {
                return Outcome::TapFailed("the TAP-Windows6 driver is not installed".into());
            };
            match create_tap(inf, &req.tap) {
                Ok(guid) => guid,
                Err(e) => return Outcome::TapFailed(e.to_string()),
            }
        }
    };
    let Some(mac) = winbridge::tap_mac(&req.mac) else {
        return Outcome::TapFailed(format!("no tap MAC for {}", req.mac));
    };
    if let Err(e) = configure_tap(&guid, &mac) {
        return Outcome::TapFailed(format!("could not configure {}: {e}", req.tap));
    }
    // `netsh bridge` names adapters by their alias.
    if !set_alias(&guid, &req.tap) {
        return Outcome::TapFailed(format!("the adapter {guid} could not be named {}", req.tap));
    }
    let bridge = match winbridge::guids(&netsh(&["bridge", "list"]).unwrap_or_default())
        .into_iter()
        .next()
    {
        Some(bridge) => bridge,
        None => {
            // The bridge takes the MAC of the adapter it is created with:
            // the tap's, fixed per slot ([`winbridge::tap_mac`]).
            if let Err(e) = netsh(&["bridge", "create", &req.tap, &req.nic]) {
                return Outcome::BridgeFailed(e);
            }
            let Some(bridge) =
                winbridge::guids(&netsh(&["bridge", "list"]).unwrap_or_default()).into_iter().next()
            else {
                return Outcome::BridgeFailed("netsh made no bridge".into());
            };
            let owned = owned_bridge_file();
            let recorded = owned
                .parent()
                .map_or(Ok(()), fs::create_dir_all)
                .and_then(|()| fs::write(&owned, &bridge));
            if let Err(e) = recorded {
                eprintln!("cdj3k-emu: could not record the bridge in {}: {e}", owned.display());
            }
            bridge
        }
    };
    // `create` reports success with only its first adapter joined; `add`
    // joins the rest.
    for name in [&req.tap, &req.nic] {
        let shown = netsh(&["bridge", "show", "adapter"]).unwrap_or_default();
        if winbridge::is_bridged(&shown, name) != Some(true) {
            if let Err(e) = netsh(&["bridge", "add", name, "to", &bridge]) {
                return Outcome::BridgeFailed(e);
            }
        }
    }
    let shown = netsh(&["bridge", "show", "adapter"]).unwrap_or_default();
    for name in [&req.tap, &req.nic] {
        if winbridge::is_bridged(&shown, name) != Some(true) {
            return Outcome::BridgeFailed(format!("{name} did not join the bridge:\n{shown}"));
        }
    }
    if !wait_openable(&guid) {
        return Outcome::TapFailed(format!("{} never became openable", req.tap));
    }
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => return Outcome::BridgeFailed(format!("cannot find the app to watch the link: {e}")),
    };
    let watcher = Request { inf: None, ..req.clone() };
    if let Err(e) = cdj3k_emu_platform::child::command(&exe)
        .arg(WATCH_FLAG)
        .args(watcher.args())
        .spawn()
    {
        return Outcome::BridgeFailed(format!("cannot start the link's watcher: {e}"));
    }
    Outcome::Done
}

/// Wait for the slot to let its link go, then take it down.
fn watch(req: &Request) {
    let claim_path = req.dir.join(CLAIM_FILE);
    let claimed = || fs::read_to_string(&claim_path).is_ok_and(|c| c.trim() == req.claim);
    // SAFETY: the handle is closed below; a failed open reads as an app that
    // has already gone.
    let app = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, req.pid) }.ok();
    let app_alive = || {
        app.is_some_and(|h| unsafe { WaitForSingleObject(h, WATCH_POLL.as_millis() as u32) } != WAIT_OBJECT_0)
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

/// Wait until the adapter `guid` can be opened the way QEMU opens it: a new
/// adapter's device appears some time after its install.
fn wait_openable(guid: &str) -> bool {
    let path = format!(r"\\.\Global\{guid}.tap");
    let deadline = Instant::now() + NET_CFG_WAIT;
    loop {
        if fs::OpenOptions::new().read(true).write(true).open(&path).is_ok() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(200));
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

/// Whether a program (QEMU) holds the adapter `guid` open: TAP-Windows6
/// reports media connected while it is.
fn is_connected(guid: &str) -> bool {
    let Ok(id) = GUID::try_from(guid.trim_matches(['{', '}'])) else {
        return false;
    };
    let mut row = MIB_IF_ROW2::default();
    // SAFETY: `row` is a zeroed MIB_IF_ROW2 whose LUID the first call fills
    // and the second reads.
    unsafe {
        ConvertInterfaceGuidToLuid(&id, &mut row.InterfaceLuid).is_ok()
            && GetIfEntry2(&mut row).is_ok()
            && row.MediaConnectState == MediaConnectStateConnected
    }
}

/// The interface GUIDs of the present TAP-Windows6 adapters whose connection
/// name is `name`, the name QEMU opens them by.
fn taps_named(name: &str) -> Vec<String> {
    present_taps()
        .into_iter()
        .filter(|guid| connection_name(guid).is_some_and(|n| n.eq_ignore_ascii_case(name)))
        .collect()
}

/// The interface GUIDs of the present TAP-Windows6 adapters.
fn present_taps() -> Vec<String> {
    let class: GUID = GUID_DEVCLASS_NET;
    let mut guids = Vec::new();
    // SAFETY: the set and the device data outlive every call made with them;
    // `info.cbSize` is set before each use.
    unsafe {
        let Ok(set) = SetupDiGetClassDevsW(Some(&class), PCWSTR::null(), None, DIGCF_PRESENT) else {
            return guids;
        };
        let set = DevInfoSet(set);
        let mut index = 0;
        loop {
            let mut info = SP_DEVINFO_DATA {
                cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            if SetupDiEnumDeviceInfo(set.0, index, &mut info).is_err() {
                return guids;
            }
            index += 1;
            let Ok(key) = SetupDiOpenDevRegKey(set.0, &info, DICS_FLAG_GLOBAL.0, 0, DIREG_DRV, KEY_READ.0)
            else {
                continue;
            };
            let component = read_string(key, w!("ComponentId"));
            let id = read_string(key, w!("NetCfgInstanceId"));
            let _ = RegCloseKey(key);
            if let (Some(component), Some(id)) = (component, id) {
                if component.eq_ignore_ascii_case(TAP_HWID) {
                    guids.push(id);
                }
            }
        }
    }
}

/// The connection name of the adapter `guid`.
fn connection_name(guid: &str) -> Option<String> {
    let path = HSTRING::from(format!(r"{NETWORK_KEY}\{guid}\Connection"));
    let mut key = HKEY::default();
    // SAFETY: `key` receives the opened handle, closed below.
    unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, &path, None, KEY_READ, &mut key) }.ok().ok()?;
    let name = read_string(key, w!("Name"));
    // SAFETY: opened above.
    let _ = unsafe { RegCloseKey(key) };
    name
}

/// Call `f` with the present network device whose `NetCfgInstanceId` is
/// `guid`.
fn with_device<T>(guid: &str, f: impl FnOnce(HDEVINFO, &SP_DEVINFO_DATA) -> io::Result<T>) -> io::Result<T> {
    let class: GUID = GUID_DEVCLASS_NET;
    // SAFETY: the set and the device data outlive every call made with them;
    // `info.cbSize` is set before each use.
    unsafe {
        let set = DevInfoSet(
            SetupDiGetClassDevsW(Some(&class), PCWSTR::null(), None, DIGCF_PRESENT)
                .map_err(io::Error::from)?,
        );
        let mut index = 0;
        loop {
            let mut info = SP_DEVINFO_DATA {
                cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            if SetupDiEnumDeviceInfo(set.0, index, &mut info).is_err() {
                return Err(io::Error::new(io::ErrorKind::NotFound, format!("no device {guid}")));
            }
            index += 1;
            let Ok(key) = SetupDiOpenDevRegKey(set.0, &info, DICS_FLAG_GLOBAL.0, 0, DIREG_DRV, KEY_READ.0)
            else {
                continue;
            };
            let id = read_string(key, w!("NetCfgInstanceId"));
            let _ = RegCloseKey(key);
            if id.is_some_and(|id| id.eq_ignore_ascii_case(guid)) {
                return f(set.0, &info);
            }
        }
    }
}

/// Remove the present network device whose `NetCfgInstanceId` is `guid`.
fn remove_tap(guid: &str) -> io::Result<()> {
    with_device(guid, |set, info| {
        // SAFETY: `set` and `info` are the live device `with_device` found.
        unsafe { SetupDiCallClassInstaller(DIF_REMOVE, set, Some(info)) }.map_err(io::Error::from)
    })
}

/// Set the adapter `guid`'s parameters, which TAP-Windows6 reads when it
/// starts, and restart it when one changed:
///
/// * `AllowNonAdmin` "1": the device opens to a program without admin rights
///   (QEMU). The class installer steps do not write the INF's default into
///   the adapter's key.
/// * `NetworkAddress` `mac`: the adapter's own MAC ([`winbridge::tap_mac`]).
fn configure_tap(guid: &str, mac: &str) -> io::Result<()> {
    with_device(guid, |set, info| {
        // SAFETY: `set` and `info` are the live device `with_device` found;
        // the key is closed before the restart.
        unsafe {
            let key = SetupDiOpenDevRegKey(set, info, DICS_FLAG_GLOBAL.0, 0, DIREG_DRV, (KEY_READ | KEY_SET_VALUE).0)
                .map_err(io::Error::from)?;
            let mut changed = false;
            let mut written = Ok(());
            for (name, want) in [(w!("AllowNonAdmin"), "1"), (w!("NetworkAddress"), mac)] {
                if read_string(key, name).is_some_and(|v| v.eq_ignore_ascii_case(want)) {
                    continue;
                }
                let value: Vec<u8> = want.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
                written = written.and(RegSetValueExW(key, name, None, REG_SZ, Some(&value)).ok());
                changed = true;
            }
            let _ = RegCloseKey(key);
            written.map_err(io::Error::from)?;
            if !changed {
                return Ok(());
            }
            let params = SP_PROPCHANGE_PARAMS {
                ClassInstallHeader: SP_CLASSINSTALL_HEADER {
                    cbSize: std::mem::size_of::<SP_CLASSINSTALL_HEADER>() as u32,
                    InstallFunction: DIF_PROPERTYCHANGE,
                },
                StateChange: DICS_PROPCHANGE,
                Scope: DICS_FLAG_GLOBAL,
                HwProfile: 0,
            };
            SetupDiSetClassInstallParamsW(
                set,
                Some(info),
                Some(&params.ClassInstallHeader),
                std::mem::size_of::<SP_PROPCHANGE_PARAMS>() as u32,
            )
            .map_err(io::Error::from)?;
            SetupDiCallClassInstaller(DIF_PROPERTYCHANGE, set, Some(info)).map_err(io::Error::from)
        }
    })
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

/// A device list handle, destroyed when it goes.
struct DevInfoSet(HDEVINFO);

impl Drop for DevInfoSet {
    fn drop(&mut self) {
        // SAFETY: the handle came from `SetupDiCreateDeviceInfoList`.
        let _ = unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

/// Register a root-enumerated `tap0901` device, install the driver in `inf`
/// on it alone and name its connection `name`. Returns its interface GUID.
fn create_tap(inf: &Path, name: &str) -> io::Result<String> {
    if !inf.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("the TAP-Windows6 driver is not installed: {} is missing", inf.display()),
        ));
    }
    // SAFETY: a NUL-terminated path; no output buffers are asked for.
    unsafe {
        // Puts the package in the driver store, where the compatible-driver
        // search below finds it. Already there is fine.
        let _ = SetupCopyOEMInfW(
            &HSTRING::from(inf.as_os_str()),
            PCWSTR::null(),
            SPOST_PATH,
            SP_COPY_NOOVERWRITE,
            None,
            None,
            None,
        );
    }
    let class: GUID = GUID_DEVCLASS_NET;
    // SAFETY: each call below is handed live handles and buffers that outlive
    // it; `info.cbSize` is set before it is passed anywhere.
    unsafe {
        let set = DevInfoSet(SetupDiCreateDeviceInfoList(Some(&class), None).map_err(io::Error::from)?);
        let mut info = SP_DEVINFO_DATA {
            cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
            ..Default::default()
        };
        SetupDiCreateDeviceInfoW(set.0, w!("NET"), &class, PCWSTR::null(), None, DICD_GENERATE_ID, Some(&mut info))
            .map_err(io::Error::from)?;

        // A REG_MULTI_SZ: the id, its NUL, and the list's closing NUL.
        let hwid: Vec<u8> = TAP_HWID
            .encode_utf16()
            .chain([0, 0])
            .flat_map(u16::to_le_bytes)
            .collect();
        SetupDiSetDeviceRegistryPropertyW(set.0, &mut info, SPDRP_HARDWAREID, Some(&hwid))
            .map_err(io::Error::from)?;
        SetupDiBuildDriverInfoList(set.0, Some(&mut info), SPDIT_COMPATDRIVER).map_err(io::Error::from)?;
        SetupDiCallClassInstaller(DIF_SELECTBESTCOMPATDRV, set.0, Some(&info))
            .map_err(|e| io::Error::other(format!("no TAP-Windows6 driver in the driver store: {e}")))?;
        SetupDiCallClassInstaller(DIF_REGISTERDEVICE, set.0, Some(&info)).map_err(io::Error::from)?;

        // The device is registered; a failure from here removes it.
        let installed = [DIF_REGISTER_COINSTALLERS, DIF_INSTALLINTERFACES, DIF_INSTALLDEVICE]
            .into_iter()
            .try_for_each(|step| SetupDiCallClassInstaller(step, set.0, Some(&info)));
        if let Err(e) = installed {
            let _ = SetupDiCallClassInstaller(DIF_REMOVE, set.0, Some(&info));
            return Err(io::Error::other(format!("installing the TAP driver failed: {e}")));
        }

        // The network setup writes the interface GUID after the install
        // returns.
        let deadline = Instant::now() + NET_CFG_WAIT;
        let guid = loop {
            let guid = SetupDiOpenDevRegKey(set.0, &info, DICS_FLAG_GLOBAL.0, 0, DIREG_DRV, KEY_READ.0)
                .ok()
                .and_then(|key| {
                    let guid = read_string(key, w!("NetCfgInstanceId"));
                    let _ = RegCloseKey(key);
                    guid
                });
            if guid.is_some() || Instant::now() >= deadline {
                break guid;
            }
            thread::sleep(Duration::from_millis(200));
        };
        let named = guid
            .ok_or_else(|| io::Error::other("the new adapter has no NetCfgInstanceId"))
            .and_then(|guid| set_connection_name(&guid, name).map(|()| guid));
        match named {
            Ok(guid) => Ok(guid),
            Err(e) => {
                let _ = SetupDiCallClassInstaller(DIF_REMOVE, set.0, Some(&info));
                Err(e)
            }
        }
    }
}

/// Set the interface alias (what `GetIfTable2`, `netsh` and Network
/// Connections show) to `name`, retrying for 20 s while the new adapter
/// appears to `Get-NetAdapter`. The registry name alone reaches the alias only
/// at the next boot. Returns whether the alias is `name`.
fn set_alias(guid: &str, name: &str) -> bool {
    let script = format!(
        "$deadline = (Get-Date).AddSeconds(20); \
         do {{ \
           $a = Get-NetAdapter -IncludeHidden | Where-Object {{ $_.InterfaceGuid -eq '{guid}' }}; \
           if ($a -and $a.Name -eq '{name}') {{ exit 0 }}; \
           if ($a) {{ $a | Rename-NetAdapter -NewName '{name}' -ErrorAction SilentlyContinue }}; \
           Start-Sleep -Milliseconds 300 \
         }} while ((Get-Date) -lt $deadline); exit 1"
    );
    cdj3k_emu_platform::child::command("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()
        .is_ok_and(|s| s.success())
}

/// Write the connection name Network Connections and QEMU both read. The
/// `Connection` key appears once the network stack has picked the adapter up.
fn set_connection_name(guid: &str, name: &str) -> io::Result<()> {
    let path = HSTRING::from(format!(r"{NETWORK_KEY}\{guid}\Connection"));
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let mut key = HKEY::default();
        // SAFETY: `key` receives the opened handle, closed below.
        let opened = unsafe {
            RegOpenKeyExW(HKEY_LOCAL_MACHINE, &path, None, KEY_SET_VALUE, &mut key)
        };
        if opened.is_ok() {
            let value: Vec<u8> = name
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect();
            // SAFETY: `key` is the handle opened above.
            let set = unsafe { RegSetValueExW(key, w!("Name"), None, REG_SZ, Some(&value)) };
            let _ = unsafe { RegCloseKey(key) };
            return set.ok().map_err(io::Error::from);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the new adapter never showed up in Network Connections",
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

/// A REG_SZ value of `key`.
fn read_string(key: HKEY, value: PCWSTR) -> Option<String> {
    let mut len = 0u32;
    // SAFETY: a null data pointer asks only for the size.
    unsafe { RegQueryValueExW(key, value, None, None, None, Some(&mut len)) }.ok().ok()?;
    let mut buf = vec![0u8; len as usize];
    // SAFETY: `buf` is `len` bytes.
    unsafe {
        RegQueryValueExW(key, value, None, None, Some(buf.as_mut_ptr()), Some(&mut len))
    }
    .ok()
    .ok()?;
    let wide: Vec<u16> = buf[..len as usize]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect();
    Some(String::from_utf16_lossy(&wide))
}
