//! The setup window's Mods view: a slot's mods in boot order, the result of
//! the last boot for each, and each mod's journal.
//!
//! The view draws a [`ModsPage`] that the shell fills from the slot's storage,
//! and returns the user's request as a [`ModsAction`]. It writes nothing to
//! disk itself.

use std::path::{Path, PathBuf};

use cdj3k_emu_panel::Model;
use cdj3k_emu_runtime::mods_report::{ModOutcome, ModsReport, Script};
use cdj3k_emu_storage::mods::{self as slot_mods, Compat, Source};
use cdj3k_emu_storage::InstanceSettings;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Vec2};

use super::picker::{
    draw_header, footer_split, hairline, k_of, text_width, tracked, trash_glyph, Header,
};
use super::theme::{self, Palette};

mod dialogs;
mod list;
mod log_page;
mod pieces;

use dialogs::*;
use list::*;
use log_page::*;
use pieces::*;
pub(in crate::app) use pieces::{cube_glyph, dashed_rect};

pub(in crate::app) const DOCS_URL: &str =
    "https://github.com/nsaintot/cdj3k-emu/blob/main/docs/mods.md";
pub(in crate::app) const TEMPLATE_URL: &str =
    "https://github.com/nsaintot/cdj3k-emu/tree/main/docs/mods/example";

const LIST_SUB: &str = "Enabled mods run in list order at each boot, before the player app starts";
const EMPTY_SUB: &str = "No mods in this slot.";
const LOG_SUB: &str = "Details and script output from the last boot";

// ── Toolbar ───────────────────────────────────────────────────────────────────
const TOOLBAR_H: f32 = 32.0;
const TOOLBAR_GAP: f32 = 10.0;
const TOOL_PAD: f32 = 14.0;
const TOOL_GLYPH: f32 = 11.0;
const TOOL_GLYPH_GAP: f32 = 7.0;
const TOOL_GAP: f32 = 8.0;
const NOTE_FONT: f32 = 12.0;
const NOTE_DOT: f32 = 7.0;
const NOTE_DOT_GAP: f32 = 8.0;

// ── Table ─────────────────────────────────────────────────────────────────────
const HEAD_H: f32 = 32.0;
const HEAD_TRACK: f32 = 1.4;
const ROW_H: f32 = 54.0;
const ROW_PAD: f32 = 12.0;
const COL_ON: f32 = 60.0;
const COL_STATUS: f32 = 210.0;
const HANDLE_W: f32 = 18.0;
const HANDLE_H: f32 = 28.0;
const ON_GAP: f32 = 10.0;
const CHECK: f32 = 15.0;
const NAME_FONT: f32 = 13.5;
const MONO_FONT: f32 = 10.5;
const LINE_GAP: f32 = 3.0;
const STATUS_FONT: f32 = 9.0;
const STATUS_TRACK: f32 = 1.26;
const CHIP_H: f32 = 16.0;
const CHIP_PAD: f32 = 5.0;
const CHIP_FONT: f32 = 8.5;
const CHIP_TRACK: f32 = 1.2;
const CHIP_GAP: f32 = 8.0;
const ACTION: [f32; 2] = [40.0, 32.0];
const ACTION_GAP: f32 = 8.0;
const ACTION_GLYPH: f32 = 14.0;

// ── Empty state ───────────────────────────────────────────────────────────────
const ZONE_BOTTOM: f32 = 20.0;
const ZONE_LINKS_GAP: f32 = 14.0;
const ZONE_GLYPH: f32 = 28.0;
const ZONE_TITLE_FONT: f32 = 15.0;
const ZONE_SUB_FONT: f32 = 12.5;
const DASH: f32 = 4.0;
const DASH_GAP: f32 = 3.0;
const LINK_FONT: f32 = 13.0;
const LINK_GLYPH: f32 = 14.0;
const LINK_GAP: f32 = 24.0;
const LINKS_H: f32 = 20.0;

// ── Modals ────────────────────────────────────────────────────────────────────
const MODAL_W: f32 = 460.0;
/// Where the dialogs sit: their top, as a fraction of the window height.
const MODAL_TOP: f32 = 0.287;
const MODAL_PAD_X: f32 = 22.0;
const MODAL_PAD_TOP: f32 = 20.0;
const MODAL_TEXT_GAP: f32 = 8.0;
const MODAL_BODY_BOTTOM: f32 = 16.0;
const MODAL_BAR_PAD: f32 = 12.0;
const MODAL_TITLE_FONT: f32 = 18.0;
const URL_FONT: f32 = 12.0;
const URL_MARGIN: f32 = 11.0;
const MODAL_SCRIM: Color32 = Color32::from_rgba_premultiplied(8, 8, 9, 87);

// ── Log ───────────────────────────────────────────────────────────────────────
const PILL_H: f32 = 22.0;
const PILL_PAD: f32 = 8.0;
const DL_H: f32 = 50.0;
const DL_PAD: f32 = 14.0;
const DL_VALUE_FONT: f32 = 12.0;
const META_ROW: f32 = 24.0;
const META_PAD: f32 = 8.0;
const META_LABEL: f32 = 110.0;
const SECTION_GAP: f32 = 12.0;
const LOG_FONT: f32 = 11.0;
const LOG_PAD: f32 = 12.0;
const LOG_BOTTOM: f32 = 16.0;
const COPY_PAD: f32 = 32.0;
const BACK_W: f32 = 100.0;

/// What happened to a mod on the last boot, or why it did not run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum RowStatus {
    /// Not in the last boot's report.
    NotRun,
    /// Its loader.sh exited 0.
    Loaded,
    /// No loader.sh: what its `mod.toml` passed to the app, e.g. `env loaded`.
    Declared(String),
    /// How its loader.sh ended: `exited 1`, `stopped at 60 s`.
    Failed(String),
    /// The slot's firmware, then the firmware the mod declares, e.g.
    /// `3.22/CDJ-3000 → ≤3.20/CDJ-3000`.
    Incompatible(String),
    /// No usable `mod.toml`, and why.
    Invalid(String),
    /// The last boot reported it in a format this app cannot read.
    Unreadable,
}

impl RowStatus {
    fn word(&self) -> &'static str {
        match self {
            RowStatus::NotRun => "NOT RUN",
            RowStatus::Loaded | RowStatus::Declared(_) => "LOADED",
            RowStatus::Failed(_) => "FAILED",
            RowStatus::Incompatible(_) => "INCOMPATIBLE",
            RowStatus::Invalid(_) => "INVALID",
            RowStatus::Unreadable => "UNKNOWN",
        }
    }

    fn detail(&self) -> String {
        match self {
            RowStatus::NotRun => "not part of the last boot".into(),
            RowStatus::Loaded => "script ran successfully".into(),
            RowStatus::Failed(d) => format!("script failed: {d}"),
            // A parse error runs over several lines; the row shows the first.
            RowStatus::Declared(d) | RowStatus::Incompatible(d) | RowStatus::Invalid(d) => {
                one_line(d.lines().next().unwrap_or_default())
            }
            RowStatus::Unreadable => "unrecognized report".into(),
        }
    }

    fn tone(&self, pal: &Palette) -> Color32 {
        match self {
            RowStatus::NotRun => pal.faint,
            RowStatus::Loaded | RowStatus::Declared(_) => pal.ok,
            RowStatus::Unreadable => pal.warn,
            _ => pal.danger,
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::app) struct ModRow {
    pub name: String,
    /// Where it comes from: the archive or URL, or the folder used in place.
    pub source: String,
    /// Its `mod.toml` version, when the file can be read.
    pub version: Option<String>,
    /// Its `mod.toml` author, when it names one.
    pub author: Option<String>,
    /// Its `mod.toml` description, shown on hovering its name.
    pub description: Option<String>,
    /// The project's web page, from the `url` key of mod.toml.
    pub url: Option<String>,
    /// A folder used in place: ejected, never deleted.
    pub dev: bool,
    pub enabled: bool,
    pub status: RowStatus,
}

/// A mod's log page: its description, version, author and project page, how
/// it ran on the last boot, and its journal.
#[derive(Clone, Debug)]
pub(in crate::app) struct LogView {
    pub name: String,
    pub dev: bool,
    pub version: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
    pub status: RowStatus,
    /// Its position among the mods the last boot ran, and how many it ran.
    pub order: Option<(usize, usize)>,
    /// Libraries it asked for with `mod-preload`.
    pub libs: u32,
    /// The journal, as cfgd sent it; `None` when there is none.
    pub text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum ModsScreen {
    List,
    Log,
}

/// Everything the view shows, filled by the shell.
pub(in crate::app) struct ModsPage {
    pub slot: u32,
    pub model: Option<Model>,
    pub release: Option<String>,
    /// The slot's "Enable Mods".
    pub gate: bool,
    pub rows: Vec<ModRow>,
    /// The footer's emulation button.
    pub run: Run,
    pub screen: ModsScreen,
    pub log: Option<LogView>,
    /// The mod the remove dialog asks about.
    pub remove: Option<String>,
    /// The mod a same-name add would replace: name, the version in the list,
    /// and the one coming in.
    pub replace: Option<(String, String, String)>,
    /// The URL dialog's text, while it is open.
    pub url: Option<String>,
    /// A line shown instead of the mod count: the progress of an add, or why
    /// it failed.
    pub note: Option<(String, bool)>,
}

/// What the footer offers for the viewed slot's emulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum Run {
    /// The slot is running, here or in its own window; `pending` when the
    /// list has changed since it booted.
    Restart { pending: bool },
    /// Another slot, installed and not running: opens its window.
    Start,
    /// No button: the slot has no firmware, or this window's emulation is
    /// stopped.
    None,
}

pub(in crate::app) enum ModsAction {
    Back,
    Close,
    Restart,
    /// Start the viewed slot in its own window.
    Start,
    Toggle(String),
    Reorder(usize, usize),
    AskRemove(String),
    Remove(String),
    CancelRemove,
    Eject(String),
    Add,
    AskUrl,
    Replace,
    KeepOld,
    AddUrl(String),
    CancelUrl,
    OpenLog(String),
    CloseLog,
    /// Open a web page.
    Open(String),
    /// Files or folders dropped on the window.
    Dropped(Vec<PathBuf>),
}

// ── What the view shows, read from the slot ───────────────────────────────────

impl ModsPage {
    pub(in crate::app) fn new(slot: u32) -> Self {
        Self {
            slot,
            model: None,
            release: None,
            gate: true,
            rows: Vec::new(),
            run: Run::None,
            screen: ModsScreen::List,
            log: None,
            remove: None,
            replace: None,
            url: None,
            note: None,
        }
    }

    /// Read the slot again. `own` is this window's slot and `running` whether
    /// its emulation is up; `elsewhere` whether the viewed slot's emulation
    /// runs in another window.
    pub(in crate::app) fn refresh(&mut self, own: u32, running: bool, elsewhere: bool) {
        // This window's boot, with the mods it left out because their files
        // could not be read.
        let current = (self.slot == own && running)
            .then(slot_mods::current_boot)
            .flatten();
        let unpacked: Vec<(String, String)> = current
            .iter()
            .flat_map(|b| &b.incompatible)
            .filter_map(|(name, c)| match c {
                Compat::Invalid(e) => Some((name.clone(), e.clone())),
                _ => None,
            })
            .collect();
        let slot = Slot::read(self.slot, &unpacked);
        let up = if self.slot == own { running } else { elsewhere };
        self.run = match slot.model {
            Some(model) if up => {
                let no_mods = current.as_ref().is_some_and(|b| b.no_mods);
                let mut planned = slot_mods::planned_slot_run(
                    self.slot,
                    model,
                    slot.release.as_deref(),
                    slot_mods::slot_mods_on(self.slot) && !no_mods,
                );
                planned.retain(|id| {
                    let name = id.split('\t').next().unwrap_or_default();
                    !unpacked.iter().any(|(n, _)| n == name)
                });
                // For another window's slot, the last report says what it
                // booted.
                let booted = if self.slot == own {
                    slot_mods::current_boot().map(|b| b.slot_run)
                } else {
                    Some(slot_mods::last_boot_ids(self.slot))
                };
                Run::Restart {
                    pending: booted != Some(planned),
                }
            }
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

/// The Mods bar on Manage Emulation: how many mods are on, and whether the
/// slot's mods are disabled or some are incompatible.
pub(in crate::app) struct ModsBar {
    pub total: usize,
    pub on: usize,
    /// The note, and whether it is a failure rather than a caution.
    pub note: Option<(String, bool)>,
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

impl Slot {
    /// `unpacked`: mods the running boot left out because their files could
    /// not be read, with the reason.
    fn read(slot: u32, unpacked: &[(String, String)]) -> Self {
        let model = InstanceSettings::saved_model(slot);
        let release = InstanceSettings::saved_firmware_release(slot);
        let gate = InstanceSettings::saved_mods_enabled(slot);
        let list = slot_mods::SlotMods::load(slot);
        let report = last_report(slot);
        let ran = slot_mods::last_boot_ids(slot);
        let rows = list
            .mods
            .iter()
            .map(|m| {
                let manifest = slot_mods::manifest::read(&list.dir_of(m));
                let env = manifest.as_ref().map_or(0, |man| man.env.len());
                let compat = model.map(|model| match &manifest {
                    Ok(man) => man.check(model, release.as_deref()),
                    Err(e) => Compat::Invalid(e.clone()),
                });
                // The last report applies only when its boot ids include this
                // mod's: a mod re-added from another source or in another
                // version shows NOT RUN.
                let (outcome, unread) = match &report {
                    ModsReport::Done { outcomes, unread } if ran.contains(&list.boot_id(m)) => (
                        outcomes.iter().find(|o| o.name == m.name),
                        unread.iter().any(|u| u.name == m.name),
                    ),
                    _ => (None, false),
                };
                let status = match (compat, model) {
                    (Some(c @ Compat::Incompatible { .. }), Some(model)) => {
                        RowStatus::Incompatible(incompatible_detail(&c, model, release.as_deref()))
                    }
                    (Some(Compat::Invalid(e)), _) => RowStatus::Invalid(e),
                    _ => match outcome {
                        Some(o) => outcome_status(o, env),
                        None if unread => RowStatus::Unreadable,
                        None => match unpacked.iter().find(|(n, _)| *n == m.name) {
                            Some((_, e)) => RowStatus::Invalid(e.clone()),
                            None => RowStatus::NotRun,
                        },
                    },
                };
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
            })
            .collect();
        Self {
            model,
            release,
            gate,
            rows,
        }
    }
}

fn last_report(slot: u32) -> ModsReport {
    slot_mods::last_boot_report(slot)
        .map(|text| ModsReport::from_lines(text.lines()))
        .unwrap_or_default()
}

/// `env` is the number of variables the mod's `mod.toml` declares; a mod
/// without a loader.sh can pass only those.
fn outcome_status(o: &ModOutcome, env: usize) -> RowStatus {
    match o.script {
        Script::Exit(0) => RowStatus::Loaded,
        Script::Exit(rc) => RowStatus::Failed(format!("exited {rc}")),
        Script::Timeout => RowStatus::Failed("stopped at 60 s".into()),
        Script::Missing => RowStatus::Declared(declared_detail(o.libs, env)),
    }
}

/// `env loaded`, `2 libraries + env loaded`: what a mod without a loader.sh
/// passed to the app.
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
        (Some(l), true) => format!("{l} + env loaded"),
    }
}

/// `3.22/CDJ-3000 → ≤3.20/CDJ-3000`: the slot's firmware, then what the mod
/// declares.
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

/// `text` on one line: each run of whitespace or control characters becomes
/// one space.
pub(super) fn one_line(text: &str) -> String {
    text.split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `path` with the home directory written as `~`.
fn home_relative(path: &Path) -> String {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    match home.and_then(|h| path.strip_prefix(h).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

pub(in crate::app) fn draw_mods(ui: &mut egui::Ui, page: &mut ModsPage) -> Option<ModsAction> {
    match page.screen {
        ModsScreen::Log => draw_log(ui, page),
        ModsScreen::List => draw_list(ui, page),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mod_without_loader_says_what_it_passed() {
        assert_eq!(declared_detail(0, 1), "env loaded");
        assert_eq!(declared_detail(1, 0), "library loaded");
        assert_eq!(declared_detail(2, 3), "2 libraries + env loaded");
        assert_eq!(declared_detail(0, 0), "nothing to load");
    }

    #[test]
    fn only_web_addresses_are_links() {
        for url in [
            "https://github.com/nsaintot/cdj3k-mods",
            "http://localhost:8080/x?y#z",
        ] {
            assert!(web_url(url), "{url}");
        }
        for url in [
            "github.com/nsaintot",
            "https://",
            "https:///path",
            "ftp://host/x",
            "https://host/a b",
            "javascript:alert(1)",
        ] {
            assert!(!web_url(url), "{url}");
        }
    }
}
