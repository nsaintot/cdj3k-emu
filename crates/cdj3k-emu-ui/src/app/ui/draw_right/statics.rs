//! Static labels and chrome for the right (modes / tempo) section. Cached
//! and rebuilt only when the window scale or layout changes.

use egui::{FontId, Pos2, Rect, Shape, Stroke};

use super::super::draw_cache::ShapeList;
use super::super::{
    collect_bordered_rect_section, DoubleBorderSpec, StrokeSpec, UiScale, COL_BLACK, COL_BTN_TEXT,
    COL_DARK, COL_SILVER,
};
use super::vinyl_speed::{
    VINYL_SPEED_CENTER_U, VINYL_SPEED_CENTER_V, VINYL_SPEED_ENDSHAPE_ARM_W,
    VINYL_SPEED_ENDSHAPE_GAP_Y, VINYL_SPEED_ENDSHAPE_H, VINYL_SPEED_ENDSHAPE_INSET_X,
    VINYL_SPEED_ENDSHAPE_STROKE, VINYL_SPEED_KNOB_RADIUS_REF, VINYL_SPEED_LABEL_GAP_REF,
    VINYL_SPEED_LABEL_SPACING_REF, VINYL_SPEED_RAMP_ARM_FRAC, VINYL_SPEED_SIZE_SCALE,
    VINYL_SPEED_SUB_LABEL_FONT_SIZE, VINYL_SPEED_TICK_ARC_SPAN_RAD,
    VINYL_SPEED_TICK_ARC_START_CW_RAD, VINYL_SPEED_TICK_RING_OFFSET_REF, VINYL_SPEED_TOOTH_AMP_REF,
    VINYL_SPEED_TOP_LABEL_FONT_SIZE,
};
use super::*;

pub(super) fn collect_right_statics(list: &mut ShapeList, ctx: &egui::Context, layout: &UiScale) {
    use egui::Align2;
    let col = MODES_COL_REF;

    // ── JOG MODE combo bordered background ────────────────────────────────────
    {
        let center_x = col.left() + col.width() * 0.55;
        let cy_ref = col.top() + col.height() * JOG_MODE_V;
        let btn_h = JOG_MODE_BTN_WIDTH / JOG_MODE_MODE_ASPECT_RATIO;
        let vinyl_btn_left = center_x - JOG_MODE_BTN_WIDTH - JOG_MODE_BTN_GAP_L * 0.45;
        let vinyl_btn_top = cy_ref - btn_h * 0.5;
        let jog_btn_left = center_x + JOG_MODE_BTN_GAP_R * 0.5;
        let combo_ref = Rect::from_min_max(
            Pos2::new(
                vinyl_btn_left - JOG_MODE_BORDER_PAD_L,
                vinyl_btn_top - JOG_MODE_BORDER_PAD,
            ),
            Pos2::new(
                jog_btn_left + JOG_MODE_BTN_WIDTH + JOG_MODE_BORDER_PAD_R,
                cy_ref + btn_h * 0.5 + JOG_MODE_BORDER_PAD,
            ),
        );
        let combo_screen = Rect::from_min_max(
            layout.sp(combo_ref.left(), combo_ref.top()),
            layout.sp(combo_ref.right(), combo_ref.bottom()),
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

    // ── "- INST. DOUBLES" label ────────────────────────────────────────────────
    list.text(
        ctx,
        layout.sp_in_rect(col, 0.5, SYNC_INST_DOUBLES_V),
        Align2::CENTER_CENTER,
        "— INST. DOUBLES",
        FontId::proportional(layout.sc(SYNC_INST_DOUBLES_FONT_SIZE)),
        COL_BTN_TEXT,
    );

    // ── Sync section bordered background ──────────────────────────────────────
    {
        let center_x = col.left() + col.width() * 0.5;
        let cy_ref = col.top() + col.height() * SYNC_V;
        let sync_btn_h = SYNC_BTN_WIDTH / SYNC_BTN_ASPECT;
        let bs_left = center_x - SYNC_BTN_WIDTH - SYNC_BTN_GAP * 0.5;
        let bs_top = cy_ref - sync_btn_h * 0.5;
        let master_left = center_x + SYNC_BTN_GAP * 0.5;
        let ks_btn_h = KEY_SYNC_BTN_WIDTH / KEY_SYNC_BTN_ASPECT_RATIO;
        let ks_cy = cy_ref + sync_btn_h * 0.5 + col.height() * KEY_SYNC_V_OFFSET;
        let section_ref = Rect::from_min_max(
            Pos2::new(bs_left - SYNC_BORDER_PAD_X, bs_top - SYNC_BORDER_PAD_Y),
            Pos2::new(
                master_left + SYNC_BTN_WIDTH + SYNC_BORDER_PAD_X,
                ks_cy + ks_btn_h + SYNC_BORDER_PAD_Y,
            ),
        );
        let section_screen = Rect::from_min_max(
            layout.sp(section_ref.left(), section_ref.top()),
            layout.sp(section_ref.right(), section_ref.bottom()),
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

    // ── TEMPO range labels ─────────────────────────────────────────────────────
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

    // ── MASTER TEMPO label ─────────────────────────────────────────────────────
    list.text(
        ctx,
        layout.sp_in_rect(col, 0.5, MASTER_TEMPO_LABEL_V),
        Align2::CENTER_CENTER,
        "MASTER\n TEMPO",
        FontId::proportional(layout.sc(MASTER_TEMPO_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );

    // ── Tempo slider: backdrop, label, track bars, tick marks & segments ───────
    {
        let col_w = col.width();
        let col_h = col.height();
        let slider_top = col.top() + col_h * TEMPO_SLIDER_V_TOP;
        let slider_bot = col.top() + col_h * TEMPO_SLIDER_V_BOT;
        let slider_left = col.left() + col_w * TEMPO_SLIDER_LEFT_U;
        let slider_right = col.left() + col_w * TEMPO_SLIDER_RIGHT_U;
        let slider = Rect::from_min_max(
            layout.sp(slider_left, slider_top),
            layout.sp(slider_right, slider_bot),
        );
        let backdrop = Rect::from_min_max(
            layout.sp(
                slider_left - TEMPO_BACKDROP_PAD_X,
                slider_top - TEMPO_BACKDROP_PAD_Y,
            ),
            layout.sp(
                slider_right + TEMPO_BACKDROP_PAD_X,
                slider_bot + TEMPO_BACKDROP_PAD_Y,
            ),
        );
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
        // Double track bars
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
        // Tick marks + horizontal segments
        let tick_x = layout.sp(col.left() + col_w * TEMPO_TICK_X_U, slider_top).x;
        for i in 0..TEMPO_TICK_COUNT {
            let t = i as f32 / (TEMPO_TICK_COUNT - 1) as f32;
            let y = slider.top() + slider.height() * t;
            let is_gap = i == 1 || i == 24 || i == 26 || i == 49;
            let is_major = i % 5 == 0;
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
            if !is_gap {
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
                        Pos2::new(tick_x, y),
                        Align2::CENTER_CENTER,
                        ch,
                        FontId::proportional(layout.sc(TEMPO_TICK_CHAR_FONT_SIZE)),
                        COL_BTN_TEXT,
                    );
                } else {
                    list.circle_filled(
                        Pos2::new(tick_x, y),
                        layout.sc(TEMPO_TICK_DOT_R),
                        COL_BTN_TEXT,
                    );
                }
            }
        }
        // TEMPO RESET label (static - always the same text)
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

    // ── VINYL SPEED ADJ. labels + endstop brackets ────────────────────────────
    {
        let s = |r: f32| layout.sc(r * VINYL_SPEED_SIZE_SCALE);
        let center = layout.sp_in_rect(col, VINYL_SPEED_CENTER_U, VINYL_SPEED_CENTER_V);
        let r_outer = s(VINYL_SPEED_KNOB_RADIUS_REF) + s(VINYL_SPEED_TOOTH_AMP_REF);
        let tick_ring_r = r_outer + s(VINYL_SPEED_TICK_RING_OFFSET_REF);

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

        // Endstop brackets at the arc endpoints.
        let min_angle = VINYL_SPEED_TICK_ARC_START_CW_RAD;
        let max_angle = VINYL_SPEED_TICK_ARC_START_CW_RAD + VINYL_SPEED_TICK_ARC_SPAN_RAD;
        let min_tx = center.x + min_angle.sin() * tick_ring_r;
        let min_ty = center.y - min_angle.cos() * tick_ring_r;
        let max_tx = center.x + max_angle.sin() * tick_ring_r;
        let max_ty = center.y - max_angle.cos() * tick_ring_r;

        let aw = s(VINYL_SPEED_ENDSHAPE_ARM_W);
        let eh = s(VINYL_SPEED_ENDSHAPE_H);
        let egap = s(VINYL_SPEED_ENDSHAPE_GAP_Y);
        let estroke = Stroke::new(s(VINYL_SPEED_ENDSHAPE_STROKE), COL_BTN_TEXT);
        let inset = s(VINYL_SPEED_ENDSHAPE_INSET_X);

        // Min shape (stair).
        let sx = min_tx + inset;
        let sy = min_ty + egap;
        list.add(Shape::line(
            vec![
                Pos2::new(sx, sy),
                Pos2::new(sx + aw, sy),
                Pos2::new(sx + aw, sy + eh),
                Pos2::new(sx + aw * 2.0, sy + eh),
            ],
            estroke,
        ));

        // Max shape (ramp).
        let ramp_total = aw * 2.0;
        let ramp_arm = ramp_total * VINYL_SPEED_RAMP_ARM_FRAC;
        let rx = max_tx - ramp_total - inset;
        let ry = max_ty + egap;
        list.add(Shape::line(
            vec![
                Pos2::new(rx, ry),
                Pos2::new(rx + ramp_arm, ry),
                Pos2::new(rx + ramp_total - ramp_arm, ry + eh),
                Pos2::new(rx + ramp_total, ry + eh),
            ],
            estroke,
        ));
    }

    // ── Bottom label ──────────────────────────────────────────────────────────
    {
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
            crate::app::ui::DEVICE_NAME,
            FontId::proportional(layout.sc(BOTTOM_LABEL_SERIE_FONT_SIZE)),
            COL_BTN_TEXT,
        );
    }
}
