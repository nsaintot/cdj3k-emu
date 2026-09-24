//! A slot's next installation, written beside the live one and swapped in by
//! the process that owns the slot.
//!
//! An install writes `instance-N/.staging/` while holding an exclusive `flock`
//! on `.staging/.lock`, and once every file is in place records what it
//! installed in `.staging/complete`. The record is the signal: the slot's
//! owner ([`SlotClaim`]) swaps the files in with [`apply_staged`] whenever the
//! slot's emulation is not running, so an install never has to stop one.
//!
//! Anyone may ask the owner to restart onto a finished install
//! ([`request_restart`]): that leaves `.staging/restart` beside the record,
//! and the owner restarts its emulation when it sees it.
//!
//! The lock is the `flock`, not the file: the OS drops it with the process
//! however the process ends, and the empty `.lock` file stays. A staging dir
//! whose lock nobody holds and which has no record is an install that died
//! part way, and is emptied by whoever finds it.

use std::io;
use std::path::{Path, PathBuf};

use cdj3k_emu_panel::Model;

use crate::{
    emulation_running, forget_release, instance_dir, lock_exclusive, lock_exclusive_retry,
    settings::InstanceSettings, FirmwarePaths, SlotClaim, FIRMWARE_FILES,
};

pub(crate) const STAGING: &str = ".staging";
const LOCK: &str = ".lock";
const RECORD: &str = "complete";
const RESTART: &str = "restart";

fn staging_dir(instance_id: u32) -> PathBuf {
    instance_dir(instance_id).join(STAGING)
}

fn staged_paths(dir: &Path) -> FirmwarePaths {
    FirmwarePaths {
        kernel: dir.join(FIRMWARE_FILES[0]),
        initramfs: dir.join(FIRMWARE_FILES[1]),
        emmc: dir.join(FIRMWARE_FILES[2]),
    }
}

/// What a finished install put in a slot. [`apply_staged`] writes it into the
/// slot's settings along with the files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedRecord {
    pub model: Model,
    pub firmware_release: Option<String>,
    /// The serial the staged eMMC's cabinet was keyed for.
    pub soc_serial: String,
    /// Someone asked the owner to restart onto it ([`request_restart`]).
    pub restart: bool,
}

fn read_record(dir: &Path) -> Option<StagedRecord> {
    let path = dir.join(RECORD);
    let text = std::fs::read_to_string(&path).ok()?;
    let mut model = None;
    let mut firmware_release = None;
    let mut soc_serial = None;
    for line in text.lines() {
        match line.split_once('=') {
            Some(("model", v)) => model = Model::parse(v),
            Some(("firmware_release", v)) if !v.is_empty() => firmware_release = Some(v.to_owned()),
            Some(("soc_serial", v)) => soc_serial = InstanceSettings::parse_soc_serial(v).ok(),
            _ => {}
        }
    }
    Some(StagedRecord {
        model: model?,
        firmware_release,
        soc_serial: soc_serial?,
        restart: dir.join(RESTART).exists(),
    })
}

/// Delete everything in `dir` but the lock file.
fn clear(dir: &Path) -> io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_name() == LOCK {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// An install under way into a slot's staging dir. Dropping it before
/// [`finish`](Self::finish) deletes what it wrote.
#[derive(Debug)]
pub struct StagedFirmware {
    _lock: std::fs::File,
    /// Where the install writes: `instance-N/.staging/`.
    pub paths: FirmwarePaths,
    finished: bool,
}

impl StagedFirmware {
    /// An empty staging dir for `instance_id`, held by this install until it
    /// finishes or drops. Fails while another install into the slot runs.
    /// Whatever an earlier install left is deleted, including one finished
    /// but never applied.
    pub fn new(instance_id: u32) -> io::Result<Self> {
        let dir = staging_dir(instance_id);
        std::fs::create_dir_all(&dir)?;
        let lock_path = dir.join(LOCK);
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)?;
        let lock = lock_exclusive_retry(&lock_path)
            .map_err(|_| {
                io::Error::other(format!(
                    "another install into slot {instance_id} is running"
                ))
            })?
            .ok_or_else(|| io::Error::other(format!("{} vanished", lock_path.display())))?;
        // A record with a file already moved is a swap cut short: the slot's
        // settings name this install, so it has to be finished, not replaced.
        if dir.join(RECORD).exists() && !staged_paths(&dir).provisioned() {
            return Err(io::Error::other(format!(
                "slot {instance_id}'s last install is part way swapped in; start the slot \
                 to finish it"
            )));
        }
        clear(&dir)?;
        Ok(Self {
            _lock: lock,
            paths: staged_paths(&dir),
            finished: false,
        })
    }

    /// Record the install as complete, for the slot's owner to apply. Fails,
    /// and records nothing, if a file was not written.
    pub fn finish(
        mut self,
        model: Model,
        firmware_release: Option<&str>,
        soc_serial: &str,
    ) -> io::Result<()> {
        for path in [&self.paths.kernel, &self.paths.initramfs, &self.paths.emmc] {
            let f = std::fs::File::open(path).map_err(|e| {
                io::Error::other(format!("{} was not written: {e}", path.display()))
            })?;
            f.sync_all()?;
        }
        let dir = self.paths.dir().to_path_buf();
        let tmp = dir.join(format!("{RECORD}.tmp"));
        let text = format!(
            "model={}\nfirmware_release={}\nsoc_serial={}\n",
            model.slug(),
            firmware_release.unwrap_or(""),
            InstanceSettings::parse_soc_serial(soc_serial)?,
        );
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, dir.join(RECORD))?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for StagedFirmware {
    fn drop(&mut self) {
        if !self.finished {
            let _ = clear(self.paths.dir());
        }
    }
}

/// The finished install waiting in `instance_id`'s staging dir, if there is
/// one. `None` while an install is still writing it. Leftovers of an install
/// that died part way are deleted.
pub fn pending_install(instance_id: u32) -> Option<StagedRecord> {
    let dir = staging_dir(instance_id);
    // Held for the read, so an install starting now cannot clear the record
    // from under it.
    let _held = lock_exclusive(&dir.join(LOCK)).ok()??;
    let record = read_record(&dir);
    if record.is_none() {
        let _ = clear(&dir);
    }
    record
}

/// Ask the owner of `instance_id` to restart its emulation onto the finished
/// install waiting there. `false` when there is none.
pub fn request_restart(instance_id: u32) -> io::Result<bool> {
    let dir = staging_dir(instance_id);
    let Ok(Some(_held)) = lock_exclusive_retry(&dir.join(LOCK)) else {
        return Ok(false);
    };
    if !dir.join(RECORD).exists() {
        return Ok(false);
    }
    std::fs::write(dir.join(RESTART), "")?;
    Ok(true)
}

/// Swap the slot's finished install in and record it in the slot's settings;
/// `None` when there is none. Only the slot's owner may, and only while its
/// emulation is not running: the running QEMU's eMMC is the file replaced.
pub fn apply_staged(claim: &SlotClaim) -> io::Result<Option<StagedRecord>> {
    let instance_id = claim.instance_id;
    let dir = staging_dir(instance_id);
    let Ok(Some(_held)) = lock_exclusive_retry(&dir.join(LOCK)) else {
        // Nothing staged, or an install still writing.
        return Ok(None);
    };
    let Some(record) = read_record(&dir) else {
        clear(&dir)?;
        return Ok(None);
    };
    let live = FirmwarePaths::new(instance_id);
    let _emmc = lock_exclusive_retry(&live.emmc).map_err(|_| emulation_running())?;
    // The settings go first: a failure here leaves the slot as it was, and a
    // failed rename below is picked up again on the next try.
    InstanceSettings::update(instance_id, |s| {
        s.model = Some(record.model);
        s.firmware_release = record.firmware_release.clone();
        s.soc_serial = record.soc_serial.clone();
    })?;
    let staged = staged_paths(&dir);
    for (from, to) in [
        (&staged.kernel, &live.kernel),
        (&staged.initramfs, &live.initramfs),
        (&staged.emmc, &live.emmc),
    ] {
        // The record is written only once all three exist, so a missing one
        // was moved by an earlier try.
        if from.exists() {
            std::fs::rename(from, to)?;
        }
    }
    forget_release(&live);
    clear(&dir)?;
    Ok(Some(record))
}

/// Drop a finished install nobody has applied, so an emptied slot stays
/// empty. An install still writing is left to finish.
pub(crate) fn discard_staged(dir: &Path) -> io::Result<()> {
    match lock_exclusive_retry(&dir.join(LOCK)) {
        Ok(Some(_held)) => clear(dir),
        Ok(None) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestHome;

    const SERIAL: &str = "0123456789abcd05";

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

    fn stage(slot: u32, tag: &str) {
        let staged = StagedFirmware::new(slot).unwrap();
        write_all(&staged.paths, tag);
        staged.finish(Model::Cdj3kx, Some("1.40"), SERIAL).unwrap();
    }

    /// A finished install waits for the owner, who swaps every file in and
    /// records the model, release and serial with them.
    #[test]
    fn the_owner_applies_a_finished_install() {
        let _home = TestHome::new("stage-apply");
        let live = FirmwarePaths::new(1);
        write_all(&live, "old");
        stage(1, "new");

        assert_eq!(
            read_all(&live),
            ["old", "old", "old"],
            "staging leaves the slot alone"
        );
        let pending = pending_install(1).expect("a finished install is pending");
        assert_eq!(pending.model, Model::Cdj3kx);
        assert!(!pending.restart);
        assert!(request_restart(1).unwrap());
        assert!(
            pending_install(1).unwrap().restart,
            "the owner sees the request"
        );

        let claim = SlotClaim::take(1).unwrap().unwrap();
        let applied = apply_staged(&claim).unwrap().expect("applied");
        assert_eq!(applied.firmware_release.as_deref(), Some("1.40"));
        assert_eq!(read_all(&live), ["new", "new", "new"]);
        let s = InstanceSettings::load_or_init(1);
        assert_eq!(s.model, Some(Model::Cdj3kx));
        assert_eq!(s.soc_serial, SERIAL);
        assert!(pending_install(1).is_none());
        assert!(apply_staged(&claim).unwrap().is_none(), "applied once");
        assert!(!request_restart(1).unwrap(), "nothing left to restart onto");
    }

    /// An install dropped before it finished, or still writing, is nothing
    /// to apply.
    #[test]
    fn an_unfinished_install_is_never_applied() {
        let _home = TestHome::new("stage-drop");
        let live = FirmwarePaths::new(1);
        write_all(&live, "old");
        let claim = SlotClaim::take(1).unwrap().unwrap();

        let staged = StagedFirmware::new(1).unwrap();
        write_all(&staged.paths, "new");
        assert!(pending_install(1).is_none(), "still writing");
        assert!(apply_staged(&claim).unwrap().is_none());
        assert!(StagedFirmware::new(1).is_err(), "one install at a time");
        let dir = staged.paths.dir().to_path_buf();
        drop(staged);

        assert!(
            !dir.join(FIRMWARE_FILES[2]).exists(),
            "its files go with it"
        );
        assert!(apply_staged(&claim).unwrap().is_none());
        assert_eq!(read_all(&live), ["old", "old", "old"]);
    }

    /// Files an install left without a record - it died part way - are
    /// deleted, and the lock it held is free.
    #[test]
    fn a_dead_install_leaves_nothing_behind() {
        let _home = TestHome::new("stage-dead");
        let dir = staging_dir(1);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(LOCK), "").unwrap();
        write_all(&staged_paths(&dir), "half");

        assert!(pending_install(1).is_none());
        assert!(!dir.join(FIRMWARE_FILES[2]).exists());
        drop(StagedFirmware::new(1).expect("the lock is free"));
    }

    /// An install missing a file records nothing; one over a running
    /// emulation waits until it has stopped.
    #[test]
    fn an_install_waits_for_the_emulation() {
        let _home = TestHome::new("stage-running");
        let staged = StagedFirmware::new(1).unwrap();
        std::fs::write(&staged.paths.kernel, "new").unwrap();
        assert!(staged.finish(Model::Cdj3k, None, SERIAL).is_err());
        assert!(pending_install(1).is_none());

        let live = FirmwarePaths::new(1);
        write_all(&live, "old");
        stage(1, "new");
        let claim = SlotClaim::take(1).unwrap().unwrap();
        let qemu = lock_exclusive(&live.emmc).unwrap();
        assert!(apply_staged(&claim).is_err());
        assert_eq!(read_all(&live), ["old", "old", "old"]);
        assert!(pending_install(1).is_some(), "still waiting");
        drop(qemu);
        apply_staged(&claim).unwrap().expect("applied once stopped");
        assert_eq!(read_all(&live), ["new", "new", "new"]);
    }

    /// A first install has no live eMMC to lock.
    #[test]
    fn an_install_into_an_empty_slot_applies() {
        let _home = TestHome::new("stage-empty");
        stage(1, "new");
        let claim = SlotClaim::take(1).unwrap().unwrap();
        apply_staged(&claim).unwrap().unwrap();
        assert_eq!(read_all(&FirmwarePaths::new(1)), ["new", "new", "new"]);
    }

    /// A swap cut short between two renames is finished by the next apply,
    /// and no new install may clear it first.
    #[test]
    fn a_half_applied_install_is_finished_not_replaced() {
        let _home = TestHome::new("stage-half");
        let live = FirmwarePaths::new(1);
        write_all(&live, "old");
        stage(1, "new");
        let dir = staging_dir(1);
        std::fs::rename(dir.join(FIRMWARE_FILES[0]), &live.kernel).unwrap();

        assert!(StagedFirmware::new(1).is_err());
        let claim = SlotClaim::take(1).unwrap().unwrap();
        apply_staged(&claim).unwrap().expect("finished");
        assert_eq!(read_all(&live), ["new", "new", "new"]);
        drop(StagedFirmware::new(1).expect("free again"));
    }
}
