//! What the view shows, read from the slot: its mods, how each ran on the
//! last boot, and whether a restart would boot different ones.

use std::path::Path;

use cdj3k_emu_panel::Model;
use cdj3k_emu_runtime::mods_report::{ModOutcome, ModsReport, Script};
use cdj3k_emu_storage::mods::{self as slot_mods, Compat, Source};
use cdj3k_emu_storage::InstanceSettings;

use super::{LogView, ModRow, ModsBar, ModsPage, ModsScreen, RowStatus, Run};

/// Return each mod that `boot` left out because its files could not be read,
/// and why.
fn unpacked(boot: Option<&slot_mods::BootMods>) -> Vec<(String, String)> {
    boot.iter()
        .flat_map(|b| &b.incompatible)
        .filter_map(|(name, c)| match c {
            Compat::Invalid(e) => Some((name.clone(), e.clone())),
            _ => None,
        })
        .collect()
}

/// Return whether a restart of this window's emulation would boot different
/// mods than the running emulation booted. `slot` must be this window's slot,
/// because the running boot is read from this process. The window's reply to
/// `restart-pending` comes from this function.
pub(in crate::app) fn restart_pending(slot: u32) -> bool {
    let Some(model) = InstanceSettings::saved_model(slot) else {
        return false;
    };
    let current = slot_mods::current_boot();
    let unpacked = unpacked(current.as_ref());
    let no_mods = current.as_ref().is_some_and(|b| b.no_mods);
    let release = InstanceSettings::saved_firmware_release(slot);
    let mut planned = slot_mods::planned_slot_run(
        slot,
        model,
        release.as_deref(),
        slot_mods::slot_mods_on(slot) && !no_mods,
    );
    // Ignore mods that the current boot left out because their files could
    // not be read.
    planned.retain(|id| {
        let name = id.split('\t').next().unwrap_or_default();
        !unpacked.iter().any(|(n, _)| n == name)
    });
    current.map(|b| b.slot_run) != Some(planned)
}

impl ModsPage {
    /// Read the slot again. `own` is this window's slot, `running` is whether
    /// its emulation is up, and `elsewhere` is whether the viewed slot's
    /// emulation runs in another window.
    pub(in crate::app) fn refresh(&mut self, own: u32, running: bool, elsewhere: bool) {
        let current = (self.slot == own && running)
            .then(slot_mods::current_boot)
            .flatten();
        let slot = Slot::read(self.slot, &unpacked(current.as_ref()));
        let up = if self.slot == own { running } else { elsewhere };
        self.run = match slot.model {
            // When another window runs the slot, use that window's last answer
            // to `restart-pending`.
            Some(_) if up && self.slot != own => Run::Restart {
                pending: self.remote_pending,
            },
            Some(_) if up => Run::Restart {
                pending: restart_pending(self.slot),
            },
            Some(_) if self.slot != own => Run::Start,
            _ => Run::None,
        };
        self.rows = slot.rows;
        self.model = slot.model;
        self.release = slot.release;
        self.gate = slot.gate;
    }

    /// Show `name`'s page: its outcome on the last boot and its journal.
    pub(in crate::app) fn open_log(&mut self, name: &str) {
        let Some(row) = self.rows.iter().find(|r| r.name == name) else {
            return;
        };
        let report = last_report(self.slot);
        let ran = match &report {
            ModsReport::Done { outcomes, .. } => outcomes.as_slice(),
            _ => &[],
        };
        // Its position in the last boot, when the row's status comes from
        // that boot's report.
        let reported = matches!(
            row.status,
            RowStatus::Loaded | RowStatus::Declared(_) | RowStatus::Failed(_)
        );
        let at = ran.iter().position(|o| o.name == name).filter(|_| reported);
        self.log = Some(LogView {
            name: row.name.clone(),
            dev: row.dev,
            version: row.version.clone(),
            author: row.author.clone(),
            description: row.description.clone(),
            url: row.url.clone(),
            status: row.status.clone(),
            order: at.map(|i| (i + 1, ran.len())),
            libs: at.map_or(0, |i| ran[i].libs),
            text: slot_mods::last_boot_log(self.slot, name),
        });
        self.screen = ModsScreen::Log;
    }
}

pub(in crate::app) fn mods_bar(slot: u32) -> ModsBar {
    let s = Slot::read(slot, &[]);
    let incompatible = s
        .rows
        .iter()
        .filter(|r| matches!(r.status, RowStatus::Incompatible(_)))
        .count();
    let note = if !s.gate {
        Some(("disabled".to_string(), false))
    } else if incompatible > 0 {
        Some((format!("{incompatible} incompatible"), true))
    } else {
        None
    };
    ModsBar {
        total: s.rows.len(),
        on: s.rows.iter().filter(|r| r.enabled).count(),
        note,
    }
}

/// The slot's mods that would not run if `model` were installed in the slot.
pub(in crate::app) fn incompatible_with(slot: u32, model: Model) -> Vec<String> {
    let list = slot_mods::SlotMods::load(slot);
    list.mods
        .iter()
        .filter(|m| !slot_mods::manifest::check_dir(&list.dir_of(m), model, None).runs())
        .map(|m| m.name.clone())
        .collect()
}

/// A slot as the view reads it.
struct Slot {
    model: Option<Model>,
    release: Option<String>,
    gate: bool,
    rows: Vec<ModRow>,
}

/// What a row's status is read from.
struct RowSource<'a> {
    model: Option<Model>,
    release: Option<&'a str>,
    report: &'a ModsReport,
    /// The boot ids of the mods the last boot ran.
    ran: &'a [String],
    /// Mods the running boot left out because their files could not be read,
    /// with the reason.
    unpacked: &'a [(String, String)],
}

impl Slot {
    /// `unpacked` lists the mods that the running boot left out because their
    /// files could not be read, with the reason.
    fn read(slot: u32, unpacked: &[(String, String)]) -> Self {
        let model = InstanceSettings::saved_model(slot);
        let release = InstanceSettings::saved_firmware_release(slot);
        let gate = InstanceSettings::saved_mods_enabled(slot);
        let list = slot_mods::SlotMods::load(slot);
        let report = last_report(slot);
        let ran = slot_mods::last_boot_ids(slot);
        let source = RowSource {
            model,
            release: release.as_deref(),
            report: &report,
            ran: &ran,
            unpacked,
        };
        let rows = list.mods.iter().map(|m| source.row(&list, m)).collect();
        Self {
            model,
            release,
            gate,
            rows,
        }
    }
}

impl RowSource<'_> {
    fn row(&self, list: &slot_mods::SlotMods, m: &slot_mods::SlotMod) -> ModRow {
        let manifest = slot_mods::manifest::read(&list.dir_of(m));
        let env = manifest.as_ref().map_or(0, |man| man.env.len());
        let compat = self.model.map(|model| match &manifest {
            Ok(man) => man.check(model, self.release),
            Err(e) => Compat::Invalid(e.clone()),
        });
        let status = self.status(list, m, compat, env);
        let (source, dev) = match &m.source {
            Source::Installed { origin } => (origin.clone(), false),
            Source::Folder(path) => (home_relative(path), true),
        };
        let (version, author, description, url) =
            manifest.ok().map_or((None, None, None, None), |man| {
                (
                    Some(one_line(&man.version)),
                    man.author.as_deref().map(one_line),
                    man.description.as_deref().map(one_line),
                    man.url,
                )
            });
        ModRow {
            name: m.name.clone(),
            source,
            version,
            author,
            description,
            url,
            dev,
            enabled: m.enabled,
            status,
        }
    }

    /// Returns the row's status: why the slot's firmware stops the mod or, if
    /// it does not, how the mod ran on the last boot. `env` is the number of
    /// variables its `mod.toml` declares.
    fn status(
        &self,
        list: &slot_mods::SlotMods,
        m: &slot_mods::SlotMod,
        compat: Option<Compat>,
        env: usize,
    ) -> RowStatus {
        // The last report applies only when its boot ids include this
        // mod's: a mod re-added from another source or in another
        // version shows NOT RUN.
        let (outcome, unread) = match self.report {
            ModsReport::Done { outcomes, unread } if self.ran.contains(&list.boot_id(m)) => (
                outcomes.iter().find(|o| o.name == m.name),
                unread.iter().any(|u| u.name == m.name),
            ),
            _ => (None, false),
        };
        match (compat, self.model) {
            (Some(c @ Compat::Incompatible { .. }), Some(model)) => {
                RowStatus::Incompatible(incompatible_detail(&c, model, self.release))
            }
            (Some(Compat::Invalid(e)), _) => RowStatus::Invalid(e),
            _ => match outcome {
                Some(o) => outcome_status(o, env),
                None if unread => RowStatus::Unreadable,
                None => match self.unpacked.iter().find(|(n, _)| *n == m.name) {
                    Some((_, e)) => RowStatus::Invalid(e.clone()),
                    None => RowStatus::NotRun,
                },
            },
        }
    }
}

fn last_report(slot: u32) -> ModsReport {
    slot_mods::last_boot_report(slot)
        .map(|text| ModsReport::from_lines(text.lines()))
        .unwrap_or_default()
}

/// Returns the status for a mod's outcome on the last boot. `env` is the
/// number of variables the mod's `mod.toml` declares; a mod without a
/// loader.sh can pass only those.
fn outcome_status(o: &ModOutcome, env: usize) -> RowStatus {
    match o.script {
        Script::Exit(0) => RowStatus::Loaded,
        Script::Exit(rc) => RowStatus::Failed(format!("exited {rc}")),
        Script::Timeout => RowStatus::Failed("stopped at 60 s".into()),
        Script::Missing => RowStatus::Declared(declared_detail(o.libs, env)),
    }
}

/// Describes what a mod without a loader.sh passed to the app, such as
/// `env loaded` or `2 libraries and env loaded`.
fn declared_detail(libs: u32, env: usize) -> String {
    let libs = match libs {
        0 => None,
        1 => Some("library".to_string()),
        n => Some(format!("{n} libraries")),
    };
    match (libs, env > 0) {
        (None, false) => "nothing to load".into(),
        (None, true) => "env loaded".into(),
        (Some(l), false) => format!("{l} loaded"),
        (Some(l), true) => format!("{l} and env loaded"),
    }
}

/// Formats the slot's firmware, then the firmware the mod declares, such as
/// `3.22/CDJ-3000 → ≤3.20/CDJ-3000`.
fn incompatible_detail(compat: &Compat, model: Model, release: Option<&str>) -> String {
    let current = match release {
        Some(r) => format!("{r}/{}", model.title()),
        None => model.title().to_string(),
    };
    let Compat::Incompatible { wanted, models } = compat else {
        return current;
    };
    let wants = match wanted {
        Some(range) => format!("{}/{}", pretty_range(range), model.title()),
        None if models.is_empty() => "mod.toml does not parse".to_string(),
        None => models
            .iter()
            .filter_map(|slug| Model::parse(slug))
            .map(|m| m.title())
            .collect::<Vec<_>>()
            .join(", "),
    };
    format!("{current} → {wants}")
}

fn pretty_range(range: &str) -> String {
    if range.trim() == "*" {
        return "any".into();
    }
    range
        .split(',')
        .map(|c| {
            c.trim()
                .replace(">=", "≥")
                .replace("<=", "≤")
                .replace("==", "")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Returns `text` on one line: each run of whitespace or control characters
/// becomes one space.
pub(super) fn one_line(text: &str) -> String {
    text.split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Returns `path` with the home directory written as `~`.
fn home_relative(path: &Path) -> String {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    match home.and_then(|h| path.strip_prefix(h).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mod_without_loader_says_what_it_passed() {
        assert_eq!(declared_detail(0, 1), "env loaded");
        assert_eq!(declared_detail(1, 0), "library loaded");
        assert_eq!(declared_detail(2, 3), "2 libraries and env loaded");
        assert_eq!(declared_detail(0, 0), "nothing to load");
    }
}
