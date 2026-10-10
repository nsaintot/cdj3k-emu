//! The Mods bar between the cards and the action bar.

use egui::{Align2, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use super::paint::tracked;
use crate::app::mods_view::{self, ModsBar};
use crate::app::theme::{self, Palette};

const MODS_BAR_H: f32 = 44.0;
const MODS_BAR_BOTTOM: f32 = 16.0;
const MODS_BAR_PAD: f32 = 16.0;
const MODS_BAR_GAP: f32 = 12.0;
const MODS_BAR_GLYPH: f32 = 16.0;
const MODS_BAR_FONT: f32 = 12.5;
const MODS_BAR_DOT: f32 = 7.0;
const MODS_BAR_DOT_GAP: f32 = 6.0;

/// The Mods bar's rect: along the foot of `space`, inside the gutters.
pub(super) fn mods_bar_rect(space: Rect, k: f32) -> Rect {
    Rect::from_min_max(
        Pos2::new(
            space.left() + theme::GUTTER * k,
            space.bottom() - (MODS_BAR_H + MODS_BAR_BOTTOM) * k,
        ),
        Pos2::new(
            space.right() - theme::GUTTER * k,
            space.bottom() - MODS_BAR_BOTTOM * k,
        ),
    )
}

/// Draws the Mods bar: how many mods are enabled, with a note when they are
/// disabled or some are incompatible, or an invitation to add one when there
/// are none.
/// Clicking it opens the Mods view.
pub(super) fn draw_mods_bar(
    ui: &egui::Ui,
    rect: Rect,
    mods: &ModsBar,
    k: f32,
    pal: &Palette,
) -> egui::Response {
    let resp = theme::pointer(ui.interact(rect, ui.id().with("cdj_mods_bar"), Sense::click()));
    let p = ui.painter();
    let empty = mods.total == 0;
    bar_ground(p, rect, empty, resp.hovered(), pal, k);
    let cy = rect.center().y;
    let mut x = bar_lead(
        p,
        Pos2::new(rect.left() + MODS_BAR_PAD * k, cy),
        empty,
        pal,
        k,
    );
    let font = FontId::proportional(MODS_BAR_FONT * k);
    let summary = if empty {
        "None installed".to_string()
    } else {
        format!("{} enabled", mods.on)
    };
    x += p
        .text(
            Pos2::new(x, cy),
            Align2::LEFT_CENTER,
            summary,
            font.clone(),
            pal.muted,
        )
        .width()
        + MODS_BAR_GAP * k;
    if let Some((note, failure)) = &mods.note {
        let col = if *failure { pal.danger } else { pal.warn };
        let r = MODS_BAR_DOT * k * 0.5;
        p.circle_filled(Pos2::new(x + r, cy), r, col);
        p.text(
            Pos2::new(x + 2.0 * r + MODS_BAR_DOT_GAP * k, cy),
            Align2::LEFT_CENTER,
            note,
            font.clone(),
            col,
        );
    }
    p.text(
        Pos2::new(rect.right() - MODS_BAR_PAD * k, cy),
        Align2::RIGHT_CENTER,
        if empty {
            "Add a mod →"
        } else {
            "Manage mods →"
        },
        font,
        pal.ink,
    );
    resp
}

/// Draws the bar's ground: a dashed outline while there are no mods, or a
/// plate otherwise. Both darken on hover.
fn bar_ground(p: &egui::Painter, rect: Rect, empty: bool, hot: bool, pal: &Palette, k: f32) {
    if empty {
        mods_view::dashed_rect(p, rect, if hot { pal.ink } else { pal.line_strong }, k);
        return;
    }
    let round = theme::ROUND * k;
    p.rect_filled(rect, round, if hot { pal.plate_hover } else { pal.plate });
    p.rect_stroke(
        rect,
        round,
        Stroke::new(theme::HAIRLINE * k, if hot { pal.ink } else { pal.line }),
        egui::StrokeKind::Middle,
    );
}

/// Draws the cube and the MODS label from `at`. Returns the x where the text
/// after them starts.
fn bar_lead(p: &egui::Painter, at: Pos2, empty: bool, pal: &Palette, k: f32) -> f32 {
    let (mut x, cy) = (at.x, at.y);
    mods_view::cube_glyph(
        p,
        Rect::from_min_size(
            Pos2::new(x, cy - MODS_BAR_GLYPH * k * 0.5),
            Vec2::splat(MODS_BAR_GLYPH * k),
        ),
        if empty { pal.faint } else { pal.dim },
        k,
    );
    x += (MODS_BAR_GLYPH + MODS_BAR_GAP) * k;
    x += tracked(
        p,
        Pos2::new(x, cy),
        "MODS",
        &FontId::proportional(theme::MICRO_FONT * k),
        if empty { pal.dim } else { pal.ink },
        theme::MICRO_TRACK * k,
    ) + MODS_BAR_GAP * k;
    x
}
