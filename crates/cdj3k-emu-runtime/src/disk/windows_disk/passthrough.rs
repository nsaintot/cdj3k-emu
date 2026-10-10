//! Give a whole disk to the guest. Its volumes stay locked and dismounted
//! while the guest has the disk, and are then released to Windows.

use std::io;
use std::os::windows::io::OwnedHandle;
use std::sync::Mutex;
use std::time::Duration;

use windows_sys::Win32::System::Ioctl::{
    FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, IOCTL_DISK_UPDATE_PROPERTIES,
};

use super::enumerate::{volume_label, volumes_of, Volume};
use super::grant::grant_access;
use super::holders::holders_of;
use super::{ioctl, open, GENERIC_READ, GENERIC_WRITE};
use crate::disk::windows_parse as parse;

/// `FSCTL_LOCK_VOLUME` fails while any program has a file open. Indexers and
/// scanners close their files within moments, so the lock is tried a few times.
const LOCK_TRIES: u32 = 4;
const LOCK_RETRY: Duration = Duration::from_millis(400);

/// What keeps a disk's volumes from being mounted again while the guest has it.
struct Held {
    volumes: Vec<Volume>,
    /// Handles to the locked, dismounted volumes. While a handle is open,
    /// Windows cannot mount its volume.
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

/// Lock and dismount every volume of a disk, and keep them that way until
/// [`mount_disk`] runs. Ask once for administrator rights when the disk or a
/// volume refuses this user.
///
/// If a program still has a file open on a volume, the unmount fails with
/// `ResourceBusy` and names the programs that hold it, so the user knows what
/// to close.
pub fn unmount_disk(name: &str) -> io::Result<()> {
    let n = parse::drive_number(name).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("not a disk name: {name}"),
        )
    })?;
    // Release any hold left by an earlier attach; otherwise the disk's own
    // volumes look busy.
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
/// This is best effort, because the user may have unplugged the disk.
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
/// once for administrator rights when this user is not allowed to.
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
