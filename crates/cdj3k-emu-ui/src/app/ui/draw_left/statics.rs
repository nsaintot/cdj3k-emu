//! Static labels and chrome for the left (transport) section. Cached and
//! rebuilt only when the window scale or layout changes.

use egui::{FontId, Pos2, Rect, Shape, Stroke, Vec2};

use super::super::draw_cache::ShapeList;
use super::super::{
    collect_back_double_circle_border, collect_bordered_rect_section, DoubleBorderSpec, StrokeSpec,
    UiScale, COL_BLACK, COL_BTN_OUTLINED_WHITE, COL_BTN_TEXT, COL_DARK, COL_SILVER,
};
use super::direction::{
    DIRECTION_BORDER_ROUNDING, DIRECTION_COMBO_HALF_H, DIRECTION_COMBO_HALF_W, DIRECTION_COMBO_V,
    PERF_V_DIRECTION_LABEL,
};
use super::*;

pub(super) fn collect_left_statics(list: &mut ShapeList, ctx: &egui::Context, layout: &UiScale) {
    use egui::Align2;

    // ── USB STOP icon + label (button is interactive, drawn separately) ────
    {
        let col = PERF_TRANSPORT_COL_REF;
        let cx_ref = col.left() + col.width() * PERF_U_USB_STOP;

        // Indicator dash above the laptop icon.
        let led_cy_ref = col.top() + col.height() * PERF_V_USB_STOP_PRE_LABEL;
        let led_rect = Rect::from_center_size(
            layout.sp(cx_ref, led_cy_ref),
            Vec2::new(
                layout.sc(PERF_USB_STOP_LED_W),
                layout.sc(PERF_USB_STOP_LED_H),
            ),
        );
        list.rect_filled(led_rect, 0.0, COL_BTN_TEXT);

        // Screen rectangle: black fill, white stroke.
        let screen_cy_ref = col.top() + col.height() * PERF_V_USB_STOP_ICON_SCREEN;
        let screen_rect = Rect::from_center_size(
            layout.sp(cx_ref, screen_cy_ref),
            Vec2::new(
                layout.sc(PERF_USB_STOP_SCREEN_W),
                layout.sc(PERF_USB_STOP_SCREEN_H),
            ),
        );
        list.rect_filled(screen_rect, 0.0, COL_BLACK);
        list.rect_stroke(
            screen_rect,
            0.0,
            Stroke::new(layout.sc(PERF_USB_STOP_SCREEN_STROKE), COL_BTN_TEXT),
        );

        // Trapezoid base with small notch on the bottom center (keyboard + pad).
        let base_cy_ref = col.top() + col.height() * PERF_V_USB_STOP_ICON_BASE;
        let base_top_y = base_cy_ref - PERF_USB_STOP_BASE_H * 0.5;
        let base_bot_y = base_cy_ref + PERF_USB_STOP_BASE_H * 0.5;
        let top_hw = PERF_USB_STOP_BASE_TOP_W * 0.5;
        let bot_hw = PERF_USB_STOP_BASE_BOT_W * 0.5;
        let notch_hw = PERF_USB_STOP_BASE_NOTCH_W * 0.5;
        let notch_dy = PERF_USB_STOP_BASE_NOTCH_H;

        // Trapezoid (convex) - wider at the bottom, like a keyboard.
        let trap = vec![
            layout.sp(cx_ref - top_hw, base_top_y),
            layout.sp(cx_ref + top_hw, base_top_y),
            layout.sp(cx_ref + bot_hw, base_bot_y),
            layout.sp(cx_ref - bot_hw, base_bot_y),
        ];
        list.add(Shape::convex_polygon(trap, COL_BTN_TEXT, Stroke::NONE));

        // Notch tab inset into the trapezoid's bottom-center (touchpad).
        let notch_rect = Rect::from_min_max(
            layout.sp(cx_ref - notch_hw, base_bot_y - notch_dy),
            layout.sp(cx_ref + notch_hw, base_bot_y),
        );
        list.rect_filled(notch_rect, 0.0, COL_DARK);

        // "USB STOP" label (two lines).
        list.text(
            ctx,
            layout.sp_in_rect(
                PERF_TRANSPORT_COL_REF,
                PERF_U_USB_STOP,
                PERF_V_USB_STOP_LABEL,
            ),
            Align2::CENTER_CENTER,
            " USB\nSTOP",
            FontId::proportional(layout.sc(PERF_USB_STOP_LABEL_FONT_SIZE)),
            COL_BTN_TEXT,
        );
    }

    // ── USB trident logo (left of the USB STOP button) ─────────────────────
    {
        let col = PERF_TRANSPORT_COL_REF;
        let cx_ref = col.left() + col.width() * PERF_U_USB_SIGN;
        let cy_ref = col.top() + col.height() * PERF_V_USB_SIGN;
        let stroke = Stroke::new(layout.sc(PERF_USB_SIGN_STROKE), COL_BTN_TEXT);

        // Trunk: horizontal line spanning the full width.
        let half_w = PERF_USB_SIGN_W * 0.5;
        let trunk_left = layout.sp(cx_ref - half_w, cy_ref);
        let trunk_right = layout.sp(cx_ref + half_w - PERF_USB_SIGN_ARROW_LEN, cy_ref);
        list.line_segment([trunk_left, trunk_right], stroke);

        // Tail dot (left end of the trunk).
        list.circle_filled(
            layout.sp(cx_ref - half_w, cy_ref),
            layout.sc(PERF_USB_SIGN_TAIL_DOT_R),
            COL_BTN_TEXT,
        );

        // Arrow head (right end): filled triangle pointing right.
        let arrow_base_x = cx_ref + half_w - PERF_USB_SIGN_ARROW_LEN;
        let arrow_tip_x = cx_ref + half_w;
        let arrow_pts = vec![
            layout.sp(arrow_base_x, cy_ref - PERF_USB_SIGN_ARROW_HALF),
            layout.sp(arrow_tip_x, cy_ref),
            layout.sp(arrow_base_x, cy_ref + PERF_USB_SIGN_ARROW_HALF),
        ];
        list.add(Shape::convex_polygon(arrow_pts, COL_BTN_TEXT, Stroke::NONE));

        // Upper fork: trunk → diagonal up → horizontal → filled circle at the end.
        let fork_x_start = cx_ref + PERF_USB_SIGN_TOP_FORK_X_START;
        let fork_x_diag_end = cx_ref + PERF_USB_SIGN_TOP_FORK_X_DIAG_END;
        let fork_x_end = cx_ref + PERF_USB_SIGN_TOP_FORK_X_END;
        let fork_top_y = cy_ref - PERF_USB_SIGN_FORK_OFF_Y;
        list.line_segment(
            [
                layout.sp(fork_x_start, cy_ref),
                layout.sp(fork_x_diag_end, fork_top_y),
            ],
            stroke,
        );
        list.line_segment(
            [
                layout.sp(fork_x_diag_end, fork_top_y),
                layout.sp(fork_x_end, fork_top_y),
            ],
            stroke,
        );
        let cr = PERF_USB_SIGN_CIRCLE_R;
        list.circle_filled(
            layout.sp(fork_x_end + cr, fork_top_y),
            layout.sc(cr),
            COL_BTN_TEXT,
        );

        // Lower fork: trunk → diagonal down → horizontal → filled square at the end.
        let lfork_x_start = cx_ref + PERF_USB_SIGN_BOT_FORK_X_START;
        let lfork_x_diag_end = cx_ref + PERF_USB_SIGN_BOT_FORK_X_DIAG_END;
        let lfork_x_end = cx_ref + PERF_USB_SIGN_BOT_FORK_X_END;
        let lfork_bot_y = cy_ref + PERF_USB_SIGN_FORK_OFF_Y;
        list.line_segment(
            [
                layout.sp(lfork_x_start, cy_ref),
                layout.sp(lfork_x_diag_end, lfork_bot_y),
            ],
            stroke,
        );
        list.line_segment(
            [
                layout.sp(lfork_x_diag_end, lfork_bot_y),
                layout.sp(lfork_x_end, lfork_bot_y),
            ],
            stroke,
        );
        let sq = PERF_USB_SIGN_SQUARE_S;
        let sq_rect = Rect::from_center_size(
            layout.sp(lfork_x_end + sq * 0.5, lfork_bot_y),
            Vec2::new(layout.sc(sq), layout.sc(sq)),
        );
        list.rect_filled(sq_rect, 0.0, COL_BTN_TEXT);
    }

    list.text(
        ctx,
        layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            0.265,
            PERF_V_TIME_MODE_AUTO_CUE_LABEL,
        ),
        Align2::CENTER_CENTER,
        "      ▪\n TIME\nMODE",
        FontId::proportional(layout.sc(PERF_TIME_MODE_AUTO_CUE_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );
    list.text(
        ctx,
        layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            0.66,
            PERF_V_TIME_MODE_AUTO_CUE_LABEL,
        ),
        Align2::CENTER_CENTER,
        "   —\nAUTO\n CUE",
        FontId::proportional(layout.sc(PERF_TIME_MODE_AUTO_CUE_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );
    list.text(
        ctx,
        layout.sp_in_rect(PERF_TRANSPORT_COL_REF, 0.555, PERF_V_BEAT_JUMP_LABEL),
        Align2::CENTER_CENTER,
        "BEAT\nJUMP",
        FontId::proportional(layout.sc(PERF_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );
    list.text(
        ctx,
        layout.sp_in_rect(PERF_TRANSPORT_COL_REF, 0.5, PERF_V_TRACK_SEARCH_LABEL),
        Align2::CENTER_CENTER,
        "TRACK SEARCH",
        FontId::proportional(layout.sc(PERF_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );
    // Back-capsule border behind TRACK SEARCH paired buttons.
    {
        let tl = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            0.5 - PERF_PAIR_U_OFF_FRAC,
            PERF_V_TRACK_SEARCH_CTRLS,
        );
        let tr = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            0.5 + PERF_PAIR_U_OFF_FRAC,
            PERF_V_TRACK_SEARCH_CTRLS,
        );
        collect_back_double_circle_border(
            list,
            tl,
            tr,
            layout.sc(PERF_DOUBLE_CIRCLE_RING_R),
            layout.sc(PERF_DOUBLE_CIRCLE_RING_STROKE),
            COL_DARK,
            COL_SILVER,
        );
    }
    list.text(
        ctx,
        layout.sp_in_rect(PERF_TRANSPORT_COL_REF, 0.5, PERF_V_SEARCH_LABEL),
        Align2::CENTER_CENTER,
        "SEARCH",
        FontId::proportional(layout.sc(PERF_DOUBLE_CIRCLE_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );
    // Back-capsule border behind SEARCH paired buttons.
    {
        let sl = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            0.5 - PERF_PAIR_U_OFF_FRAC,
            PERF_V_SEARCH_CTRLS,
        );
        let sr = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            0.5 + PERF_PAIR_U_OFF_FRAC,
            PERF_V_SEARCH_CTRLS,
        );
        collect_back_double_circle_border(
            list,
            sl,
            sr,
            layout.sc(PERF_DOUBLE_CIRCLE_RING_R),
            layout.sc(PERF_DOUBLE_CIRCLE_RING_STROKE),
            COL_DARK,
            COL_SILVER,
        );
    }

    // ── CALL / DELETE chrome + labels ────────────────────────────────────────
    {
        let col = PERF_TRANSPORT_COL_REF;
        let cy_ref = col.top() + col.height() * PERF_V_CALL_DELETE;
        let btn_cx_ref = col.left() + col.width() * PERF_CALL_DELETE_BTN_U;

        // Outer bordered container.
        let outer_ref = Rect::from_center_size(
            Pos2::new(btn_cx_ref, cy_ref),
            Vec2::new(PERF_CALL_DELETE_OUTER_W, PERF_CALL_DELETE_OUTER_H),
        );
        let outer_screen = Rect::from_min_max(
            layout.sp(outer_ref.left(), outer_ref.top()),
            layout.sp(outer_ref.right(), outer_ref.bottom()),
        );
        collect_bordered_rect_section(
            list,
            outer_screen,
            Some(layout.sc(PERF_CALL_DELETE_OUTER_ROUNDING)),
            Some(COL_BLACK),
            DoubleBorderSpec::from_strokes_with_gap(
                StrokeSpec {
                    width: layout.sc(PERF_CALL_DELETE_BTN_INNER_STROKE),
                    color: COL_SILVER,
                },
                StrokeSpec {
                    width: layout.sc(PERF_CALL_DELETE_BTN_OUTER_STROKE),
                    color: COL_DARK,
                },
                layout.sc(2.0),
            ),
        );

        let label_y_ref = cy_ref - PERF_CALL_DELETE_LABEL_NUDGE_Y;

        // "▪ CALL/" label.
        list.text(
            ctx,
            layout.sp(
                col.left() + col.width() * PERF_CALL_DELETE_LEFT_LABEL_U,
                label_y_ref,
            ),
            Align2::CENTER_CENTER,
            "▪ CALL/",
            FontId::proportional(layout.sc(PERF_LABEL_DELETE_FONT_SIZE)),
            COL_BTN_TEXT,
        );

        // "DELETE" white-pill sublabel.
        let del_center = layout.sp(
            col.left() + col.width() * PERF_CALL_DELETE_RIGHT_LABEL_U,
            label_y_ref,
        );
        let del_sublabel_size = Vec2::new(
            layout.sc(PERF_CALL_DELETE_SUBLABEL_W),
            layout.sc(PERF_CALL_DELETE_SUBLABEL_H),
        );
        list.rect_filled(
            Rect::from_center_size(del_center, del_sublabel_size),
            layout.sc(PERF_CALL_DELETE_SUBLABEL_ROUNDING),
            COL_BTN_OUTLINED_WHITE,
        );
        list.text(
            ctx,
            del_center,
            Align2::CENTER_CENTER,
            "DELETE",
            FontId::proportional(layout.sc(PERF_LABEL_DELETE_FONT_SIZE)),
            COL_BLACK,
        );

        // Segment to the right of the DELETE label.
        let seg_start = layout.sp(
            col.left() + col.width() * PERF_CALL_DELETE_SEGMENT_U,
            cy_ref,
        );
        let seg_end = Pos2::new(
            seg_start.x + layout.sc(PERF_CALL_DELETE_SEGMENT_LENGTH),
            seg_start.y,
        );
        list.line_segment(
            [seg_start, seg_end],
            Stroke::new(layout.sc(PERF_CALL_DELETE_SEGMENT_THICKNESS), COL_BTN_TEXT),
        );
    }

    // ── Loop row: "IN/CUE" / "OUT" / "RELOOP/EXIT" labels + 3 separator lines ─
    {
        list.text(
            ctx,
            layout.sp_in_rect(PERF_TRANSPORT_COL_REF, 0.320, PERF_V_LOOP_IN_OUT_LABEL),
            Align2::CENTER_CENTER,
            "IN/CUE",
            FontId::proportional(layout.sc(PERF_LABEL_FONT_SIZE)),
            COL_BTN_TEXT,
        );
        list.text(
            ctx,
            layout.sp_in_rect(PERF_TRANSPORT_COL_REF, 0.8, PERF_V_LOOP_IN_OUT_LABEL),
            Align2::CENTER_CENTER,
            "OUT",
            FontId::proportional(layout.sc(PERF_LABEL_FONT_SIZE)),
            COL_BTN_TEXT,
        );
        list.text(
            ctx,
            layout.sp_in_rect(PERF_TRANSPORT_COL_REF, 1.37, PERF_V_LOOP_IN_OUT_LABEL),
            Align2::CENTER_CENTER,
            "RELOOP/EXIT",
            FontId::proportional(layout.sc(PERF_LABEL_FONT_SIZE)),
            COL_BTN_TEXT,
        );

        let line_stroke = Stroke::new(layout.sc(PERF_LOOP_LINE_THICKNESS), COL_BTN_TEXT);
        for &(u, len) in &[
            (0.51_f32, PERF_LOOP_LINE_1_LENGTH),
            (1.0, PERF_LOOP_LINE_2_LENGTH),
            (1.55, PERF_LOOP_LINE_3_LENGTH),
        ] {
            let s = layout.sp_in_rect(PERF_TRANSPORT_COL_REF, u, PERF_V_LOOP_LINE);
            let e = Pos2::new(s.x + layout.sc(len), s.y);
            list.line_segment([s, e], line_stroke);
        }
    }

    // ── "LOOP" + IN/OUT ADJUST sublabels ─────────────────────────────────────
    {
        list.text(
            ctx,
            layout.sp_in_rect(PERF_TRANSPORT_COL_REF, 1.95, PERF_V_LOOP_LABEL),
            Align2::CENTER_CENTER,
            "LOOP",
            FontId::proportional(layout.sc(PERF_LABEL_FONT_SIZE)),
            COL_BTN_TEXT,
        );

        let in_adj = layout.sp_in_rect(PERF_TRANSPORT_COL_REF, 0.317, PERF_V_LOOP_SUBLABEL);
        let out_adj = layout.sp_in_rect(PERF_TRANSPORT_COL_REF, 0.79, PERF_V_LOOP_SUBLABEL);
        let in_size = Vec2::new(
            layout.sc(PERF_LOOP_SUBLABEL_IN_W),
            layout.sc(PERF_LOOP_SUBLABEL_H),
        );
        let out_size = Vec2::new(
            layout.sc(PERF_LOOP_SUBLABEL_OUT_W),
            layout.sc(PERF_LOOP_SUBLABEL_H),
        );
        let rounding = layout.sc(PERF_LOOP_SUBLABEL_ROUNDING);

        list.rect_filled(
            Rect::from_center_size(in_adj, in_size),
            rounding,
            COL_BTN_OUTLINED_WHITE,
        );
        list.rect_filled(
            Rect::from_center_size(out_adj, out_size),
            rounding,
            COL_BTN_OUTLINED_WHITE,
        );
        list.text(
            ctx,
            in_adj,
            Align2::CENTER_CENTER,
            "IN ADJUST",
            FontId::proportional(layout.sc(PERF_LABEL_FONT_SIZE)),
            COL_BLACK,
        );
        list.text(
            ctx,
            out_adj,
            Align2::CENTER_CENTER,
            "OUT ADJUST",
            FontId::proportional(layout.sc(PERF_LABEL_FONT_SIZE)),
            COL_BLACK,
        );
    }

    // ── BEAT LOOP separator lines + label + 1/2X / 2X sublabels ──────────────
    {
        let line_stroke = Stroke::new(layout.sc(PERF_BEAT_LOOP_LINE_THICKNESS), COL_BTN_TEXT);
        for &u in &[0.45_f32, 0.95] {
            let s = layout.sp_in_rect(PERF_TRANSPORT_COL_REF, u, PERF_V_BEAT_LOOP_LINE);
            let e = Pos2::new(s.x + layout.sc(PERF_BEAT_LOOP_LINE_LENGTH), s.y);
            list.line_segment([s, e], line_stroke);
        }

        list.text(
            ctx,
            layout.sp_in_rect(PERF_TRANSPORT_COL_REF, 1.36, PERF_V_BEAT_LOOP_LABEL),
            Align2::CENTER_CENTER,
            "BEAT LOOP",
            FontId::proportional(layout.sc(PERF_LABEL_FONT_SIZE)),
            COL_BTN_TEXT,
        );

        let half = layout.sp_in_rect(PERF_TRANSPORT_COL_REF, 0.317, PERF_V_BEAT_LOOP_SUBLABEL);
        let twox = layout.sp_in_rect(PERF_TRANSPORT_COL_REF, 0.79, PERF_V_BEAT_LOOP_SUBLABEL);
        let size = Vec2::new(
            layout.sc(PERF_BEAT_LOOP_SUBLABEL_W),
            layout.sc(PERF_BEAT_LOOP_SUBLABEL_H),
        );
        let rounding = layout.sc(PERF_BEAT_LOOP_SUBLABEL_ROUNDING);
        list.rect_filled(
            Rect::from_center_size(half, size),
            rounding,
            COL_BTN_OUTLINED_WHITE,
        );
        list.rect_filled(
            Rect::from_center_size(twox, size),
            rounding,
            COL_BTN_OUTLINED_WHITE,
        );
        list.text(
            ctx,
            half,
            Align2::CENTER_CENTER,
            "1/2X",
            FontId::proportional(layout.sc(PERF_LABEL_FONT_SIZE)),
            COL_BLACK,
        );
        list.text(
            ctx,
            twox,
            Align2::CENTER_CENTER,
            "2X",
            FontId::proportional(layout.sc(PERF_LABEL_FONT_SIZE)),
            COL_BLACK,
        );
    }

    // ── DIRECTION label + combo bordered container ───────────────────────────
    {
        let col = PERF_TRANSPORT_COL_REF;
        let cx_ref = col.left() + col.width() * 0.5;
        let cy_ref = col.top() + col.height() * DIRECTION_COMBO_V;

        list.text(
            ctx,
            layout.sp_in_rect(col, 0.5, PERF_V_DIRECTION_LABEL),
            Align2::CENTER_CENTER,
            "DIRECTION",
            FontId::proportional(layout.sc(PERF_LABEL_FONT_SIZE)),
            COL_BTN_TEXT,
        );

        let combo_screen = Rect::from_min_max(
            layout.sp(
                cx_ref - DIRECTION_COMBO_HALF_W,
                cy_ref - DIRECTION_COMBO_HALF_H,
            ),
            layout.sp(
                cx_ref + DIRECTION_COMBO_HALF_W,
                cy_ref + DIRECTION_COMBO_HALF_H,
            ),
        );
        collect_bordered_rect_section(
            list,
            combo_screen,
            Some(layout.sc(DIRECTION_BORDER_ROUNDING)),
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

    // ── PLAY / PAUSE label (above the play button) ───────────────────────────
    {
        let play_c = layout.sp_in_rect(PERF_TRANSPORT_COL_REF, 0.5, PERF_V_PLAY);
        list.text(
            ctx,
            play_c - Vec2::new(0.0, layout.sc(PERF_PLAY_LABEL_GAP_Y)),
            Align2::CENTER_CENTER,
            "PLAY / PAUSE",
            FontId::proportional(layout.sc(PERF_PLAY_LABEL_FONT_SIZE)),
            COL_BTN_TEXT,
        );
    }
}
