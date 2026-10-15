//! The control API's `mods` commands. They edit the slot's mods list, as the
//! Mods view does; a change applies at the next boot, and `push` restarts a
//! running emulation for it.

use std::path::Path;

use cdj3k_emu_control::ModsCommand;
use cdj3k_emu_storage::mods::{self, manifest, SlotMods, Source};
use serde_json::{json, Value};

use super::{done, Answer};
use crate::app::shell::CdjShell;

impl CdjShell {
    pub(super) fn on_mods(&mut self, command: ModsCommand) -> Answer {
        let mut slot = SlotMods::load(self.instance);
        match command {
            ModsCommand::List => Ok(self.mods_list(&slot)),
            ModsCommand::Push(path) => self.push_mod(&mut slot, &path),
            ModsCommand::Enable(name, on) => {
                known(&slot, &name)?;
                slot.set_enabled(&name, on).map_err(|e| e.to_string())?;
                done()
            }
            ModsCommand::Remove(name) => {
                known(&slot, &name)?;
                slot.remove(&name).map_err(|e| e.to_string())?;
                done()
            }
            ModsCommand::Log(name) => {
                known(&slot, &name)?;
                let log = mods::last_boot_log(self.instance, &name)
                    .ok_or_else(|| format!("the last boot left no journal of mod {name}"))?;
                Ok(json!({ "ok": true, "log": log }))
            }
        }
    }

    fn mods_list(&self, slot: &SlotMods) -> Value {
        let list: Vec<Value> = slot
            .mods
            .iter()
            .map(|m| {
                let version = manifest::read(&slot.dir_of(m)).ok().map(|man| man.version);
                let (source, from) = match &m.source {
                    Source::Installed { origin } => ("installed", origin.clone()),
                    Source::Folder(path) => ("folder", path.display().to_string()),
                };
                json!({
                    "name": m.name,
                    "version": version,
                    "enabled": m.enabled,
                    "source": source,
                    "from": from,
                })
            })
            .collect();
        let booted = mods::current_boot().map_or_else(Vec::new, |b| b.run);
        json!({
            "ok": true,
            "mods": list,
            "mods_on": mods::slot_mods_on(self.instance),
            "booted": booted,
        })
    }

    /// Add or replace the mod at `path`, then restart a running emulation so
    /// that it boots with it.
    fn push_mod(&mut self, slot: &mut SlotMods, path: &Path) -> Answer {
        let add = slot.prepare(path, None).map_err(|e| e.to_string())?;
        let replaced = add.replaces.is_some();
        let version = add.manifest.version.clone();
        let name = slot.commit(add).map_err(|e| e.to_string())?;
        let restarted =
            !cdj3k_emu_runtime::worker_is_finished() && self.restart_from_api(false).is_ok();
        Ok(json!({
            "ok": true,
            "name": name,
            "version": version,
            "replaced": replaced,
            "restarted": restarted,
            "mods_on": mods::slot_mods_on(self.instance),
        }))
    }
}

fn known(slot: &SlotMods, name: &str) -> Result<(), String> {
    match slot.get(name) {
        Some(_) => Ok(()),
        None => Err(format!("this slot has no mod named {name:?}")),
    }
}
