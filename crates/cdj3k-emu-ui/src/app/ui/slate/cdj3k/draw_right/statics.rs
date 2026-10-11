//! Static labels and chrome for the right section, which holds the modes and
//! the tempo controls. They are cached and rebuilt only when the window scale
//! or the layout changes.

use egui::{Align2, FontId, Pos2, Rect, Stroke, Vec2};

use super::super::draw_cache::ShapeList;
use super::super::{
    collect_bordered_rect_section, DoubleBorderSpec, StrokeSpec, UiScale, COL_BLACK, COL_BTN_TEXT,
    COL_DARK, COL_SILVER,
};
use super::tempo::*;
use super::*;
use crate::app::ui::draw_vinyl_speed::collect_vinyl_speed_endstops;
use crate::app::ui::draw_vinyl_speed::{
    VINYL_SPEED_KNOB_RADIUS_REF, VINYL_SPEED_LABEL_GAP_REF, VINYL_SPEED_SIZE_SCALE,
    VINYL_SPEED_TOOTH_AMP_REF, VINYL_SPEED_TOP_LABEL_FONT_SIZE,
};

pub(super) fn collect_right_statics(list: &mut ShapeList, ctx: &egui::Context, layout: &UiScale) {
    collect_jog_mode_chrome(list, layout);
    collect_inst_doubles(list, ctx, layout);
    collect_sync_chrome(list, layout);
    collect_tempo_labels(list, ctx, layout);
    collect_tempo_slider(list, ctx, layout);
    collect_vinyl_speed_legend(list, ctx, layout);
    collect_vinyl_speed_endstops(list, layout, VINYL_SPEED_PLACE);
    collect_bottom_label(list, ctx, layout);
}

/// Collects the bordered background of the JOG MODE combo.
fn collect_jog_mode_chrome(list: &mut ShapeList, layout: &UiScale) {
    let combo_ref = jog_mode_boxes().0;
    let combo_screen = layout.sr(
        combo_ref.left(),
        combo_ref.top(),
        combo_ref.width(),
        combo_ref.height(),
    );
    collect_bordered_rect_section(
        list,
        combo_screen,
        Some(layout.sc(JOG_MODE_BORDER_ROUNDING)),
        None,
        DoubleBorderSpec::from_strokes(
            StrokeSpec {
                width: layout.sc(3.0),
                color: COL_SILVER,
            },
            StrokeSpec {
                width: layout.sc(2.0),
                color: COL_SILVER,
            },
        ),
    );
}

/// Collects the "INST. DOUBLES" legend and the dash to its left.
fn collect_inst_doubles(list: &mut ShapeList, ctx: &egui::Context, layout: &UiScale) {
    let col = MODES_COL_REF;
    let left_ref = col.left() + col.width() * SYNC_INST_DOUBLES_U;
    let cy_ref = col.top() + col.height() * SYNC_INST_DOUBLES_V;
    list.rect_filled(
        Rect::from_center_size(
            layout.sp(left_ref - SYNC_INST_DOUBLES_TAB_GAP, cy_ref),
            Vec2::new(layout.sc(LEGEND_TAB_W), layout.sc(LEGEND_TAB_H)),
        ),
        0.0,
        COL_BTN_TEXT,
    );
    list.text(
        ctx,
        layout.sp(left_ref, cy_ref),
        Align2::LEFT_CENTER,
        "INST. DOUBLES",
        FontId::proportional(layout.sc(SYNC_INST_DOUBLES_FONT_SIZE)),
        COL_BTN_TEXT,
    );
}

/// Collects the bordered background of the sync section.
fn collect_sync_chrome(list: &mut ShapeList, layout: &UiScale) {
    let section_ref = sync_boxes().0;
    let section_screen = layout.sr(
        section_ref.left(),
        section_ref.top(),
        section_ref.width(),
        section_ref.height(),
    );
    collect_bordered_rect_section(
        list,
        section_screen,
        Some(layout.sc(SYNC_BORDER_ROUNDING)),
        Some(COL_BLACK),
        DoubleBorderSpec::from_strokes(
            StrokeSpec {
                width: layout.sc(3.0),
                color: COL_SILVER,
            },
            StrokeSpec {
                width: layout.sc(2.0),
                color: COL_SILVER,
            },
        ),
    );
}

/// Collects the TEMPO range and MASTER TEMPO labels.
fn collect_tempo_labels(list: &mut ShapeList, ctx: &egui::Context, layout: &UiScale) {
    let col = MODES_COL_REF;
    list.text(
        ctx,
        layout.sp_in_rect(col, 0.5, TEMPO_RANGE_LABEL_V),
        Align2::CENTER_CENTER,
        "TEMPO",
        FontId::proportional(layout.sc(TEMPO_RANGE_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );
    list.text(
        ctx,
        layout.sp_in_rect(col, 0.5, TEMPO_RANGE_SUBLABEL_V),
        Align2::CENTER_CENTER,
        "±6/±10/±16/WIDE",
        FontId::proportional(layout.sc(TEMPO_RANGE_SUBLABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );

    list.text(
        ctx,
        layout.sp_in_rect(col, 0.5, MASTER_TEMPO_LABEL_V),
        Align2::CENTER_CENTER,
        "MASTER\n TEMPO",
        FontId::proportional(layout.sc(MASTER_TEMPO_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );
}

/// Collects the tempo slider's backdrop, its TEMPO label, its two track bars,
/// its scale and the TEMPO RESET label.
fn collect_tempo_slider(list: &mut ShapeList, ctx: &egui::Context, layout: &UiScale) {
    let col = MODES_COL_REF;
    let col_w = col.width();
    let col_h = col.height();
    let (slider, backdrop) = tempo::tempo_slider_rects(layout);
    collect_bordered_rect_section(
        list,
        backdrop,
        Some(layout.sc(TEMPO_BACKDROP_ROUNDING)),
        Some(COL_BLACK),
        DoubleBorderSpec::from_strokes_with_gap(
            StrokeSpec {
                width: layout.sc(TEMPO_BACKDROP_INNER_STROKE),
                color: COL_SILVER,
            },
            StrokeSpec {
                width: layout.sc(TEMPO_BACKDROP_OUTER_STROKE),
                color: COL_DARK,
            },
            layout.sc(TEMPO_BACKDROP_GAP),
        ),
    );
    list.text(
        ctx,
        layout.sp(
            col.left() + col_w * 0.5,
            col.top() + col_h * TEMPO_SLIDER_LABEL_V,
        ),
        Align2::CENTER_CENTER,
        "TEMPO",
        FontId::proportional(layout.sc(TEMPO_SLIDER_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );
    // The two track bars.
    let track_cx = slider.center().x;
    let bar_half_gap = layout.sc(TEMPO_TRACK_BAR_GAP) * 0.5;
    let bar_inset = layout.sc(TEMPO_TRACK_BAR_INSET);
    let bar_w = layout.sc(TEMPO_TRACK_BAR_W);
    for &x in &[track_cx - bar_half_gap, track_cx + bar_half_gap] {
        list.line_segment(
            [
                Pos2::new(x, backdrop.top() + bar_inset),
                Pos2::new(x, backdrop.bottom() - bar_inset),
            ],
            Stroke::new(bar_w, COL_SILVER),
        );
    }
    collect_tempo_scale(list, ctx, layout, slider);
    let reset_cx = layout
        .sp(col.left() + col_w * TEMPO_RESET_BTN_X_U, col.top())
        .x;
    let zero_y = slider.top() + slider.height() * 0.5;
    let reset_r = layout.sc(TEMPO_RESET_BTN_R);
    list.text(
        ctx,
        Pos2::new(
            reset_cx,
            zero_y + reset_r + layout.sc(TEMPO_RESET_LABEL_GAP),
        ),
        Align2::CENTER_TOP,
        "TEMPO\nRESET",
        FontId::proportional(layout.sc(TEMPO_RESET_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );
}

/// Collects the detents beside the slider and the segments ruled across it.
fn collect_tempo_scale(list: &mut ShapeList, ctx: &egui::Context, layout: &UiScale, slider: Rect) {
    let col = MODES_COL_REF;
    let col_w = col.width();
    let tick_x = layout.sp(col.left() + col_w * TEMPO_TICK_X_U, col.top()).x;
    for i in 0..TEMPO_TICK_COUNT {
        let t = i as f32 / (TEMPO_TICK_COUNT - 1) as f32;
        let y = slider.top() + slider.height() * t;
        let is_gap = i == 1 || i == 24 || i == 26 || i == 49;
        collect_tempo_segment(list, layout, slider, y, i % 5 == 0);
        if !is_gap {
            collect_tempo_detent(list, ctx, layout, Pos2::new(tick_x, y), i);
        }
    }
}

/// Collects the VINYL SPEED ADJ. and TOUCH / BRAKE legends above the knob.
fn collect_vinyl_speed_legend(list: &mut ShapeList, ctx: &egui::Context, layout: &UiScale) {
    let s = |r: f32| layout.sc(r * VINYL_SPEED_SIZE_SCALE);
    let center = VINYL_SPEED_PLACE.center(layout);
    let r_outer = s(VINYL_SPEED_KNOB_RADIUS_REF) + s(VINYL_SPEED_TOOTH_AMP_REF);

    let knob_top_y = center.y - r_outer;
    let sub_label_y = knob_top_y - layout.sc(VINYL_SPEED_LABEL_GAP_REF);
    list.text(
        ctx,
        Pos2::new(center.x, sub_label_y),
        Align2::CENTER_BOTTOM,
        "TOUCH / BRAKE",
        FontId::proportional(layout.sc(VINYL_SPEED_SUB_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );
    list.text(
        ctx,
        Pos2::new(
            center.x,
            sub_label_y - layout.sc(VINYL_SPEED_LABEL_SPACING_REF),
        ),
        Align2::CENTER_BOTTOM,
        "VINYL SPEED ADJ.",
        FontId::proportional(layout.sc(VINYL_SPEED_TOP_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );
}

fn collect_bottom_label(list: &mut ShapeList, ctx: &egui::Context, layout: &UiScale) {
    let col = MODES_COL_REF;
    list.text(
        ctx,
        layout.sp(
            col.left() + col.width() * BOTTOM_LABEL_TYPE_X,
            col.top() + col.height() * BOTTOM_LABEL_TYPE_V,
        ),
        Align2::CENTER_CENTER,
        "MULTI PLAYER",
        FontId::proportional(layout.sc(BOTTOM_LABEL_TYPE_FONT_SIZE)),
        COL_BTN_TEXT,
    );

    list.text(
        ctx,
        layout.sp(
            col.left() + col.width() * BOTTOM_LABEL_SERIE_X,
            col.top() + col.height() * BOTTOM_LABEL_SERIE_V,
        ),
        Align2::CENTER_CENTER,
        "CDJ-3000",
        FontId::proportional(layout.sc(BOTTOM_LABEL_SERIE_FONT_SIZE)),
        COL_BTN_TEXT,
    );
}

/// Collects detent `i` of the side column: the − and + signs at the ends, the
/// 0 at the centre, and a dot everywhere else.
fn collect_tempo_detent(
    list: &mut ShapeList,
    ctx: &egui::Context,
    layout: &UiScale,
    at: Pos2,
    i: usize,
) {
    let label = if i == 0 {
        Some("−")
    } else if i == 25 {
        Some("0")
    } else if i == TEMPO_TICK_COUNT - 1 {
        Some("+")
    } else {
        None
    };
    if let Some(ch) = label {
        list.text(
            ctx,
            at,
            Align2::CENTER_CENTER,
            ch,
            FontId::proportional(layout.sc(TEMPO_TICK_CHAR_FONT_SIZE)),
            COL_BTN_TEXT,
        );
    } else {
        list.circle_filled(at, layout.sc(TEMPO_TICK_DOT_R), COL_BTN_TEXT);
    }
}

/// Collects the segment ruled across the slider at `y`; every fifth segment is
/// a major one.
fn collect_tempo_segment(
    list: &mut ShapeList,
    layout: &UiScale,
    slider: Rect,
    y: f32,
    is_major: bool,
) {
    let track_cx = slider.center().x;
    let seg_half_w = slider.width()
        * 0.5
        * if is_major {
            TEMPO_SEG_MAJOR_W
        } else {
            TEMPO_SEG_NORMAL_W
        };
    let seg_stroke = layout.sc(if is_major {
        TEMPO_SEG_MAJOR_STROKE
    } else {
        TEMPO_SEG_NORMAL_STROKE
    });
    list.line_segment(
        [
            Pos2::new(track_cx - seg_half_w, y),
            Pos2::new(track_cx + seg_half_w, y),
        ],
        Stroke::new(seg_stroke, COL_BTN_TEXT),
    );
}
