pub mod emmc;
pub mod gpt;
mod qcow2;
pub mod settings;

pub use emmc::{default_path, provision_emmc, EmmcConfig, FirmwareInfo};
pub use settings::{AppSettings, InstanceSettings};

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
const FIRMWARE_FILES: [&str; 3] = ["Image", "initramfs-patched.cpio.gz", "emmc.qcow2"];

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
    /// to its eMMC, so another model can be installed over them. QEMU holds
    /// the eMMC open, so the emulation has to be stopped first.
    pub fn remove(&self) -> std::io::Result<()> {
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

/// Serialises the tests that point `HOME` at a scratch directory: the paths
/// here are all derived from it, so two of them running at once would read
/// each other's slots.
#[cfg(test)]
pub(crate) fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A slot with firmware and no recorded model is a CDJ-3000; an empty
    /// one stays unrecorded.
    #[test]
    fn an_unrecorded_slot_is_a_cdj3000() {
        let _env = test_env_lock();
        let home = std::env::temp_dir().join(format!("cdj3k-adopt-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        std::env::set_var("HOME", &home);
        let root = instance_dir(1);
        std::fs::create_dir_all(&root).unwrap();
        for name in FIRMWARE_FILES {
            std::fs::write(root.join(name), b"cdj3k").unwrap();
        }
        adopt_unrecorded_slot(1);
        assert_eq!(
            settings::InstanceSettings::saved_model(1),
            Some(Model::Cdj3k)
        );
        adopt_unrecorded_slot(2);
        assert_eq!(settings::InstanceSettings::saved_model(2), None);
        let _ = std::fs::remove_dir_all(&home);
    }
}
