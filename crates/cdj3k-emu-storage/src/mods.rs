//! A slot's mods: folders with a `mod.toml` at their root ([`manifest`]),
//! which the guest loads once per boot before the player app (initramfs
//! patch 30).
//!
//! `instance-N/mods/` holds:
//!
//! - `list.txt`: the order, one mod per line, `on|off <name>\t<origin>` for an
//!   installed mod and `on|off <absolute path>` for a folder used in place
//! - `installed/<name>/`: an installed mod, unpacked from its archive
//! - `boot-initramfs.cpio.gz`: the slot's initramfs with the last boot's mods
//!   appended ([`boot`])
//! - `last-boot.txt`: the guest's mods report for the last boot, as the
//!   runtime received it, followed by an `id <boot id>` line for each slot
//!   mod that ran in the last boot ([`SlotMods::boot_id`])
//! - `last-boot/<name>.log`: each mod's journal from that boot

mod add;
mod archive;
pub mod boot;
pub mod compat;
mod list;
pub mod manifest;
mod report;
#[cfg(test)]
mod test_support;

use std::io;
use std::path::{Path, PathBuf};

use crate::instance_dir;

pub use add::PendingAdd;
pub use boot::{
    current_boot, planned_slot_run, prepare_boot, set_current_boot, slot_mods_on, BootMods,
};
pub use compat::Compat;
pub use list::{SlotMod, SlotMods, Source};
pub use manifest::{Manifest, FILE as MANIFEST, LOADER};
pub use report::{
    last_boot_ids, last_boot_log, last_boot_report, save_last_boot_log, save_last_boot_report,
};

const INSTALLED: &str = "installed";
/// The most mods a slot holds.
pub const MAX_MODS: usize = 10;

/// `instance-N/mods`.
pub fn mods_dir(instance_id: u32) -> PathBuf {
    instance_dir(instance_id).join("mods")
}

/// Write `text` to `path` through a temporary file, so a reader sees the old
/// file or the new one.
fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}
