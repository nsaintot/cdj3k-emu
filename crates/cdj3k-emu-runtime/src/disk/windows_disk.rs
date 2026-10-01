//! Removable-disk enumeration, volume locking, access grants and VHDX
//! creation on Windows.
//!
//! Disks are whole `\\.\PhysicalDriveN` devices, found by opening each number
//! with no access and asking the storage stack about it, which a standard user
//! may do. Only opening one for I/O needs rights, and the one elevated step
//! that grants them is [`grant_access`].

use std::collections::HashMap;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, GetVolumeInformationW,
    GetVolumePathNamesForVolumeNameW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::Storage::Vhd::{
    CreateVirtualDisk, CREATE_VIRTUAL_DISK_FLAG_NONE, CREATE_VIRTUAL_DISK_PARAMETERS,
    CREATE_VIRTUAL_DISK_VERSION_2, VIRTUAL_DISK_ACCESS_NONE, VIRTUAL_STORAGE_TYPE,
    VIRTUAL_STORAGE_TYPE_DEVICE_VHDX, VIRTUAL_STORAGE_TYPE_VENDOR_MICROSOFT,
};
use windows_sys::Win32::System::Ioctl::{
    FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
    IOCTL_DISK_UPDATE_PROPERTIES, IOCTL_STORAGE_QUERY_PROPERTY,
};
use windows_sys::Win32::System::RestartManager::{
    RmEndSession, RmGetList, RmRegisterResources, RmStartSession, CCH_RM_SESSION_KEY,
    RM_PROCESS_INFO,
};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows_sys::Win32::System::IO::DeviceIoControl;

use super::windows_parse as parse;
use super::PhysicalDisk;

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
/// `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS`, which windows-sys does not carry.
const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x0056_0000;
const SHARE_RW: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE;

/// Disk numbers probed.
const MAX_DRIVES: u32 = 32;

/// `FSCTL_LOCK_VOLUME` fails while anything has a file open; indexers and
/// scanners let go within moments, so it is tried a few times.
const LOCK_TRIES: u32 = 4;
const LOCK_RETRY: Duration = Duration::from_millis(400);

/// Most entries handed to the Restart Manager when naming what holds a
/// volume. A stick with more files than this is named from the first ones.
const HOLDER_SCAN_LIMIT: usize = 4096;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_os(p: &Path) -> Vec<u16> {
    p.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn open(path: &str, access: u32) -> io::Result<OwnedHandle> {
    let path = wide(path);
    // SAFETY: `path` is NUL-terminated and outlives the call.
    let h = unsafe {
        CreateFileW(
            path.as_ptr(),
            access,
            SHARE_RW,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `h` is a valid handle this call just created.
    Ok(unsafe { OwnedHandle::from_raw_handle(h) })
}

/// `DeviceIoControl`, returning the bytes written to `out`.
fn ioctl(h: &OwnedHandle, code: u32, input: &[u8], out: &mut [u8]) -> io::Result<usize> {
    let mut returned = 0u32;
    // SAFETY: both buffers are live for the call and their lengths are passed.
    let ok = unsafe {
        DeviceIoControl(
            h.as_raw_handle() as HANDLE,
            code,
            if input.is_empty() {
                std::ptr::null()
            } else {
                input.as_ptr().cast()
            },
            input.len() as u32,
            if out.is_empty() {
                std::ptr::null_mut()
            } else {
                out.as_mut_ptr().cast()
            },
            out.len() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(returned as usize)
    }
}

// ── Enumeration ──────────────────────────────────────────────────────────────

/// A volume and what says where it is.
#[derive(Clone, Debug)]
struct Volume {
    /// `\\?\Volume{guid}`: what a handle opens.
    path: String,
    /// The first drive letter or folder it is mounted at.
    mount: Option<String>,
}

/// Whole disks the user could plausibly hand to the deck.
pub fn list_removable() -> Vec<PhysicalDisk> {
    // A disk Windows runs from is never offered; when it cannot be told which
    // that is, nothing is.
    let Some(system) = system_disks() else {
        return Vec::new();
    };
    let volumes = volumes_by_disk();
    (0..MAX_DRIVES)
        .filter_map(|n| describe(n, system.contains(&n), volumes.get(&n)))
        .collect()
}

fn describe(n: u32, is_system: bool, volumes: Option<&Vec<Volume>>) -> Option<PhysicalDisk> {
    let path = parse::drive_path(n);
    let h = open(&path, 0).ok()?;

    let mut buf = [0u8; 1024];
    // PropertyId StorageDeviceProperty, QueryType PropertyStandardQuery: zeros.
    ioctl(&h, IOCTL_STORAGE_QUERY_PROPERTY, &[0u8; 12], &mut buf).ok()?;
    let descriptor = parse::parse_descriptor(&buf)?;

    // An empty card reader slot answers with no medium.
    let mut geometry = [0u8; 256];
    ioctl(&h, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &[], &mut geometry).ok()?;
    let size_bytes = parse::geometry_disk_size(&geometry)?;

    if !parse::is_eligible(descriptor.bus_type, size_bytes, is_system) {
        return None;
    }
    let volume_label = volumes
        .into_iter()
        .flatten()
        .find_map(|v| volume_label(&v.path))
        .unwrap_or_default();
    Some(PhysicalDisk {
        label: format!(
            "{} ({})",
            parse::display_name(&volume_label, &descriptor, n),
            parse::human_size(size_bytes)
        ),
        bsd_name: parse::drive_name(n),
        bsd_path: path,
        size_bytes,
    })
}

/// Disks the Windows directory lives on.
fn system_disks() -> Option<Vec<u32>> {
    let mut buf = [0u16; 260];
    // SAFETY: the buffer's length is passed.
    let len = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    let dir = String::from_utf16(buf.get(..len)?).ok()?;
    let drive: String = dir.chars().take(2).collect();
    if drive.len() != 2 || !drive.ends_with(':') {
        return None;
    }
    let disks = disks_of_volume(&format!(r"\\.\{drive}"))?;
    (!disks.is_empty()).then_some(disks)
}

/// Disk numbers a volume spans.
fn disks_of_volume(path: &str) -> Option<Vec<u32>> {
    let h = open(path, 0).ok()?;
    let mut buf = [0u8; 1024];
    ioctl(&h, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, &[], &mut buf).ok()?;
    Some(parse::parse_disk_extents(&buf))
}

/// Every mounted volume, under each disk it lies on.
fn volumes_by_disk() -> HashMap<u32, Vec<Volume>> {
    let mut by_disk: HashMap<u32, Vec<Volume>> = HashMap::new();
    let mut name = [0u16; 260];
    // SAFETY: the buffer's length is passed.
    let find = unsafe { FindFirstVolumeW(name.as_mut_ptr(), name.len() as u32) };
    if find == INVALID_HANDLE_VALUE {
        return by_disk;
    }
    loop {
        let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        let with_slash = String::from_utf16_lossy(&name[..len]);
        let path = with_slash.trim_end_matches('\\').to_string();
        if parse::is_volume_path(&path) {
            if let Some(disks) = disks_of_volume(&path) {
                let volume = Volume {
                    mount: mount_point(&with_slash),
                    path,
                };
                for d in disks {
                    by_disk.entry(d).or_default().push(volume.clone());
                }
            }
        }
        // SAFETY: `find` is the live search handle; the buffer's length is passed.
        if unsafe { FindNextVolumeW(find, name.as_mut_ptr(), name.len() as u32) } == 0 {
            break;
        }
    }
    // SAFETY: `find` came from FindFirstVolumeW.
    unsafe { FindVolumeClose(find) };
    by_disk
}

fn volumes_of(disk: u32) -> Vec<Volume> {
    volumes_by_disk().remove(&disk).unwrap_or_default()
}

/// The first place a volume is mounted, `E:\` for instance.
fn mount_point(volume_with_slash: &str) -> Option<String> {
    let name = wide(volume_with_slash);
    let mut buf = [0u16; 512];
    let mut len = 0u32;
    // SAFETY: `name` is NUL-terminated; the buffer's length is passed.
    let ok = unsafe {
        GetVolumePathNamesForVolumeNameW(
            name.as_ptr(),
            buf.as_mut_ptr(),
            buf.len() as u32,
            &mut len,
        )
    };
    if ok == 0 {
        return None;
    }
    let first = buf.split(|&c| c == 0).next()?;
    (!first.is_empty()).then(|| String::from_utf16_lossy(first))
}

/// The label written on a volume, when its filesystem is readable.
fn volume_label(volume_path: &str) -> Option<String> {
    let root = wide(&format!("{volume_path}\\"));
    let mut label = [0u16; 262];
    // SAFETY: `root` is NUL-terminated; the label buffer's length is passed and
    // the other outputs are not wanted.
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            label.as_mut_ptr(),
            label.len() as u32,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    if ok == 0 {
        return None;
    }
    let len = label.iter().position(|&c| c == 0)?;
    let label = String::from_utf16_lossy(&label[..len]);
    (!label.trim().is_empty()).then_some(label)
}

// ── Passthrough ──────────────────────────────────────────────────────────────

/// What keeps a disk's volumes from being mounted again while the guest has it.
struct Held {
    volumes: Vec<Volume>,
    /// Locked, dismounted volume handles: while one is open Windows cannot
    /// mount its volume.
    _handles: Vec<OwnedHandle>,
}

static HELD: Mutex<Vec<(String, Held)>> = Mutex::new(Vec::new());

fn held() -> std::sync::MutexGuard<'static, Vec<(String, Held)>> {
    HELD.lock().unwrap_or_else(|e| e.into_inner())
}

enum LockError {
    Open(io::Error),
    Busy,
}

fn lock_and_dismount(volume: &Volume) -> Result<OwnedHandle, LockError> {
    let h = open(&volume.path, GENERIC_READ | GENERIC_WRITE).map_err(LockError::Open)?;
    for attempt in 0..LOCK_TRIES {
        if ioctl(&h, FSCTL_LOCK_VOLUME, &[], &mut []).is_ok() {
            ioctl(&h, FSCTL_DISMOUNT_VOLUME, &[], &mut []).map_err(LockError::Open)?;
            return Ok(h);
        }
        if attempt + 1 < LOCK_TRIES {
            std::thread::sleep(LOCK_RETRY);
        }
    }
    Err(LockError::Busy)
}

/// Lock and dismount every volume of a disk, and keep them so until
/// [`mount_disk`]. Asks once for administrator rights when the disk or a
/// volume refuses this user.
///
/// A volume something still has open fails with `ResourceBusy`, naming the
/// programs holding it so the user knows what to close.
pub fn unmount_disk(name: &str) -> io::Result<()> {
    let n = parse::drive_number(name).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("not a disk name: {name}"),
        )
    })?;
    // An earlier attach's hold makes the disk's own volumes look busy.
    mount_disk(name)?;

    let volumes = volumes_of(n);
    ensure_access(n, &volumes)?;

    let mut handles = Vec::new();
    for v in &volumes {
        match lock_and_dismount(v) {
            Ok(h) => handles.push(h),
            Err(LockError::Open(e)) => return Err(e),
            Err(LockError::Busy) => {
                let mount = v.mount.clone().unwrap_or_else(|| v.path.clone());
                let holders = v.mount.as_deref().map(holders_of).unwrap_or_default();
                return Err(io::Error::new(
                    io::ErrorKind::ResourceBusy,
                    parse::busy_message(&mount, &holders),
                ));
            }
        }
    }
    held().push((
        name.to_string(),
        Held {
            volumes,
            _handles: handles,
        },
    ));
    Ok(())
}

/// Release the volumes and let Windows mount them again.
///
/// Best effort: the user may have unplugged the disk.
pub fn mount_disk(name: &str) -> io::Result<()> {
    let taken = {
        let mut held = held();
        held.iter()
            .position(|(n, _)| n == name)
            .map(|i| held.remove(i).1)
    };
    let Some(Held { volumes, _handles }) = taken else {
        return Ok(());
    };
    drop(_handles);
    // The guest may have rewritten the partition table.
    if let Some(n) = parse::drive_number(name) {
        if let Ok(h) = open(&parse::drive_path(n), 0) {
            let _ = ioctl(&h, IOCTL_DISK_UPDATE_PROPERTIES, &[], &mut []);
        }
    }
    // A volume mounts on first access.
    for v in volumes {
        let _ = volume_label(&v.path);
    }
    Ok(())
}

/// Open the disk and its volumes for I/O, as QEMU and the lock will, and ask
/// for administrator rights once when this user may not.
///
/// A refusal comes back as `PermissionDenied`.
fn ensure_access(n: u32, volumes: &[Volume]) -> io::Result<()> {
    let probe = || -> io::Result<()> {
        open(&parse::drive_path(n), GENERIC_READ | GENERIC_WRITE)?;
        for v in volumes {
            open(&v.path, GENERIC_READ | GENERIC_WRITE)?;
        }
        Ok(())
    };
    match probe() {
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {}
        other => return other,
    }
    grant_access(n, volumes)?;
    probe()
}

/// Run the grant script elevated and wait for it.
fn grant_access(n: u32, volumes: &[Volume]) -> io::Result<()> {
    let sid = current_user_sid()?;
    let paths: Vec<String> = volumes.iter().map(|v| v.path.clone()).collect();
    let err_file = scratch_file(&format!("grant-{n}.err"));
    let script = parse::grant_script(&sid, &parse::drive_path(n), &paths, &path_str(&err_file)?);
    run_script(&script, &err_file).map_err(|e| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("administrator access to the disk was not granted: {e}"),
        )
    })
}

/// Run PowerShell elevated, and turn a failure into the message the script
/// left in `err_file`, or the launcher's own.
fn run_script(script: &str, err_file: &Path) -> io::Result<()> {
    let _ = std::fs::remove_file(err_file);
    let result = crate::elevate::run_elevated(&parse::powershell_command(script));
    let message = std::fs::read_to_string(err_file).ok();
    let _ = std::fs::remove_file(err_file);
    match (result, message) {
        (Ok(()), None) => Ok(()),
        (Ok(()), Some(m)) | (Err(_), Some(m)) => Err(io::Error::other(m.trim().to_string())),
        (Err(e), None) => Err(e),
    }
}

fn scratch_file(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("cdj3k-emu-{}-{name}", std::process::id()))
}

fn path_str(p: &Path) -> io::Result<String> {
    p.to_str().map(str::to_string).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path is not valid Unicode: {}", p.display()),
        )
    })
}

/// The user this process runs as, as a SID string.
fn current_user_sid() -> io::Result<String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the pseudo-handle is always valid; `token` receives a new handle.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `token` is a valid handle this function now owns.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };

    // 8-byte aligned, as TOKEN_USER is.
    let mut buf = vec![0u64; 64];
    let mut needed = 0u32;
    // SAFETY: the buffer's byte length is passed.
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw_handle() as HANDLE,
            TokenUser,
            buf.as_mut_ptr().cast(),
            (buf.len() * 8) as u32,
            &mut needed,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the call filled the buffer with a TOKEN_USER whose SID points
    // into the same buffer.
    let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: `sid` is valid; `text` receives a LocalAlloc'd string.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `text` is a NUL-terminated string until it is freed below.
    let s = unsafe {
        let mut len = 0;
        while *text.add(len) != 0 {
            len += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
        LocalFree(text.cast());
        s
    };
    if parse::is_valid_sid(&s) {
        Ok(s)
    } else {
        Err(io::Error::other(format!("unexpected SID: {s}")))
    }
}

/// Names of the programs with a file open under `root`, each once.
///
/// The Restart Manager names the holders of files, not of a volume, so this
/// registers what lies under `root`, breadth first up to [`HOLDER_SCAN_LIMIT`]
/// entries.
fn holders_of(root: &str) -> Vec<String> {
    let mut paths: Vec<Vec<u16>> = Vec::new();
    let mut queue = std::collections::VecDeque::from([PathBuf::from(root)]);
    while paths.len() < HOLDER_SCAN_LIMIT {
        let Some(dir) = queue.pop_front() else {
            break;
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten().take(HOLDER_SCAN_LIMIT - paths.len()) {
            let path = entry.path();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                queue.push_back(path.clone());
            }
            paths.push(wide_os(&path));
        }
    }
    if paths.is_empty() {
        return Vec::new();
    }

    let mut session = 0u32;
    let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
    // SAFETY: the key buffer has the room the API documents.
    if unsafe { RmStartSession(&mut session, 0, key.as_mut_ptr()) } != 0 {
        return Vec::new();
    }
    let names = (|| {
        let ptrs: Vec<*const u16> = paths.iter().map(|p| p.as_ptr()).collect();
        // SAFETY: every pointer is a NUL-terminated string that outlives the call.
        let registered = unsafe {
            RmRegisterResources(
                session,
                ptrs.len() as u32,
                ptrs.as_ptr(),
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
            )
        };
        if registered != 0 {
            return Vec::new();
        }
        let mut have = 16u32;
        loop {
            // SAFETY: RM_PROCESS_INFO is plain data; zero is a valid value.
            let mut infos: Vec<RM_PROCESS_INFO> =
                (0..have).map(|_| unsafe { std::mem::zeroed() }).collect();
            let mut needed = 0u32;
            let mut reasons = 0u32;
            let mut count = have;
            // SAFETY: `infos` holds `count` entries.
            let rc = unsafe {
                RmGetList(
                    session,
                    &mut needed,
                    &mut count,
                    infos.as_mut_ptr(),
                    &mut reasons,
                )
            };
            if rc == ERROR_MORE_DATA && needed > have {
                have = needed;
                continue;
            }
            if rc != 0 {
                return Vec::new();
            }
            let mut names: Vec<String> = Vec::new();
            for info in &infos[..count as usize] {
                let len = info
                    .strAppName
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(info.strAppName.len());
                let name = String::from_utf16_lossy(&info.strAppName[..len]);
                if !name.is_empty() && !names.contains(&name) {
                    names.push(name);
                }
            }
            return names;
        }
    })();
    // SAFETY: `session` came from RmStartSession.
    unsafe { RmEndSession(session) };
    names
}

// ── Virtual image ────────────────────────────────────────────────────────────

/// Create an empty dynamic VHDX of `size_bytes`, rounded down to a whole MiB.
///
/// Made by this process so the file belongs to the user; attaching it for
/// partitioning is what needs administrator rights.
pub fn create_vhdx(path: &Path, size_bytes: u64) -> io::Result<()> {
    const MIB: u64 = 1 << 20;
    let size = size_bytes / MIB * MIB;
    if size < 2 * MIB {
        return Err(io::Error::other(format!(
            "{} is too small to hold a partition",
            path.display()
        )));
    }

    let storage_type = VIRTUAL_STORAGE_TYPE {
        DeviceId: VIRTUAL_STORAGE_TYPE_DEVICE_VHDX,
        VendorId: VIRTUAL_STORAGE_TYPE_VENDOR_MICROSOFT,
    };
    // SAFETY: the parameters are plain data; zero means "default" throughout.
    let mut params: CREATE_VIRTUAL_DISK_PARAMETERS = unsafe { std::mem::zeroed() };
    params.Version = CREATE_VIRTUAL_DISK_VERSION_2;
    params.Anonymous.Version2.MaximumSize = size;
    params.Anonymous.Version2.SectorSizeInBytes = 512;
    params.Anonymous.Version2.PhysicalSectorSizeInBytes = 512;
    let path_w = wide_os(path);
    let mut handle: HANDLE = std::ptr::null_mut();
    // SAFETY: every pointer is live for the call; `handle` receives the disk.
    let rc = unsafe {
        CreateVirtualDisk(
            &storage_type,
            path_w.as_ptr(),
            VIRTUAL_DISK_ACCESS_NONE,
            std::ptr::null_mut(),
            CREATE_VIRTUAL_DISK_FLAG_NONE,
            0,
            &params,
            std::ptr::null(),
            &mut handle,
        )
    };
    if rc != 0 {
        return Err(io::Error::from_raw_os_error(rc as i32));
    }
    // SAFETY: `handle` is the valid handle CreateVirtualDisk returned.
    drop(unsafe { OwnedHandle::from_raw_handle(handle) });
    Ok(())
}

/// Attach the VHDX, write an MBR and an exFAT partition, and detach it.
pub fn format_vhdx(path: &Path) -> io::Result<()> {
    let mut err_file = path.as_os_str().to_owned();
    err_file.push(".err");
    let err_file = PathBuf::from(err_file);
    run_script(
        &parse::format_script(&path_str(path)?, &path_str(&err_file)?),
        &err_file,
    )
}
