//! SHIFT, CUE and PLAY down the left of the jog. SHIFT is a key like the
//! others; CUE and PLAY stand taller.
//!
//! CUE and PLAY light their legends: the word CUE and its
//! back-to-cue mark in amber, the play/pause mark in the CDJ-3000's green.

use cdj3k_emu_panel::{Btn, Lamp};
use egui::{FontFamily, Pos2, Rect, Shape, Stroke, Vec2};

use crate::app::ui::{COL_BTN_CUE, COL_BTN_PLAY, COL_BTN_TEXT, COL_SILVER};
use crate::app::CdjApp;

use super::key::{self, Outline};
use super::layout::px;
use super::UiScale;

// ── SHIFT ─────────────────────────────────────────────────────────────────────
const SHIFT: (f32, f32, f32, f32) = (81.0, 1550.0, 203.0, 1605.0);
/// The printed outline around the word, inside the cap.
const SHIFT_PRINT_INSET: f32 = 13.0;
const SHIFT_TEXT: (f32, f32, f32, f32) = (111.0, 1570.0, 174.0, 1585.0);

// ── CUE / PLAY ────────────────────────────────────────────────────────────────
const CUE_CENTER: (f32, f32) = (141.84, 1766.89);
const PLAY_CENTER: (f32, f32) = (141.85, 1993.65);
const ROUND_R: f32 = 77.0;

const CUE_TEXT: (f32, f32, f32, f32) = (116.0, 1758.0, 168.0, 1776.0);
/// The back-to-cue mark under the word: a pill around a bar and a
/// left-pointing triangle.
const CUE_MARK_PILL: (f32, f32, f32, f32) = (128.0, 1799.0, 157.0, 1813.0);
const CUE_MARK_BAR: (f32, f32, f32, f32) = (136.0, 1802.0, 138.0, 1810.0);
const CUE_MARK_TRI: [(f32, f32); 3] = [(138.5, 1805.5), (148.5, 1801.5), (148.5, 1809.5)];

/// The play/pause mark: a right-pointing triangle, a slash, two bars.
const PLAY_TRI: [(f32, f32); 3] = [(117.0, 1987.0), (136.0, 1994.5), (117.0, 2002.0)];
const PLAY_SLASH: [(f32, f32); 4] = [
    (139.0, 2002.0),
    (143.0, 1987.0),
    (146.5, 1987.0),
    (142.5, 2002.0),
];
const PLAY_BARS: [(f32, f32, f32, f32); 2] = [
    (152.0, 1986.0, 158.0, 2002.0),
    (161.0, 1986.0, 168.0, 2002.0),
];

const PRINT_STROKE: f32 = 2.0;
const MARK_STROKE: f32 = 3.0;

fn p2(layout: &UiScale, (x, y): (f32, f32)) -> Pos2 {
    layout.sp(px(x), px(y))
}

fn screen_box(layout: &UiScale, (l, t, r, b): (f32, f32, f32, f32)) -> Rect {
    Rect::from_min_max(p2(layout, (l, t)), p2(layout, (r, b)))
}

fn ref_box((l, t, r, b): (f32, f32, f32, f32)) -> Rect {
    Rect::from_min_max(Pos2::new(px(l), px(t)), Pos2::new(px(r), px(b)))
}

pub(super) fn draw_shift(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
    let ctx = ui.ctx().clone();
    let bold = FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS_BOLD.into());
    let outline = shift_outline();
    app.shape_btn(
        ui,
        outline.screen_rect(layout),
        0,
        "shift",
        Some(Btn::Shift),
        |list, pressed| {
            key::collect_key(list, layout, outline, pressed);
            list.add(Shape::closed_line(
                outline.inset(px(SHIFT_PRINT_INSET)).points(layout),
                Stroke::new(layout.sc(PRINT_STROKE), COL_SILVER),
            ));
            list.text_in_ink_box(
                &ctx,
                screen_box(layout, SHIFT_TEXT),
                "SHIFT",
                bold,
                COL_BTN_TEXT,
            );
        },
    );
}

pub(super) fn draw_cue_play(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
    let ctx = ui.ctx().clone();
    let bold = FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS_BOLD.into());

    // CUE
    {
        let col = key::led(COL_BTN_CUE, app.mosi().drive(Lamp::Cue));
        let outline = round_outline(CUE_CENTER);
        let ctx = ctx.clone();
        let bold = bold.clone();
        let state = u32::from_le_bytes(col.to_array());
        app.shape_btn(
            ui,
            outline.screen_rect(layout),
            state,
            "cue",
            Some(Btn::Cue),
            |list, pressed| {
                key::collect_key(list, layout, outline, pressed);
                list.text_in_ink_box(&ctx, screen_box(layout, CUE_TEXT), "CUE", bold, col);
                // The mark under the legend is printed.
                let pill = screen_box(layout, CUE_MARK_PILL);
                list.rect_stroke(
                    pill,
                    pill.height() * 0.5,
                    Stroke::new(layout.sc(MARK_STROKE), COL_BTN_TEXT),
                );
                list.rect_filled(screen_box(layout, CUE_MARK_BAR), 0.0, COL_BTN_TEXT);
                list.add(Shape::convex_polygon(
                    CUE_MARK_TRI.iter().map(|&p| p2(layout, p)).collect(),
                    COL_BTN_TEXT,
                    Stroke::NONE,
                ));
            },
        );
    }

    // PLAY
    {
        // The legend lights in one tint, whatever colour the frame asks for.
        let col = key::led(COL_BTN_PLAY, app.mosi().drive(Lamp::Play));
        let outline = round_outline(PLAY_CENTER);
        let state = u32::from_le_bytes(col.to_array());
        app.shape_btn(
            ui,
            outline.screen_rect(layout),
            state,
            "play",
            Some(Btn::Play),
            |list, pressed| {
                key::collect_key(list, layout, outline, pressed);
                list.add(Shape::convex_polygon(
                    PLAY_TRI.iter().map(|&p| p2(layout, p)).collect(),
                    col,
                    Stroke::NONE,
                ));
                list.add(Shape::convex_polygon(
                    PLAY_SLASH.iter().map(|&p| p2(layout, p)).collect(),
                    col,
                    Stroke::NONE,
                ));
                for bar in PLAY_BARS {
                    list.rect_filled(screen_box(layout, bar), 0.0, col);
                }
            },
        );
    }
}

fn shift_outline() -> Outline {
    let rect = ref_box(SHIFT);
    Outline::new(rect, [rect.height() * 0.5; 4])
}

/// SHIFT's cap, then CUE's and PLAY's, reference units.
pub(super) fn footprints() -> (Vec<Pos2>, Vec<Vec<Pos2>>) {
    let cap = |o: Outline| key::face(o).ref_points();
    (
        cap(shift_outline()),
        [CUE_CENTER, PLAY_CENTER]
            .map(|c| cap(round_outline(c)))
            .to_vec(),
    )
}

fn round_outline(center: (f32, f32)) -> Outline {
    let c = Pos2::new(px(center.0), px(center.1));
    let r = px(ROUND_R);
    Outline::new(Rect::from_center_size(c, Vec2::splat(2.0 * r)), [r; 4])
}
