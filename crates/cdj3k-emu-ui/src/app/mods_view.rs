//! The setup window's Mods view: a slot's mods in boot order, the result of
//! the last boot for each, and each mod's journal.
//!
//! The view draws a [`ModsPage`] that the shell fills from the slot's storage,
//! and returns the user's request as a [`ModsAction`]. It writes nothing to
//! disk itself.
//!
//! | Module | Holds |
//! |---|---|
//! | `read` | What the page shows, read from the slot |
//! | `list` | The list screen: toolbar, empty state, links, footer |
//! | `rows` | The table of mods on the list screen |
//! | `log_page` | One mod's details and journal |
//! | `dialogs` | The remove, replace and add-from-URL dialogs |
//! | `pieces`, `glyphs` | Drawing pieces the screens share |

use std::path::PathBuf;

use cdj3k_emu_panel::Model;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Vec2};

use super::picker::{
    draw_header, footer_split, hairline, k_of, text_width, tracked, trash_glyph, Header,
};
use super::theme::{self, Palette};

mod dialogs;
mod glyphs;
mod list;
mod log_page;
mod pieces;
mod read;
mod rows;

use dialogs::*;
use glyphs::*;
use list::*;
use log_page::*;
use pieces::*;
use read::one_line;
pub(in crate::app) use read::{incompatible_with, mods_bar, restart_pending};
use rows::*;

pub(in crate::app) use glyphs::cube_glyph;
pub(in crate::app) use pieces::dashed_rect;

pub(in crate::app) const DOCS_URL: &str =
    "https://github.com/nsaintot/cdj3k-emu/blob/main/docs/mods.md";
pub(in crate::app) const TEMPLATE_URL: &str =
    "https://github.com/nsaintot/cdj3k-emu/tree/main/docs/mods/example";

const LIST_SUB: &str = "Enabled mods run in list order at each boot, before the player app starts";
const EMPTY_SUB: &str = "No mods in this slot.";
const LOG_SUB: &str = "Details and script output from the last boot";

/// What happened to a mod on the last boot, or why it did not run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum RowStatus {
    /// Not in the last boot's report.
    NotRun,
    /// Its loader.sh exited 0.
    Loaded,
    /// The mod has no loader.sh. The text is what its `mod.toml` passed to
    /// the app, such as `env loaded`.
    Declared(String),
    /// How its loader.sh ended, such as `exited 1` or `stopped at 60 s`.
    Failed(String),
    /// The slot's firmware, then the firmware the mod declares, such as
    /// `3.22/CDJ-3000 → ≤3.20/CDJ-3000`.
    Incompatible(String),
    /// The mod has no usable `mod.toml`; the text gives the reason.
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
    /// The mod is a folder used in place, so removing it ejects it and
    /// never deletes it.
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
    /// The slot's "Enable Mods" setting.
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
    /// Whether the other window that runs the viewed slot answered `yes` to
    /// the last `restart-pending`. Read only when another window runs the
    /// slot.
    pub remote_pending: bool,
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
            remote_pending: false,
        }
    }

    /// Apply an action that changes only what the view shows: a dialog
    /// opening or closing, or the log page. Returns any other action, for the
    /// shell to carry out.
    pub(in crate::app) fn apply(&mut self, action: ModsAction) -> Option<ModsAction> {
        match action {
            ModsAction::AskRemove(name) => self.remove = Some(name),
            ModsAction::CancelRemove => self.remove = None,
            ModsAction::AskUrl => self.url = Some(String::new()),
            ModsAction::CancelUrl => self.url = None,
            ModsAction::OpenLog(name) => self.open_log(&name),
            ModsAction::CloseLog => {
                self.screen = ModsScreen::List;
                self.log = None;
            }
            other => return Some(other),
        }
        None
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

pub(in crate::app) fn draw_mods(ui: &mut egui::Ui, page: &mut ModsPage) -> Option<ModsAction> {
    match page.screen {
        ModsScreen::Log => draw_log(ui, page),
        ModsScreen::List => draw_list(ui, page),
    }
}
