//! The slot's TAP-Windows6 adapter, for the elevated helper
//! ([`super::windows_helper`]): found by its connection name, created with
//! SetupAPI (the steps `devcon install` takes, with the driver bound to that
//! device alone), configured, named and removed.

use std::path::Path;
use std::time::{Duration, Instant};
use std::{fs, io, thread};

use windows::core::{w, GUID, HSTRING, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupCopyOEMInfW, SetupDiBuildDriverInfoList, SetupDiCallClassInstaller,
    SetupDiCreateDeviceInfoList, SetupDiCreateDeviceInfoW, SetupDiDestroyDeviceInfoList,
    SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiOpenDevRegKey,
    SetupDiSetClassInstallParamsW, SetupDiSetDeviceRegistryPropertyW, DICD_GENERATE_ID,
    DICS_FLAG_GLOBAL, DICS_PROPCHANGE, DIF_INSTALLDEVICE, DIF_INSTALLINTERFACES,
    DIF_PROPERTYCHANGE, DIF_REGISTERDEVICE, DIF_REGISTER_COINSTALLERS, DIF_REMOVE,
    DIF_SELECTBESTCOMPATDRV, DIGCF_PRESENT, DIREG_DRV, GUID_DEVCLASS_NET, HDEVINFO,
    SPDIT_COMPATDRIVER, SPDRP_HARDWAREID, SPOST_PATH, SP_CLASSINSTALL_HEADER, SP_COPY_NOOVERWRITE,
    SP_DEVINFO_DATA, SP_PROPCHANGE_PARAMS,
};
use windows::Win32::NetworkManagement::IpHelper::{
    ConvertInterfaceGuidToLuid, GetIfEntry2, MIB_IF_ROW2,
};
use windows::Win32::NetworkManagement::Ndis::MediaConnectStateConnected;
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE,
    KEY_READ, KEY_SET_VALUE, REG_SAM_FLAGS, REG_SZ,
};

const TAP_HWID: &str = "tap0901";
/// Where the class-keyed connection names live; QEMU looks adapters up here.
const NETWORK_KEY: &str =
    r"SYSTEM\CurrentControlSet\Control\Network\{4D36E972-E325-11CE-BFC1-08002BE10318}";
/// How long a new adapter is given to get its interface GUID, and to become
/// openable.
const NET_CFG_WAIT: Duration = Duration::from_secs(15);

/// Whether a program (QEMU) holds the adapter `guid` open: TAP-Windows6
/// reports media connected while it is.
pub(super) fn is_connected(guid: &str) -> bool {
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
pub(super) fn taps_named(name: &str) -> Vec<String> {
    present_taps()
        .into_iter()
        .filter(|guid| connection_name(guid).is_some_and(|n| n.eq_ignore_ascii_case(name)))
        .collect()
}

/// The interface GUIDs of the present TAP-Windows6 adapters.
fn present_taps() -> Vec<String> {
    let Ok(devices) = NetDevices::present() else {
        return Vec::new();
    };
    devices
        .filter_map(|(_, key)| {
            let component = key.string(w!("ComponentId"));
            let id = key.string(w!("NetCfgInstanceId"));
            component
                .filter(|c| c.eq_ignore_ascii_case(TAP_HWID))
                .and(id)
        })
        .collect()
}

/// The connection name of the adapter `guid`.
fn connection_name(guid: &str) -> Option<String> {
    let path = connection_key(guid);
    let mut key = HKEY::default();
    // SAFETY: `key` receives the opened handle, closed below.
    unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, &path, None, KEY_READ, &mut key) }
        .ok()
        .ok()?;
    let name = read_string(key, w!("Name"));
    // SAFETY: opened above.
    let _ = unsafe { RegCloseKey(key) };
    name
}

/// The registry key holding the adapter `guid`'s connection name.
fn connection_key(guid: &str) -> HSTRING {
    HSTRING::from(format!(r"{NETWORK_KEY}\{guid}\Connection"))
}

/// Call `f` with the present network device whose `NetCfgInstanceId` is
/// `guid`.
fn with_device<T>(
    guid: &str,
    f: impl FnOnce(HDEVINFO, &SP_DEVINFO_DATA) -> io::Result<T>,
) -> io::Result<T> {
    let mut devices = NetDevices::present()?;
    let found = devices.by_ref().find_map(|(info, key)| {
        key.string(w!("NetCfgInstanceId"))
            .is_some_and(|id| id.eq_ignore_ascii_case(guid))
            .then_some(info)
    });
    match found {
        Some(info) => f(devices.set.0, &info),
        None => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no device {guid}"),
        )),
    }
}

/// Remove the present network device whose `NetCfgInstanceId` is `guid`.
pub(super) fn remove_tap(guid: &str) -> io::Result<()> {
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
/// * `NetworkAddress` `mac`: the adapter's own MAC ([`super::winbridge::tap_mac`]).
pub(super) fn configure_tap(guid: &str, mac: &str) -> io::Result<()> {
    with_device(guid, |set, info| {
        if write_tap_params(set, info, mac)? {
            restart_device(set, info)
        } else {
            Ok(())
        }
    })
}

/// Write the parameters [`configure_tap`] names into the device's driver key.
/// Returns whether one changed.
fn write_tap_params(set: HDEVINFO, info: &SP_DEVINFO_DATA, mac: &str) -> io::Result<bool> {
    let key = DriverKey::open(set, info, KEY_READ | KEY_SET_VALUE)?;
    let mut changed = false;
    let mut written = Ok(());
    for (name, want) in [(w!("AllowNonAdmin"), "1"), (w!("NetworkAddress"), mac)] {
        if key
            .string(name)
            .is_some_and(|v| v.eq_ignore_ascii_case(want))
        {
            continue;
        }
        // SAFETY: `key` is open for writing; the value outlives the call.
        let result = unsafe { RegSetValueExW(key.0, name, None, REG_SZ, Some(&reg_sz(want))) };
        written = written.and(result.ok());
        changed = true;
    }
    drop(key);
    written.map_err(io::Error::from)?;
    Ok(changed)
}

/// Restart a device so its driver reads its parameters again.
fn restart_device(set: HDEVINFO, info: &SP_DEVINFO_DATA) -> io::Result<()> {
    let params = SP_PROPCHANGE_PARAMS {
        ClassInstallHeader: SP_CLASSINSTALL_HEADER {
            cbSize: std::mem::size_of::<SP_CLASSINSTALL_HEADER>() as u32,
            InstallFunction: DIF_PROPERTYCHANGE,
        },
        StateChange: DICS_PROPCHANGE,
        Scope: DICS_FLAG_GLOBAL,
        HwProfile: 0,
    };
    // SAFETY: `set` and `info` are a live device; `params` outlives the call
    // and its size is passed.
    unsafe {
        SetupDiSetClassInstallParamsW(
            set,
            Some(info),
            Some(&params.ClassInstallHeader),
            std::mem::size_of::<SP_PROPCHANGE_PARAMS>() as u32,
        )
        .map_err(io::Error::from)?;
        SetupDiCallClassInstaller(DIF_PROPERTYCHANGE, set, Some(info)).map_err(io::Error::from)
    }
}

/// Register a root-enumerated `tap0901` device, install the driver in `inf`
/// on it alone and name its connection `name`. Returns its interface GUID.
pub(super) fn create_tap(inf: &Path, name: &str) -> io::Result<String> {
    if !inf.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "the TAP-Windows6 driver is not installed: {} is missing",
                inf.display()
            ),
        ));
    }
    stage_driver_package(inf);
    let (set, info) = register_tap_device()?;
    // The device is registered; a failure from here removes it.
    let named = install_driver(&set, &info)
        .and_then(|()| await_interface_guid(&set, &info))
        .and_then(|guid| set_connection_name(&guid, name).map(|()| guid));
    if named.is_err() {
        // SAFETY: `set` and `info` are the device registered above.
        let _ = unsafe { SetupDiCallClassInstaller(DIF_REMOVE, set.0, Some(&info)) };
    }
    named
}

/// Put the package in the driver store, where the compatible-driver search
/// finds it. A package that is already in the store is not an error.
fn stage_driver_package(inf: &Path) {
    // SAFETY: a NUL-terminated path; no output buffers are asked for.
    let _ = unsafe {
        SetupCopyOEMInfW(
            &HSTRING::from(inf.as_os_str()),
            PCWSTR::null(),
            SPOST_PATH,
            SP_COPY_NOOVERWRITE,
            None,
            None,
            None,
        )
    };
}

/// Create a network device with the `tap0901` hardware ID, select the best
/// driver in the store for it and register it.
fn register_tap_device() -> io::Result<(DevInfoSet, SP_DEVINFO_DATA)> {
    let class: GUID = GUID_DEVCLASS_NET;
    // SAFETY: each call below is handed live handles and buffers that outlive
    // it; `info.cbSize` is set before it is passed anywhere.
    unsafe {
        let set =
            DevInfoSet(SetupDiCreateDeviceInfoList(Some(&class), None).map_err(io::Error::from)?);
        let mut info = SP_DEVINFO_DATA {
            cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
            ..Default::default()
        };
        SetupDiCreateDeviceInfoW(
            set.0,
            w!("NET"),
            &class,
            PCWSTR::null(),
            None,
            DICD_GENERATE_ID,
            Some(&mut info),
        )
        .map_err(io::Error::from)?;

        // A REG_MULTI_SZ: the id, its NUL, and the list's closing NUL.
        let hwid: Vec<u8> = TAP_HWID
            .encode_utf16()
            .chain([0, 0])
            .flat_map(u16::to_le_bytes)
            .collect();
        SetupDiSetDeviceRegistryPropertyW(set.0, &mut info, SPDRP_HARDWAREID, Some(&hwid))
            .map_err(io::Error::from)?;
        SetupDiBuildDriverInfoList(set.0, Some(&mut info), SPDIT_COMPATDRIVER)
            .map_err(io::Error::from)?;
        SetupDiCallClassInstaller(DIF_SELECTBESTCOMPATDRV, set.0, Some(&info)).map_err(|e| {
            io::Error::other(format!("no TAP-Windows6 driver in the driver store: {e}"))
        })?;
        SetupDiCallClassInstaller(DIF_REGISTERDEVICE, set.0, Some(&info))
            .map_err(io::Error::from)?;
        Ok((set, info))
    }
}

/// Run the class installer's install steps on a registered device.
fn install_driver(set: &DevInfoSet, info: &SP_DEVINFO_DATA) -> io::Result<()> {
    [
        DIF_REGISTER_COINSTALLERS,
        DIF_INSTALLINTERFACES,
        DIF_INSTALLDEVICE,
    ]
    .into_iter()
    .try_for_each(|step| {
        // SAFETY: `set` and `info` are a live registered device.
        unsafe { SetupDiCallClassInstaller(step, set.0, Some(info)) }
    })
    .map_err(|e| io::Error::other(format!("installing the TAP driver failed: {e}")))
}

/// The new device's interface GUID. The network setup writes it after the
/// install returns.
fn await_interface_guid(set: &DevInfoSet, info: &SP_DEVINFO_DATA) -> io::Result<String> {
    poll(NET_CFG_WAIT, Duration::from_millis(200), || {
        DriverKey::open(set.0, info, KEY_READ)
            .ok()?
            .string(w!("NetCfgInstanceId"))
    })
    .ok_or_else(|| io::Error::other("the new adapter has no NetCfgInstanceId"))
}

/// Wait until the adapter `guid` can be opened the way QEMU opens it: a new
/// adapter's device appears some time after its install.
pub(super) fn wait_openable(guid: &str) -> bool {
    let path = format!(r"\\.\Global\{guid}.tap");
    poll(NET_CFG_WAIT, Duration::from_millis(200), || {
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .ok()
    })
    .is_some()
}

/// Set the interface alias (what `GetIfTable2`, `netsh` and Network
/// Connections show) to `name`, retrying for 20 s while the new adapter
/// appears to `Get-NetAdapter`. The registry name alone reaches the alias only
/// at the next boot. Returns whether the alias is `name`.
pub(super) fn set_alias(guid: &str, name: &str) -> bool {
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
    let path = connection_key(guid);
    poll(Duration::from_secs(20), Duration::from_millis(250), || {
        let mut key = HKEY::default();
        // SAFETY: `key` receives the opened handle, closed below.
        unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, &path, None, KEY_SET_VALUE, &mut key) }
            .ok()
            .ok()?;
        // SAFETY: `key` is the handle opened above.
        let set = unsafe { RegSetValueExW(key, w!("Name"), None, REG_SZ, Some(&reg_sz(name))) };
        let _ = unsafe { RegCloseKey(key) };
        Some(set.ok().map_err(io::Error::from))
    })
    .unwrap_or_else(|| {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "the new adapter never showed up in Network Connections",
        ))
    })
}

/// Call `attempt` until it gives a value or `timeout` has passed, pausing
/// `every` between tries.
fn poll<T>(
    timeout: Duration,
    every: Duration,
    mut attempt: impl FnMut() -> Option<T>,
) -> Option<T> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = attempt() {
            return Some(value);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(every);
    }
}

/// A device list handle, destroyed when it goes.
struct DevInfoSet(HDEVINFO);

impl Drop for DevInfoSet {
    fn drop(&mut self) {
        // SAFETY: the handle came from `SetupDiCreateDeviceInfoList` or
        // `SetupDiGetClassDevsW`.
        let _ = unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

/// The present network-class devices whose driver key opens for reading,
/// each with that key.
struct NetDevices {
    set: DevInfoSet,
    index: u32,
}

impl NetDevices {
    fn present() -> io::Result<Self> {
        let class: GUID = GUID_DEVCLASS_NET;
        // SAFETY: the class GUID outlives the call.
        let set =
            unsafe { SetupDiGetClassDevsW(Some(&class), PCWSTR::null(), None, DIGCF_PRESENT) }
                .map_err(io::Error::from)?;
        Ok(Self {
            set: DevInfoSet(set),
            index: 0,
        })
    }
}

impl Iterator for NetDevices {
    type Item = (SP_DEVINFO_DATA, DriverKey);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let mut info = SP_DEVINFO_DATA {
                cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            // SAFETY: the set is live and `info.cbSize` is set.
            unsafe { SetupDiEnumDeviceInfo(self.set.0, self.index, &mut info) }.ok()?;
            self.index += 1;
            if let Ok(key) = DriverKey::open(self.set.0, &info, KEY_READ) {
                return Some((info, key));
            }
        }
    }
}

/// A device's driver key, closed when it goes.
struct DriverKey(HKEY);

impl DriverKey {
    fn open(set: HDEVINFO, info: &SP_DEVINFO_DATA, access: REG_SAM_FLAGS) -> io::Result<Self> {
        // SAFETY: `set` and `info` are a live device the caller holds.
        unsafe { SetupDiOpenDevRegKey(set, info, DICS_FLAG_GLOBAL.0, 0, DIREG_DRV, access.0) }
            .map(Self)
            .map_err(io::Error::from)
    }

    /// A REG_SZ value of the key.
    fn string(&self, value: PCWSTR) -> Option<String> {
        read_string(self.0, value)
    }
}

impl Drop for DriverKey {
    fn drop(&mut self) {
        // SAFETY: opened by `DriverKey::open`, closed once.
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

/// `s` as REG_SZ data: UTF-16LE with its NUL.
fn reg_sz(s: &str) -> Vec<u8> {
    s.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// A REG_SZ value of `key`.
fn read_string(key: HKEY, value: PCWSTR) -> Option<String> {
    let mut len = 0u32;
    // SAFETY: a null data pointer asks only for the size.
    unsafe { RegQueryValueExW(key, value, None, None, None, Some(&mut len)) }
        .ok()
        .ok()?;
    let mut buf = vec![0u8; len as usize];
    // SAFETY: `buf` is `len` bytes.
    unsafe {
        RegQueryValueExW(
            key,
            value,
            None,
            None,
            Some(buf.as_mut_ptr()),
            Some(&mut len),
        )
    }
    .ok()
    .ok()?;
    let wide: Vec<u16> = buf[..len as usize]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| u16::from_le_bytes(c))
        .take_while(|&c| c != 0)
        .collect();
    Some(String::from_utf16_lossy(&wide))
}
