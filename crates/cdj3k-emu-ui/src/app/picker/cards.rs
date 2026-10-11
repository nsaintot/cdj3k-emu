//! The card row: one plate per model, its miniature, and the pill that says
//! what clicking it does to the slot.

use cdj3k_emu_panel::Model;
use egui::{Align2, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use super::glyph::{draw_deck_glyph, draw_flat_glyph};
use super::paint::{lerp_color, text_width, tracked, tracked_center};
use super::{PickerAction, PickerView};
use crate::app::theme::{self, Palette};
use crate::app::ui::slate::card::CardGlyph;

const ANIM_TIME: f32 = 0.13;

const CARD_GAP: f32 = 22.0;
const CARD_PAD: f32 = 16.0;

/// The largest size of a card. Cards are centred in the row with the spare
/// space around them, and three cards fit the window's width before this cap
/// applies.
const CARD_MAX_W: f32 = 250.0;
const CARD_MAX_H: f32 = 300.0;
/// The space under the card row, so that the cards do not touch the action
/// bar.
const CARD_ROW_BOTTOM: f32 = 20.0;
/// The height of the band of deck colour across the top of a card. It is the
/// only full-width colour in the window.
const CARD_CAP_H: f32 = 3.0;
/// How far a card's edge darkens towards the ink on hover.
const CARD_HOVER_EDGE: f32 = 0.55;

// ── A card's contents, measured up from its foot ──────────────────────────────
const CARD_TITLE_FONT: f32 = 20.0;
const CARD_SUB_FONT: f32 = 12.0;
const CARD_ERA_FONT: f32 = 10.0;
const CARD_ERA_TRACK: f32 = 1.4;
/// The gaps up from the pill to the era line, and from there to the subtitle
/// and the title above it.
const CARD_ERA_GAP: f32 = 18.0;
const CARD_SUB_GAP: f32 = 16.0;
const CARD_TITLE_GAP: f32 = 20.0;
/// The gap between the title and the foot of the miniature above it.
const CARD_GLYPH_GAP: f32 = 14.0;

/// The "INSTALLED" chip in the top-right corner of the slot's own model.
pub(super) const CHIP_FONT: f32 = 9.0;
pub(super) const CHIP_TRACK: f32 = 1.3;
const CHIP_PAD_X: f32 = 7.0;
const CHIP_H: f32 = 18.0;
/// The chip's offset below the accent cap. CHIP_GLYPH_GAP is the gap between
/// the chip and the top of the miniature.
const CHIP_TOP: f32 = 12.0;
const CHIP_GLYPH_GAP: f32 = 10.0;

const PILL_FONT: f32 = 11.0;
const PILL_TRACK: f32 = 1.5;

/// What a card's foot pill offers for its model.
enum Pill {
    /// The slot's model, already running. The card offers to reinstall its
    /// firmware.
    Reinstall,
    /// The slot's model, ready to boot.
    Launch,
    /// An empty slot.
    Install,
    /// The slot holds another model, which an install replaces.
    Replace,
}

impl Pill {
    fn of(model: Model, view: &PickerView<'_>) -> Self {
        let installed = view.installed == Some(model);
        // Another slot's deck launches in its own window; while that window
        // is open, its card offers a reinstall instead.
        let (reinstall, launch) = if view.foreign {
            (installed && view.busy, installed)
        } else {
            (
                view.running == Some(model),
                view.all_launchable || installed,
            )
        };
        if reinstall {
            Self::Reinstall
        } else if launch {
            Self::Launch
        } else if view.installed.is_none() {
            Self::Install
        } else {
            Self::Replace
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Reinstall => "REINSTALL",
            Self::Launch => "LAUNCH",
            Self::Install => "INSTALL FIRMWARE",
            Self::Replace => "INSTALL & REPLACE",
        }
    }
}

/// How one card is drawn this frame.
struct CardState {
    model: Model,
    pill: Pill,
    /// The slot holds this model.
    installed: bool,
    /// How far the pointer's hover has faded in, 0 to 1.
    hover_t: f32,
}

/// Draws one card per model, centred in `space`. Returns the action of a
/// clicked card.
pub(super) fn draw_cards(
    ui: &egui::Ui,
    view: &PickerView<'_>,
    space: Rect,
    pal: &Palette,
    k: f32,
) -> Option<PickerAction> {
    let row = Rect::from_min_max(
        Pos2::new(space.left() + theme::GUTTER * k, space.top()),
        Pos2::new(
            space.right() - theme::GUTTER * k,
            space.bottom() - CARD_ROW_BOTTOM * k,
        ),
    );
    let n = Model::ALL.len() as f32;
    let gap = CARD_GAP * k;
    let cw = ((row.width() - gap * (n - 1.0)) / n).min(CARD_MAX_W * k);
    let ch = row.height().min(CARD_MAX_H * k);
    let x0 = row.center().x - (cw * n + gap * (n - 1.0)) * 0.5;
    let y0 = row.center().y - ch * 0.5;
    let mut action = None;
    for (i, &model) in Model::ALL.iter().enumerate() {
        let rect =
            Rect::from_min_size(Pos2::new(x0 + i as f32 * (cw + gap), y0), Vec2::new(cw, ch));
        // An install never touches a running emulation, so every card of
        // every slot is live.
        let resp =
            theme::pointer(ui.interact(rect, ui.id().with(("cdj_model", i)), Sense::click()));
        let hover_t = ui.ctx().animate_bool_with_time(
            ui.id().with(("cdj_hover", i)),
            resp.hovered(),
            ANIM_TIME,
        );
        let state = CardState {
            model,
            pill: Pill::of(model, view),
            installed: view.installed == Some(model),
            hover_t,
        };
        draw_card(ui.painter(), rect, &state, k, pal);
        if resp.clicked() {
            action = Some(match state.pill {
                Pill::Reinstall => PickerAction::Reinstall(model),
                Pill::Launch if view.foreign => PickerAction::Open(view.slot),
                _ => PickerAction::Choose(model),
            });
        }
    }
    action
}

fn draw_card(p: &egui::Painter, rect: Rect, card: &CardState, k: f32, pal: &Palette) {
    let slate = crate::app::ui::slate::for_model(card.model);
    let cap = card_plate(p, rect, slate.accent, card.hover_t, pal, k);
    if card.installed {
        installed_chip(p, rect, cap.bottom(), pal, k);
    }

    // Everything below the miniature is measured up from the card's foot, so
    // all the cards agree line for line whatever the window's size factor is.
    let pad = CARD_PAD * k;
    let pill = Rect::from_min_max(
        Pos2::new(rect.left() + pad, rect.bottom() - pad - theme::PILL_H * k),
        Pos2::new(rect.right() - pad, rect.bottom() - pad),
    );
    let title_y = card_lines(p, rect.center().x, pill.top(), card.model, pal, k);

    // The miniature fills the space between the chip above and the title
    // below.
    let area = Rect::from_min_max(
        Pos2::new(
            rect.left() + pad,
            cap.bottom() + (CHIP_TOP + CHIP_H + CHIP_GLYPH_GAP) * k,
        ),
        Pos2::new(
            rect.right() - pad,
            title_y - CARD_TITLE_FONT * 0.5 * k - CARD_GLYPH_GAP * k,
        ),
    );
    match &slate.card.glyph {
        CardGlyph::Riser(g) => draw_deck_glyph(
            p,
            area,
            g,
            pal,
            lerp_color(pal.plate, pal.plate_hover, card.hover_t),
            k,
        ),
        CardGlyph::Flat => draw_flat_glyph(p, area, k, pal.glyph, pal.glyph_detail),
    }
    draw_pill(p, pill, &card.pill, card.hover_t, pal, k);
}

/// Draws a flat plate with a hairline edge and the deck's colour across its
/// top. On hover the plate takes its hover shade and the edge darkens towards
/// the ink. Returns the coloured cap.
fn card_plate(
    p: &egui::Painter,
    rect: Rect,
    accent: egui::Color32,
    hover_t: f32,
    pal: &Palette,
    k: f32,
) -> Rect {
    let round = theme::ROUND * k;
    p.rect_filled(rect, round, lerp_color(pal.plate, pal.plate_hover, hover_t));
    p.rect_stroke(
        rect,
        round,
        Stroke::new(
            theme::HAIRLINE * k,
            lerp_color(pal.line, pal.ink, hover_t * CARD_HOVER_EDGE),
        ),
        egui::StrokeKind::Middle,
    );
    let cap = Rect::from_min_size(rect.min, Vec2::new(rect.width(), CARD_CAP_H * k));
    p.rect_filled(
        cap,
        egui::epaint::CornerRadiusF32 {
            nw: round,
            ne: round,
            sw: 0.0,
            se: 0.0,
        },
        accent,
    );
    cap
}

/// Draws "INSTALLED" in the card's top-right corner, under the cap whose foot
/// is at `top`.
fn installed_chip(p: &egui::Painter, rect: Rect, top: f32, pal: &Palette, k: f32) {
    let font = FontId::proportional(CHIP_FONT * k);
    let w = text_width(p, "INSTALLED", &font, CHIP_TRACK * k) + 2.0 * CHIP_PAD_X * k;
    let chip = Rect::from_min_size(
        Pos2::new(rect.right() - CARD_PAD * k - w, top + CHIP_TOP * k),
        Vec2::new(w, CHIP_H * k),
    );
    p.rect_stroke(
        chip,
        theme::ROUND_CHIP * k,
        Stroke::new(theme::HAIRLINE * k, pal.line_strong),
        egui::StrokeKind::Middle,
    );
    tracked(
        p,
        Pos2::new(chip.left() + CHIP_PAD_X * k, chip.center().y),
        "INSTALLED",
        &font,
        pal.muted,
        CHIP_TRACK * k,
    );
}

/// Draws the era, the subtitle and the title, centred on `cx` and stacked up
/// from the pill's top. Returns the title's centre line.
fn card_lines(
    p: &egui::Painter,
    cx: f32,
    pill_top: f32,
    model: Model,
    pal: &Palette,
    k: f32,
) -> f32 {
    let card = &crate::app::ui::slate::for_model(model).card;
    let era_y = pill_top - CARD_ERA_GAP * k;
    let sub_y = era_y - CARD_SUB_GAP * k;
    let title_y = sub_y - CARD_TITLE_GAP * k;
    tracked_center(
        p,
        Pos2::new(cx, era_y),
        card.era,
        &FontId::proportional(CARD_ERA_FONT * k),
        pal.faint,
        CARD_ERA_TRACK * k,
    );
    p.text(
        Pos2::new(cx, sub_y),
        Align2::CENTER_CENTER,
        card.subtitle,
        FontId::proportional(CARD_SUB_FONT * k),
        pal.muted,
    );
    p.text(
        Pos2::new(cx, title_y),
        Align2::CENTER_CENTER,
        model.title(),
        FontId::proportional(CARD_TITLE_FONT * k),
        pal.ink,
    );
    title_y
}

/// Draws the foot pill, which names what clicking the card does to the slot.
/// Every pill has the same outline and fills on hover. Pills use no colour,
/// because colour marks destructive actions and the deck accent marks the
/// model.
fn draw_pill(p: &egui::Painter, pill: Rect, kind: &Pill, hover_t: f32, pal: &Palette, k: f32) {
    let round = theme::ROUND * k;
    let fill = lerp_color(pal.plate, pal.ink, hover_t);
    let text = lerp_color(pal.ink, pal.on_ink, hover_t);
    p.rect_filled(pill, round, fill);
    p.rect_stroke(
        pill,
        round,
        Stroke::new(theme::HAIRLINE * k, pal.ink),
        egui::StrokeKind::Middle,
    );
    tracked_center(
        p,
        pill.center(),
        kind.label(),
        &FontId::proportional(PILL_FONT * k),
        text,
        PILL_TRACK * k,
    );
}
