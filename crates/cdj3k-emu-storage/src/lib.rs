pub mod emmc;
pub mod gpt;
mod qcow2;
pub mod settings;
mod staging;

pub use emmc::{default_path, provision_emmc, EmmcConfig, FirmwareInfo};
pub use settings::{prune_app_file, InstanceSettings, PanelSettings};
pub use staging::{apply_staged, pending_install, request_restart, StagedFirmware, StagedRecord};

use std::path::PathBuf;

use cdj3k_emu_panel::Model;

/// `~/Library/Application Support/<BUNDLE_ID>/` on macOS,
/// `$XDG_DATA_HOME/<BUNDLE_ID>/` (fallback: `~/.local/share/<BUNDLE_ID>/`)
/// elsewhere. The directory name is the macOS bundle identifier, matching
/// the OS convention (reverse-DNS, never the human display name).
pub fn app_data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    let base = PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join("Library")
        .join("Application Support");
    #[cfg(not(target_os = "macos"))]
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share")
        });
    base.join(cdj3k_emu_platform::app_meta::BUNDLE_ID)
}

/// `app_data_dir()/instance-N`: the slot's settings and its firmware.
pub fn instance_dir(instance_id: u32) -> PathBuf {
    app_data_dir().join(format!("instance-{}", instance_id))
}

/// The files the install wizard provisions and QEMU boots from.
///
/// A slot holds one installation - the model it emulates - so installing
/// another model replaces these files.
#[derive(Clone, Debug)]
pub struct FirmwarePaths {
    /// Vanilla aarch64 kernel copied from the bundle.
    pub kernel: PathBuf,
    /// Pioneer rootfs with the emulation patches applied.
    pub initramfs: PathBuf,
    /// The eMMC qcow2 image.
    pub emmc: PathBuf,
}

/// The three boot artefacts, by name, for the whole-slot operations below.
pub(crate) const FIRMWARE_FILES: [&str; 3] = ["Image", "initramfs-patched.cpio.gz", "emmc.qcow2"];

impl FirmwarePaths {
    pub fn new(instance_id: u32) -> Self {
        let dir = instance_dir(instance_id);
        Self {
            kernel: dir.join(FIRMWARE_FILES[0]),
            initramfs: dir.join(FIRMWARE_FILES[1]),
            emmc: dir.join(FIRMWARE_FILES[2]),
        }
    }

    /// The directory holding the three files.
    pub fn dir(&self) -> &std::path::Path {
        self.emmc.parent().expect("emmc path has a parent")
    }

    /// Whether every boot artefact is present.
    pub fn provisioned(&self) -> bool {
        self.kernel.exists() && self.initramfs.exists() && self.emmc.exists()
    }

    /// Delete the slot's boot artefacts, including everything the guest wrote
    /// to its eMMC and a finished install waiting to replace them, so another
    /// model can be installed over them. Refused while an emulation holds the
    /// eMMC's `flock`.
    pub fn remove(&self) -> std::io::Result<()> {
        let _lock = lock_exclusive_retry(&self.emmc).map_err(|_| emulation_running())?;
        forget_release(self);
        staging::discard_staged(&self.dir().join(staging::STAGING))?;
        for path in [&self.kernel, &self.initramfs, &self.emmc] {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

fn emulation_running() -> std::io::Error {
    std::io::Error::other("the slot's emulation is running")
}

/// A process's claim on a slot: an exclusive `flock` on `instance-N/.open`,
/// held for as long as the process has the slot open and released by the OS
/// when it exits, however it exits. The file holds the holder's pid.
#[derive(Debug)]
pub struct SlotClaim {
    instance_id: u32,
    _file: std::fs::File,
}

impl SlotClaim {
    /// Claim `instance_id` for this process; `None` if another process holds
    /// it. A probe ([`slot_in_use`]) holds the lock for an instant, so a
    /// failed try is retried for a moment first.
    pub fn take(instance_id: u32) -> std::io::Result<Option<Self>> {
        let dir = instance_dir(instance_id);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(".open");
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)?;
        match lock_exclusive_retry(&path) {
            Ok(Some(mut file)) => {
                use std::io::{Seek, Write};
                file.set_len(0)?;
                file.rewind()?;
                write!(file, "{}", std::process::id())?;
                Ok(Some(Self {
                    instance_id,
                    _file: file,
                }))
            }
            Ok(None) => Err(std::io::Error::other(format!(
                "{} vanished",
                path.display()
            ))),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// The pid of the process holding `instance_id`'s [`SlotClaim`], if one does.
pub fn slot_holder(instance_id: u32) -> Option<u32> {
    let path = instance_dir(instance_id).join(".open");
    match lock_exclusive(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
            std::fs::read_to_string(&path).ok()?.trim().parse().ok()
        }
        _ => None,
    }
}

/// Whether another process has slot `instance_id` open ([`SlotClaim`]) or an
/// emulation holds its eMMC. This process's own claim counts: ask about
/// other slots.
pub fn slot_in_use(instance_id: u32) -> bool {
    let claim = instance_dir(instance_id).join(".open");
    lock_exclusive(&claim).is_err()
        || lock_exclusive(&FirmwarePaths::new(instance_id).emmc).is_err()
}

/// [`lock_exclusive`], tried again for up to a quarter of a second while
/// another process holds the lock. For taking a lock to keep it: every probe
/// holds one for an instant, and a single try could land on that instant.
fn lock_exclusive_retry(path: &std::path::Path) -> std::io::Result<Option<std::fs::File>> {
    let mut tries = 0;
    loop {
        match lock_exclusive(path) {
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && tries < 12 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            r => return r,
        }
    }
}

/// An exclusive `flock` on `path`, held until the returned file drops; `None`
/// when there is no file to lock. Fails with [`WouldBlock`] if another open
/// file holds it.
///
/// [`WouldBlock`]: std::io::ErrorKind::WouldBlock
fn lock_exclusive(path: &std::path::Path) -> std::io::Result<Option<std::fs::File>> {
    let f = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: `f` is an open file for the duration of the call.
        let rc = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                format!("{} is in use", path.display()),
            ));
        }
    }
    Ok(Some(f))
}

/// What slot `instance_id` holds: the deck installed in it and the firmware
/// release that installation came from.
///
/// `None` when the slot's boot artefacts are not all present - a slot records
/// the model it was given, and keeps the record after the files are deleted,
/// so the record alone does not say a slot is installed.
pub fn slot_summary(instance_id: u32) -> Option<(Model, Option<String>)> {
    if !FirmwarePaths::new(instance_id).provisioned() {
        return None;
    }
    let model = settings::InstanceSettings::saved_model(instance_id)?;
    Some((model, slot_release(instance_id)))
}

/// Remembers the releases read out of eMMC images, so a slot whose settings
/// predate the key costs one read per process rather than one per ask.
static ENV_RELEASE: std::sync::Mutex<std::collections::BTreeMap<u32, Option<String>>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

/// Drop what [`slot_release`] remembers of the slot `paths` belong to, its
/// eMMC having been replaced or removed.
fn forget_release(paths: &FirmwarePaths) {
    let mut cache = ENV_RELEASE.lock().expect("release cache");
    cache.retain(|&slot, _| FirmwarePaths::new(slot).emmc != paths.emmc);
}

/// The firmware release installed in `instance_id`.
///
/// The slot's own record when it has one; otherwise the `release` its eMMC's
/// U-Boot environment carries, which is where the installer has always put it.
/// That copy is not written back to the settings: another slot's file belongs
/// to that slot's process, which holds a lock this one cannot see.
pub fn slot_release(instance_id: u32) -> Option<String> {
    if let Some(release) = settings::InstanceSettings::saved_firmware_release(instance_id) {
        return Some(release);
    }
    let mut cache = ENV_RELEASE.lock().expect("release cache");
    if let Some(remembered) = cache.get(&instance_id) {
        return remembered.clone();
    }
    let release = emmc::read_uboot_env(&FirmwarePaths::new(instance_id).emmc)
        .and_then(|env| env.get("release").filter(|v| !v.is_empty()).cloned());
    cache.insert(instance_id, release.clone());
    release
}

/// Record a slot that holds firmware but no `model` as a CDJ-3000: the
/// only deck a slot held before the setting existed.
pub fn adopt_unrecorded_slot(instance_id: u32) {
    if settings::InstanceSettings::saved_model(instance_id).is_none()
        && FirmwarePaths::new(instance_id).provisioned()
    {
        let _ = settings::InstanceSettings::update(instance_id, |s| s.model = Some(Model::Cdj3k));
    }
}

/// A throwaway `HOME` for a test that reads or writes the app data dir.
///
/// Every path here derives from `HOME`, so the guard also serialises the
/// tests that use one: two at once would read each other's slots. The
/// directory goes when it drops.
#[cfg(test)]
pub(crate) struct TestHome {
    _lock: std::sync::MutexGuard<'static, ()>,
    path: PathBuf,
}

#[cfg(test)]
impl TestHome {
    pub(crate) fn new(tag: &str) -> Self {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = std::env::temp_dir().join(format!("cdj3k-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        std::env::set_var("HOME", &path);
        Self { _lock: lock, path }
    }
}

#[cfg(test)]
impl Drop for TestHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A slot with firmware and no recorded model is a CDJ-3000; an empty
    /// one stays unrecorded.
    #[test]
    fn an_unrecorded_slot_is_a_cdj3000() {
        let _home = TestHome::new("adopt");
        write_all(&FirmwarePaths::new(1), "cdj3k");
        adopt_unrecorded_slot(1);
        assert_eq!(
            settings::InstanceSettings::saved_model(1),
            Some(Model::Cdj3k)
        );
        adopt_unrecorded_slot(2);
        assert_eq!(settings::InstanceSettings::saved_model(2), None);
    }

    fn write_all(paths: &FirmwarePaths, tag: &str) {
        for path in [&paths.kernel, &paths.initramfs, &paths.emmc] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, tag).unwrap();
        }
    }

    fn read_all(paths: &FirmwarePaths) -> Vec<String> {
        [&paths.kernel, &paths.initramfs, &paths.emmc]
            .iter()
            .map(|p| std::fs::read_to_string(p).unwrap())
            .collect()
    }

    /// A slot another process has open, or whose eMMC an emulation holds,
    /// is in use; neither is once the holder lets go.
    #[test]
    fn a_slot_is_in_use_while_claimed_or_running() {
        let _home = TestHome::new("claim");
        assert!(!slot_in_use(2));

        let claim = SlotClaim::take(2).unwrap().expect("first claim");
        assert!(SlotClaim::take(2).unwrap().is_none(), "one claim at a time");
        assert!(slot_in_use(2));
        assert_eq!(slot_holder(2), Some(std::process::id()));
        drop(claim);
        assert!(!slot_in_use(2));
        assert_eq!(slot_holder(2), None);

        let live = FirmwarePaths::new(2);
        write_all(&live, "old");
        let qemu = lock_exclusive(&live.emmc).unwrap();
        assert!(slot_in_use(2));
        assert!(live.remove().is_err(), "a running slot's files stay");
        assert_eq!(read_all(&live), ["old", "old", "old"]);
        drop(qemu);
        live.remove().unwrap();
        assert!(!live.provisioned());
    }
}
