//! The model picker screen, where the user chooses a device emulation.
//!
//! It shows one card per [`Model`]. Every step uses the same frame: an
//! identity strip at the top, a title block, the step's own content, and an
//! action bar at the foot. Pixel sizes are multiplied by `k`, a size factor derived from the
//! window width.

mod cards;
mod dialogs;
mod glyph;
mod header;
mod mods_bar;
mod paint;
mod slot_menu;

use cdj3k_emu_panel::Model;
use egui::Rect;

use super::theme::{self, Palette};
pub(in crate::app) use dialogs::draw_delete_confirm;
pub(in crate::app) use header::{
    button_size, draw_busy, draw_footer, draw_header, footer_rule, footer_split, k_of, Header,
};
pub(in crate::app) use paint::{hairline, text_width, tracked, trash_glyph};

/// The width of the Delete slot button, beside Close in the action bar.
const DELETE_BTN_W: f32 = 138.0;

/// What the picker is being asked to show.
pub(in crate::app) struct PickerView<'a> {
    pub slot: u32,
    /// A line under the header, such as a stop in progress.
    pub status: Option<&'a str>,
    /// The model installed in this slot. A slot holds one installation.
    pub installed: Option<Model>,
    /// The model whose emulation is running behind the picker.
    pub running: Option<Model>,
    /// Every card launches with nothing installed (`--no-spawn`).
    pub all_launchable: bool,
    /// The model that would replace the slot's installation. Its
    /// confirmation is shown in place of the cards.
    pub confirm_replace: Option<Model>,
    /// The picker is a window over a live emulation, so it can be dismissed.
    pub dismissable: bool,
    /// The slot shown is not this window's. Its emulation runs in its own
    /// window, so the deck it holds offers a launch rather than a reinstall.
    pub foreign: bool,
    /// That slot's window is already open, so only that window changes the
    /// slot's installation; a delete from here is sent to it as a request.
    pub busy: bool,
    /// That window's emulation is running, not merely open.
    pub running_elsewhere: bool,
    /// No slot holds an installation, so the user must install one to
    /// continue.
    pub every_slot_empty: bool,
    /// The firmware release installed in the slot shown.
    pub release: Option<&'a str>,
    /// The slot shown needs a reinstall.
    pub outdated: bool,
    /// The slot's mods, for the bar under the cards.
    pub mods: Option<&'a super::mods_view::ModsBar>,
    /// The slot's mods the confirmed model would not run.
    pub replace_incompatible: Vec<String>,
}

pub(in crate::app) enum PickerAction {
    /// Point the window at another slot.
    View(u32),
    /// Bring up a slot's own window, where its emulation runs.
    Open(u32),
    /// A card was clicked.
    Choose(Model),
    /// The running deck's card was clicked: reinstall its firmware.
    Reinstall(Model),
    /// The replace confirmation was accepted.
    Replace(Model),
    /// The replace confirmation was declined.
    CancelReplace,
    /// Leave the slot as it is.
    Dismiss,
    /// Empty the slot: no firmware, no eMMC, nothing installed in its place.
    Delete,
    /// The delete confirmation was accepted.
    ConfirmDelete,
    /// The delete confirmation was declined.
    CancelDelete,
    /// The Mods bar was clicked.
    ManageMods,
    /// An action from the Mods view.
    Mods(super::mods_view::ModsAction),
}

/// The subtitle of a slot that needs a reinstall
/// ([`cdj3k_emu_storage::slot_outdated`]).
const OUTDATED: &str = "The slot installation is outdated. Reinstall to update it.";

/// Draw the picker into `ui`; returns what the user asked for.
pub(super) fn draw_picker(ui: &mut egui::Ui, view: &PickerView<'_>) -> Option<PickerAction> {
    let k = k_of(ui.max_rect());
    let pal = theme::palette(ui.ctx());

    if let Some(model) = view.confirm_replace {
        return dialogs::draw_confirm(ui, view, model, k, pal);
    }

    let warn = view.status.is_none() && view.outdated;
    let sub = view.status.unwrap_or(if warn {
        OUTDATED
    } else {
        "emulation model for the current slot"
    });
    let out = draw_header(ui, &picker_header(view, sub, warn));
    let (mut space, bar) = footer_split(out.body, k);
    let switched = out.switch_to.map(PickerAction::View);
    let mods = draw_mods_strip(ui, view, &mut space, pal, k);
    let chosen = cards::draw_cards(ui, view, space, pal, k);
    let inner = draw_footer(ui, bar, pal, k);
    let bar_action = draw_slot_actions(ui, view, inner, pal, k);
    bar_action.or(chosen).or(mods).or(switched)
}

fn picker_header<'a>(view: &PickerView<'a>, sub: &'a str, warn: bool) -> Header<'a> {
    Header {
        slot: Some(view.slot),
        // Show the deck the slot holds, running or not, so the strip names
        // the slot and what it holds.
        model: view.running.or(view.installed),
        release: view.release,
        state: match (view.running, view.busy) {
            (Some(_), _) => Some("RUNNING"),
            (None, true) if view.running_elsewhere => Some("RUNNING"),
            (None, true) => Some("OPEN"),
            _ => None,
        },
        title: if view.every_slot_empty {
            "Install an emulation"
        } else {
            "Choose an emulation"
        },
        sub,
        switchable: true,
        warn,
    }
}

/// Draws the Mods bar along the foot of `space` for a slot with something
/// installed, and shrinks `space` by the bar's height.
fn draw_mods_strip(
    ui: &egui::Ui,
    view: &PickerView<'_>,
    space: &mut Rect,
    pal: &Palette,
    k: f32,
) -> Option<PickerAction> {
    let (Some(mods), Some(_)) = (view.mods, view.installed) else {
        return None;
    };
    let strip = mods_bar::mods_bar_rect(*space, k);
    let clicked = mods_bar::draw_mods_bar(ui, strip, mods, k, pal).clicked();
    space.max.y = strip.top();
    clicked.then_some(PickerAction::ManageMods)
}

/// Draws the action bar's Close and Delete slot buttons. Close leaves the slot
/// as it is; it shows when the picker is dismissable or any slot has an
/// installation. Delete slot empties an installed slot; a slot held by another
/// window is emptied by that window on request.
fn draw_slot_actions(
    ui: &mut egui::Ui,
    view: &PickerView<'_>,
    inner: Rect,
    pal: &Palette,
    k: f32,
) -> Option<PickerAction> {
    let deletable = view.installed.is_some();
    let closable = view.dismissable || !view.every_slot_empty;
    if !closable && !deletable {
        return None;
    }
    let mut action = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
        theme::apply_setup_style(ui, pal);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if closable && theme::secondary(ui, button_size(k), "Close", pal).clicked() {
                action = Some(PickerAction::Dismiss);
            }
            // Delete slot sits after a rule and is outlined, because it is
            // destructive but is not this step's main action.
            if deletable {
                if closable {
                    footer_rule(ui, pal, k);
                }
                // The label does not name the slot, because the strip at the
                // top of the window shows it.
                if theme::destructive_line(
                    ui,
                    [DELETE_BTN_W * k, theme::BUTTON_SIZE[1] * k],
                    "Delete slot",
                    pal,
                    |p, icon, col| trash_glyph(p, icon, col, k),
                )
                .clicked()
                {
                    action = Some(PickerAction::Delete);
                }
            }
        });
    });
    action
}
