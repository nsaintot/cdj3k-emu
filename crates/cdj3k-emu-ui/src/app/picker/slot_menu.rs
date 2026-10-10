//! The slot chip in the identity strip, and the menu of slots it opens.

use egui::{Align2, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use super::cards::{CHIP_FONT, CHIP_TRACK};
use super::header::{STRIP_MARK_GAP, STRIP_SWATCH};
use super::paint::{text_width, tracked};
use crate::app::theme::{self, Palette};

const SLOT_CHIP_H: f32 = 22.0;
const SLOT_CHIP_PAD: f32 = 9.0;
const SLOT_CHEV_W: f32 = 8.0;
const SLOT_CHEV_H: f32 = 4.0;
const SLOT_MENU_W: f32 = 296.0;
const SLOT_MENU_ROW_H: f32 = 30.0;
const SLOT_MENU_PAD: f32 = 10.0;
const SLOT_MENU_GAP: f32 = 10.0;
const SLOT_MENU_FONT: f32 = 12.0;
/// The offset of a slot's deck name, after the slot number.
const SLOT_MENU_NAME_W: f32 = 52.0;
/// The room kept at the right end for the "THIS WINDOW" note.
const SLOT_MENU_HERE_W: f32 = 74.0;
/// The gap between a slot's deck name and its firmware release.
const SLOT_MENU_REL_GAP: f32 = 7.0;

/// Draws the slot chip with its left end and centre line at `at`. It names the
/// slot this window shows and opens a menu of the other slots. Picking a slot
/// points this window at it; that slot's emulation still runs in its own
/// window. Returns the chip's width and any slot picked.
pub(super) fn draw_slot_switch(
    ui: &egui::Ui,
    at: Pos2,
    slot: u32,
    switchable: bool,
    font: &FontId,
    pal: &Palette,
    k: f32,
) -> (f32, Option<u32>) {
    let label = format!("SLOT {slot}");
    let p = ui.painter();
    let text_w = text_width(p, &label, font, theme::MICRO_TRACK * k);
    let mut w = text_w + (SLOT_CHIP_PAD * 2.0 + STRIP_MARK_GAP) * k;
    if switchable {
        w += SLOT_CHEV_W * k;
    }
    let chip = Rect::from_center_size(
        Pos2::new(at.x + w * 0.5, at.y),
        Vec2::new(w, SLOT_CHIP_H * k),
    );
    // Draw the chip's outline in `edge`, and its label.
    let face = |edge: egui::Color32| {
        p.rect_stroke(
            chip,
            theme::ROUND * k,
            Stroke::new(theme::HAIRLINE * k, edge),
            egui::StrokeKind::Middle,
        );
        tracked(
            p,
            Pos2::new(chip.left() + SLOT_CHIP_PAD * k, at.y),
            &label,
            font,
            pal.ink,
            theme::MICRO_TRACK * k,
        );
    };
    if !switchable {
        face(pal.line_strong);
        return (w, None);
    }
    let id = ui.id().with("cdj_slot_switch");
    let resp = theme::pointer(ui.interact(chip, id, Sense::click()));
    let popup = ui.id().with("cdj_slot_menu");
    let open = egui::Popup::is_id_open(ui.ctx(), popup);
    face(if open || resp.hovered() {
        pal.ink
    } else {
        pal.line_strong
    });
    chevron(p, Pos2::new(chip.right(), at.y), pal, k);
    (w, slot_menu(ui, &resp, id, popup, slot, pal, k))
}

/// Draws a downward chevron before the chip's right end at `end`, to show
/// that the chip opens a menu.
fn chevron(p: &egui::Painter, end: Pos2, pal: &Palette, k: f32) {
    let c = Pos2::new(end.x - SLOT_CHIP_PAD * k - SLOT_CHEV_W * k * 0.5, end.y);
    let (hw, hh) = (SLOT_CHEV_W * k * 0.5, SLOT_CHEV_H * k * 0.5);
    let chev = Stroke::new(theme::HAIRLINE * k * 1.2, pal.dim);
    p.line_segment(
        [Pos2::new(c.x - hw, c.y - hh), Pos2::new(c.x, c.y + hh)],
        chev,
    );
    p.line_segment(
        [Pos2::new(c.x + hw, c.y - hh), Pos2::new(c.x, c.y + hh)],
        chev,
    );
}

/// Draws the menu the chip opens, with one row per slot. Returns a slot
/// other than `slot` that was picked.
fn slot_menu(
    ui: &egui::Ui,
    resp: &egui::Response,
    id: egui::Id,
    popup: egui::Id,
    slot: u32,
    pal: &Palette,
    k: f32,
) -> Option<u32> {
    let mut picked = None;
    // The popup takes its frame from the style it is given, so this host UI
    // applies the setup palette over the context's in-window menu style.
    let mut host = egui::Ui::new(
        ui.ctx().clone(),
        id.with("menu_host"),
        egui::UiBuilder::new()
            .layer_id(ui.layer_id())
            .max_rect(ui.max_rect()),
    );
    theme::apply_setup_style(&mut host, pal);
    let style = (**host.style()).clone();
    egui::Popup::from_toggle_button_response(resp)
        .id(popup)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .style(style)
        .width(SLOT_MENU_W * k)
        .show(|ui| {
            theme::apply_setup_style(ui, pal);
            ui.spacing_mut().item_spacing.y = 2.0 * k;
            for n in 1..=cdj3k_emu_platform::menu_state::MAX_INSTANCES {
                if slot_menu_row(ui, n, n == slot, pal, k).clicked() {
                    if n != slot {
                        picked = Some(n);
                    }
                    egui::Popup::close_id(ui.ctx(), popup);
                }
            }
        });
    picked
}

/// Draws one slot in that menu: its deck colour, its name, and what it holds.
fn slot_menu_row(ui: &mut egui::Ui, n: u32, here: bool, pal: &Palette, k: f32) -> egui::Response {
    // Read what the slot's files hold, because a slot keeps its model in
    // settings after its files are deleted.
    let held = cdj3k_emu_storage::slot_summary(n);
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), SLOT_MENU_ROW_H * k),
        Sense::click(),
    );
    let p = ui.painter();
    if here || resp.hovered() {
        p.rect_filled(rect, theme::ROUND_CHIP * k, pal.field);
    }
    let mark = row_swatch(p, rect, held.as_ref().map(|(m, _)| *m), pal, k);
    let x = mark.right() + SLOT_MENU_GAP * k;
    row_texts(p, rect, x, n, held.as_ref(), pal, k);
    if here {
        tracked(
            p,
            Pos2::new(
                rect.right() - SLOT_MENU_PAD * k - SLOT_MENU_HERE_W * k,
                rect.center().y,
            ),
            "THIS WINDOW",
            &FontId::proportional(CHIP_FONT * k),
            pal.faint,
            CHIP_TRACK * k,
        );
    }
    theme::pointer(resp)
}

/// Draws the square at a row's left end: filled with the deck's colour, or an
/// empty outline for an empty slot. Returns its rect.
fn row_swatch(
    p: &egui::Painter,
    rect: Rect,
    held: Option<cdj3k_emu_panel::Model>,
    pal: &Palette,
    k: f32,
) -> Rect {
    let s = STRIP_SWATCH * k;
    let mark = Rect::from_center_size(
        Pos2::new(rect.left() + SLOT_MENU_PAD * k + s * 0.5, rect.center().y),
        Vec2::splat(s),
    );
    match held {
        Some(m) => {
            p.rect_filled(
                mark,
                theme::HAIRLINE * k,
                crate::app::ui::slate::for_model(m).accent,
            );
        }
        None => {
            p.rect_stroke(
                mark,
                theme::HAIRLINE * k,
                Stroke::new(theme::HAIRLINE * k, pal.off_line),
                egui::StrokeKind::Middle,
            );
        }
    }
    mark
}

/// Draws the slot's number from `x`, then the deck it holds and the firmware
/// it was installed from.
fn row_texts(
    p: &egui::Painter,
    rect: Rect,
    x: f32,
    n: u32,
    held: Option<&(cdj3k_emu_panel::Model, Option<String>)>,
    pal: &Palette,
    k: f32,
) {
    let text = FontId::proportional(SLOT_MENU_FONT * k);
    p.text(
        Pos2::new(x, rect.center().y),
        Align2::LEFT_CENTER,
        format!("Slot {n}"),
        text.clone(),
        pal.ink,
    );
    let name = held.map_or("empty", |(m, _)| m.title());
    let name_x = x + SLOT_MENU_NAME_W * k;
    p.text(
        Pos2::new(name_x, rect.center().y),
        Align2::LEFT_CENTER,
        name,
        text.clone(),
        if held.is_some() {
            pal.muted
        } else {
            pal.off_text
        },
    );
    if let Some(release) = held.and_then(|(_, r)| r.as_deref()) {
        p.text(
            Pos2::new(
                name_x + text_width(p, name, &text, 0.0) + SLOT_MENU_REL_GAP * k,
                rect.center().y,
            ),
            Align2::LEFT_CENTER,
            release,
            text,
            pal.faint,
        );
    }
}
