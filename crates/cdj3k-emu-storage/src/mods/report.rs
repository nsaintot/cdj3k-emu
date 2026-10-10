//! What the guest reported about the slot's last boot: `last-boot.txt` and
//! the mods' journals in `last-boot/`.

use std::io;
use std::path::Path;

use super::{mods_dir, write_atomic};

pub(super) const LAST_BOOT: &str = "last-boot.txt";
pub(super) const LAST_BOOT_LOGS: &str = "last-boot";
/// The boot ids file of earlier versions, deleted when the next report is
/// saved.
const OLD_LAST_BOOT_IDS: &str = "last-boot-mods.txt";
const ID_LINE: &str = "id ";

/// The guest's mods report for the slot's last boot, as received.
pub fn last_boot_report(instance_id: u32) -> Option<String> {
    let text = std::fs::read_to_string(mods_dir(instance_id).join(LAST_BOOT)).ok()?;
    Some(
        text.lines()
            .filter(|l| !l.starts_with(ID_LINE))
            .map(|l| format!("{l}\n"))
            .collect(),
    )
}

/// Save `text` as the slot's last-boot report, with `ids`, the
/// [`SlotMods::boot_id`]s of the slot mods that ran in that boot, in the same
/// file.
/// The journals of the previous boot are deleted.
///
/// [`SlotMods::boot_id`]: super::SlotMods::boot_id
pub fn save_last_boot_report(instance_id: u32, text: &str, ids: &[String]) -> io::Result<()> {
    let dir = mods_dir(instance_id);
    std::fs::create_dir_all(&dir)?;
    let _ = std::fs::remove_dir_all(dir.join(LAST_BOOT_LOGS));
    let _ = std::fs::remove_file(dir.join(OLD_LAST_BOOT_IDS));
    let mut all: String = text.lines().map(|l| format!("{l}\n")).collect();
    for id in ids {
        all.push_str(&format!("{ID_LINE}{id}\n"));
    }
    write_atomic(&dir.join(LAST_BOOT), &all)
}

/// The [`SlotMods::boot_id`]s of the slot mods that the last report covers.
///
/// [`SlotMods::boot_id`]: super::SlotMods::boot_id
pub fn last_boot_ids(instance_id: u32) -> Vec<String> {
    std::fs::read_to_string(mods_dir(instance_id).join(LAST_BOOT))
        .map(|text| {
            text.lines()
                .filter_map(|l| l.strip_prefix(ID_LINE))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// `name`'s journal from the slot's last boot.
pub fn last_boot_log(instance_id: u32, name: &str) -> Option<String> {
    std::fs::read_to_string(
        mods_dir(instance_id)
            .join(LAST_BOOT_LOGS)
            .join(format!("{name}.log")),
    )
    .ok()
}

pub fn save_last_boot_log(instance_id: u32, name: &str, text: &str) -> io::Result<()> {
    let dir = mods_dir(instance_id).join(LAST_BOOT_LOGS);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(format!("{name}.log")), text)
}

/// Delete what the last boot reported about `name` from the slot's mods
/// folder `dir`: its report line, its boot id and its journal. Best effort:
/// a file left behind only causes a stale status.
pub(super) fn forget(dir: &Path, name: &str) {
    let keep = |file: &str, gone: &dyn Fn(&str) -> bool| {
        let path = dir.join(file);
        if let Ok(text) = std::fs::read_to_string(&path) {
            let kept: String = text
                .lines()
                .filter(|l| !gone(l))
                .map(|l| format!("{l}\n"))
                .collect();
            if kept != text {
                let _ = write_atomic(&path, &kept);
            }
        }
    };
    keep(LAST_BOOT, &|l| {
        let reported = l
            .strip_prefix("mod ")
            .and_then(|rest| rest.split_whitespace().next());
        let id = l.strip_prefix(ID_LINE).and_then(|id| id.split('\t').next());
        reported == Some(name) || id == Some(name)
    });
    let _ = std::fs::remove_file(dir.join(LAST_BOOT_LOGS).join(format!("{name}.log")));
}
