//! The confirmations shown in place of the card row before a slot's
//! installation is removed: replacing it with another deck, or emptying the
//! slot.

use cdj3k_emu_panel::Model;
use egui::{Align2, FontId, Pos2, Rect, Stroke, Vec2};

use super::header::{
    button_size, draw_footer, draw_header, footer_split, k_of, Header, STRIP_GAP, STRIP_MARK_GAP,
    STRIP_SWATCH,
};
use super::paint::{hairline, text_width, tracked};
use super::{PickerAction, PickerView};
use crate::app::theme::{self, Palette};

const CONFIRM_W: f32 = 560.0;
/// The room on both sides of the label on the confirmation's wide button.
const CONFIRM_BTN_PAD: f32 = 36.0;
/// The line above the plate that shows the current and the new deck, and the
/// arrow between them.
const SWAP_H: f32 = 14.0;
const SWAP_GAP: f32 = 18.0;
const SWAP_ARROW_W: f32 = 22.0;
const SWAP_FONT: f32 = 11.0;
/// The plate that lists what an install deletes.
const LOSS_PAD: f32 = 16.0;
const LOSS_HEAD_H: f32 = 32.0;
const LOSS_ROW_H: f32 = 34.0;
const LOSS_FONT: f32 = 13.0;
/// The dash before each item on the plate.
const LOSS_DASH_W: f32 = 8.0;
const LOSS_DASH_GAP: f32 = 12.0;
/// The line under the plate that names the mods the new model would not run.
const REPLACE_MODS_GAP: f32 = 14.0;
const REPLACE_MODS_GLYPH: f32 = 14.0;
const REPLACE_MODS_FONT: f32 = 12.5;

/// A step that asks for confirmation before the slot's installation is
/// removed.
struct Confirmation<'a> {
    head: Header<'a>,
    /// The decks shown above the plate: the one the slot holds, then the one
    /// that replaces it.
    swap: Option<(Option<Model>, Model)>,
    /// What is deleted, one row per item on the plate.
    losses: [String; 3],
    /// The slot's mods the new deck would not run, named under the plate.
    incompatible: Option<(Model, &'a [String])>,
    accept_label: String,
    accept: PickerAction,
    decline: PickerAction,
}

/// Draws the replace confirmation in place of the card row. It warns that the
/// slot's installation, its eMMC and everything the guest wrote to it are
/// deleted.
pub(super) fn draw_confirm(
    ui: &mut egui::Ui,
    view: &PickerView<'_>,
    model: Model,
    k: f32,
    pal: &Palette,
) -> Option<PickerAction> {
    let held = view.installed.map(|m| m.title()).unwrap_or("nothing");
    let title = format!("Replace the emulation in slot {}?", view.slot);
    let sub = if view.running.is_some() {
        format!(
            "Slot {} holds the {held}. Installing the {} replaces it, and the running \
             emulation stops first.",
            view.slot,
            model.title()
        )
    } else {
        format!(
            "Slot {} holds the {held}. Installing the {} replaces it.",
            view.slot,
            model.title()
        )
    };
    let incompatible = &view.replace_incompatible;
    let confirmation = Confirmation {
        head: confirm_header(view, &title, &sub),
        swap: Some((view.installed, model)),
        losses: [
            format!("The {held} firmware in this slot"),
            "Its eMMC image".to_owned(),
            "Every setting stored on it".to_owned(),
        ],
        incompatible: (!incompatible.is_empty()).then_some((model, incompatible.as_slice())),
        accept_label: format!("Install the {}", model.title()),
        accept: PickerAction::Replace(model),
        decline: PickerAction::CancelReplace,
    };
    draw_confirmation(ui, confirmation, k, pal)
}

/// Draws the delete confirmation, with the same plate as the replace
/// confirmation but with nothing installed in the slot's place.
pub(in crate::app) fn draw_delete_confirm(
    ui: &mut egui::Ui,
    view: &PickerView<'_>,
) -> Option<PickerAction> {
    let k = k_of(ui.max_rect());
    let pal = theme::palette(ui.ctx());
    let held = view.installed.map(|m| m.title()).unwrap_or("nothing");
    let title = format!("Delete the emulation in slot {}?", view.slot);
    let sub = if view.running_elsewhere {
        "Its window stops the emulation first - QEMU holds the eMMC open while it runs - and \
         the slot goes back to empty. Nothing is installed in its place."
    } else if view.running.is_some() {
        "The emulation stops first - QEMU holds the eMMC open while it runs - and the slot goes \
         back to empty. Nothing is installed in its place."
    } else {
        "The slot goes back to empty. Nothing is installed in its place."
    };
    let confirmation = Confirmation {
        head: confirm_header(view, &title, sub),
        swap: None,
        losses: [
            format!("The {held} firmware in this slot"),
            "Its eMMC image".to_owned(),
            "Everything stored on it - settings, library, cues".to_owned(),
        ],
        incompatible: None,
        accept_label: format!("Delete slot {}", view.slot),
        accept: PickerAction::ConfirmDelete,
        decline: PickerAction::CancelDelete,
    };
    draw_confirmation(ui, confirmation, k, pal)
}

fn confirm_header<'a>(view: &PickerView<'a>, title: &'a str, sub: &'a str) -> Header<'a> {
    Header {
        slot: Some(view.slot),
        model: view.installed,
        release: view.release,
        state: None,
        title,
        sub,
        switchable: false,
        warn: false,
    }
}

/// Draws the header, then the deck line, the plate and the incompatible mods
/// centred in the step's space, then the accept and decline buttons in the
/// action bar.
fn draw_confirmation(
    ui: &mut egui::Ui,
    c: Confirmation<'_>,
    k: f32,
    pal: &Palette,
) -> Option<PickerAction> {
    let body = draw_header(ui, &c.head).body;
    let (space, bar) = footer_split(body, k);

    let w = (CONFIRM_W * k).min(space.width() - 2.0 * theme::GUTTER * k);
    let plate_h = (LOSS_HEAD_H + 3.0 * LOSS_ROW_H) * k;
    let mods_line = c
        .incompatible
        .map(|(model, names)| incompatible_line(ui, model, names, pal, k));
    let mods_h = mods_line
        .as_ref()
        .map_or(0.0, |g| REPLACE_MODS_GAP * k + g.size().y);
    let lead_h = if c.swap.is_some() {
        (SWAP_H + SWAP_GAP) * k
    } else {
        0.0
    };
    let block_h = lead_h + plate_h + mods_h;
    let left = space.center().x - w * 0.5;
    let top = space.center().y - block_h * 0.5;
    let p = ui.painter();
    if let Some((from, to)) = c.swap {
        swap_line(p, [from, Some(to)], Pos2::new(left, top), pal, k);
    }
    let plate = loss_plate(
        p,
        Pos2::new(left, top + lead_h),
        Vec2::new(w, plate_h),
        &c.losses,
        pal,
        k,
    );
    if let Some(galley) = mods_line {
        incompatible_note(p, plate, galley, pal, k);
    }

    let inner = draw_footer(ui, bar, pal, k);
    confirm_buttons(ui, inner, &c.accept_label, c.accept, c.decline, pal, k)
}

/// Draws the current deck and the new deck from `at`, with an arrow between
/// them.
fn swap_line(p: &egui::Painter, decks: [Option<Model>; 2], at: Pos2, pal: &Palette, k: f32) {
    let swap_font = FontId::proportional(SWAP_FONT * k);
    let cy = at.y + SWAP_H * k * 0.5;
    let mut x = at.x;
    for (i, deck) in decks.into_iter().flatten().enumerate() {
        if i > 0 {
            let a = SWAP_ARROW_W * k;
            let arrow = Stroke::new(theme::HAIRLINE * k, pal.faint);
            p.line_segment([Pos2::new(x, cy), Pos2::new(x + a, cy)], arrow);
            for dy in [-1.0, 1.0] {
                p.line_segment(
                    [
                        Pos2::new(x + a, cy),
                        Pos2::new(x + a - a * 0.16, cy + a * 0.16 * dy),
                    ],
                    arrow,
                );
            }
            x += a + STRIP_GAP * k;
        }
        let s = STRIP_SWATCH * k;
        p.rect_filled(
            Rect::from_center_size(Pos2::new(x + s * 0.5, cy), Vec2::splat(s)),
            theme::HAIRLINE * k,
            crate::app::ui::slate::for_model(deck).accent,
        );
        x += s + STRIP_MARK_GAP * k;
        x += tracked(
            p,
            Pos2::new(x, cy),
            deck.title(),
            &swap_font,
            pal.muted,
            theme::MICRO_TRACK * k,
        ) + STRIP_GAP * k;
    }
}

/// Draws the plate that lists what is deleted: a warning band, then one
/// dashed row per item. Returns the plate's rect.
fn loss_plate(
    p: &egui::Painter,
    min: Pos2,
    size: Vec2,
    rows: &[String],
    pal: &Palette,
    k: f32,
) -> Rect {
    let plate = Rect::from_min_size(min, size);
    let round = theme::ROUND * k;
    p.rect_filled(plate, round, pal.plate);
    p.rect_stroke(
        plate,
        round,
        Stroke::new(theme::HAIRLINE * k, pal.line),
        egui::StrokeKind::Middle,
    );
    let band = Rect::from_min_size(min, Vec2::new(size.x, LOSS_HEAD_H * k));
    p.rect_filled(
        band,
        egui::epaint::CornerRadiusF32 {
            nw: round,
            ne: round,
            sw: 0.0,
            se: 0.0,
        },
        pal.field,
    );
    hairline(p, band.left_bottom(), band.right_bottom(), pal.line, k);
    tracked(
        p,
        Pos2::new(band.left() + LOSS_PAD * k, band.center().y),
        "DELETED, AND NOT RECOVERABLE",
        &FontId::proportional(theme::MICRO_FONT * k),
        pal.danger,
        theme::MICRO_TRACK * k,
    );
    for (i, row) in rows.iter().enumerate() {
        let ry = band.bottom() + (i as f32 + 0.5) * LOSS_ROW_H * k;
        let dx = plate.left() + LOSS_PAD * k;
        p.line_segment(
            [Pos2::new(dx, ry), Pos2::new(dx + LOSS_DASH_W * k, ry)],
            Stroke::new(theme::HAIRLINE * k, pal.line_strong),
        );
        p.text(
            Pos2::new(dx + (LOSS_DASH_W + LOSS_DASH_GAP) * k, ry),
            Align2::LEFT_CENTER,
            row,
            FontId::proportional(LOSS_FONT * k),
            pal.ink,
        );
    }
    plate
}

/// Lays out a line such as "2 of this slot's mods are incompatible with the
/// CDJ-3000 (a, b)." to the plate's width.
fn incompatible_line(
    ui: &egui::Ui,
    model: Model,
    names: &[String],
    pal: &Palette,
    k: f32,
) -> std::sync::Arc<egui::Galley> {
    let n = names.len();
    let lead = if n == 1 {
        format!(
            "1 of this slot's mods is incompatible with the {}",
            model.title()
        )
    } else {
        format!(
            "{n} of this slot's mods are incompatible with the {}",
            model.title()
        )
    };
    let font = FontId::proportional(REPLACE_MODS_FONT * k);
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &lead,
        0.0,
        egui::TextFormat::simple(font.clone(), pal.danger),
    );
    job.append(
        &format!(" ({}).", names.join(", ")),
        0.0,
        egui::TextFormat::simple(font, pal.muted),
    );
    job.wrap.max_width = (CONFIRM_W - REPLACE_MODS_GLYPH - 10.0) * k;
    ui.painter().layout_job(job)
}

/// Draws the incompatible mods line under `plate`, after a cube glyph.
fn incompatible_note(
    p: &egui::Painter,
    plate: Rect,
    galley: std::sync::Arc<egui::Galley>,
    pal: &Palette,
    k: f32,
) {
    let y = plate.bottom() + REPLACE_MODS_GAP * k;
    crate::app::mods_view::cube_glyph(
        p,
        Rect::from_min_size(
            Pos2::new(plate.left() + 2.0 * k, y + 2.0 * k),
            Vec2::splat(REPLACE_MODS_GLYPH * k),
        ),
        pal.danger,
        k,
    );
    p.galley(
        Pos2::new(plate.left() + (REPLACE_MODS_GLYPH + 10.0) * k, y),
        galley,
        pal.muted,
    );
}

/// Draws the wide destructive button that confirms, and "Keep it" beside it.
fn confirm_buttons(
    ui: &mut egui::Ui,
    inner: Rect,
    label: &str,
    accept: PickerAction,
    decline: PickerAction,
    pal: &Palette,
    k: f32,
) -> Option<PickerAction> {
    let mut action = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
        theme::apply_setup_style(ui, pal);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let wide = [
                text_width(
                    ui.painter(),
                    label,
                    &FontId::proportional(theme::BUTTON_FONT * k),
                    0.0,
                ) + CONFIRM_BTN_PAD * k,
                theme::BUTTON_SIZE[1] * k,
            ];
            if theme::destructive(ui, wide, label, pal).clicked() {
                action = Some(accept);
            }
            if theme::secondary(ui, button_size(k), "Keep it", pal).clicked() {
                action = Some(decline);
            }
        });
    });
    action
}
