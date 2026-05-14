//! Cached static jog wheel geometry - outer rings + inner disk + JOG ADJUST
//! gear/indicator. Consumed by the jog static cache.

use egui::{Align2, Color32, FontId, Pos2, Shape, Stroke, Vec2};

use super::super::draw_cache::ShapeList;
use super::super::{
    draw_rotary_control_collect, paint_double_circle_ring_collect, DoubleBorderSpec,
    RotaryControlSpec, RotaryGearSpec, RotaryIndicatorSpec, RotaryTickSpec, StrokeSpec, UiScale,
    COL_BTN_TEXT, COL_JOG_BODY, COL_SILVER, COL_WHITE,
};
use super::*;

// ── FWD / REV nav-arrow arcs (owned by this module) ──────────────────────────
// Two short curved arrows just outside the jog ring at compass 135° (FWD, CW
// motion) and 225° (REV, CCW motion). Both share a single virtual circle of
// radius `JOG_NAV_ARC_RADIUS_REF`, jog-centered; only ~`JOG_NAV_ARC_LEN_REF` of
// arc length is visible per side.
pub(super) const JOG_NAV_ARC_RADIUS_REF: f32 = 1115.0;
/// Length of the stroked arc body (excludes the arrowhead). Tune freely.
pub(super) const JOG_NAV_ARC_LEN_REF: f32 = 275.0;
/// Arrowhead length along the arc tangent (V1 apex → V2 base on the arc).
pub(super) const JOG_NAV_ARC_HEAD_LEN_REF: f32 = 50.0;
/// Arrowhead width perpendicular to the arc, outward from the jog (V2 → V3).
pub(super) const JOG_NAV_ARC_HEAD_WIDTH_REF: f32 = 14.0;
pub(super) const JOG_NAV_ARC_STROKE_REF: f32 = 4.0;

pub(super) const JOG_NAV_ARC_TIP_FWD_DEG: f32 = 144.5;
pub(super) const JOG_NAV_ARC_TIP_REV_DEG: f32 = 215.5;

// REV / FWD label placement under each arrow.
pub(super) const JOG_NAV_ARROW_LABEL_LEFT_X: f32 = 0.13;
pub(super) const JOG_NAV_ARROW_LABEL_RIGHT_X: f32 = 0.865;
pub(super) const JOG_NAV_ARROW_LABEL_V: f32 = 0.902;
pub(super) const JOG_NAV_ARROW_LABEL_FONT_SIZE: f32 = 34.0;

/// Compass-bearing unit vector (0° = north, increasing CW) in egui screen space (Y down).
#[inline]
fn compass_unit(deg: f32) -> Vec2 {
    let r = deg.to_radians();
    Vec2::new(r.sin(), -r.cos())
}

/// Draw one curved nav arrow (arc body + asymmetric arrowhead at the tip).
/// `tip_deg` is the compass-bearing tip position; `motion_cw` selects the
/// rotation direction the arrow indicates (CW = body extends toward smaller
/// compass angles; CCW = body extends toward larger compass angles).
fn append_jog_nav_arrow(
    out: &mut ShapeList,
    center: Pos2,
    tip_deg: f32,
    motion_cw: bool,
    radius_px: f32,
    arc_len_px: f32,
    head_len_px: f32,
    head_width_px: f32,
    stroke_px: f32,
    color: Color32,
) {
    // Body spans backward from the tip (opposite of the indicated rotation),
    // and stops at the arrowhead's base so the apex stays pointy.
    let span_deg = (arc_len_px / radius_px).to_degrees();
    let head_span_deg = (head_len_px / radius_px).to_degrees();
    let dir = if motion_cw { -1.0 } else { 1.0 };
    let body_start_deg = tip_deg + dir * span_deg;
    let body_end_deg = tip_deg + dir * head_span_deg;

    const SEGMENTS: usize = 16;
    let arc_pts: Vec<Pos2> = (0..=SEGMENTS)
        .map(|i| {
            let t = i as f32 / SEGMENTS as f32;
            let theta = body_start_deg + (body_end_deg - body_start_deg) * t;
            center + compass_unit(theta) * radius_px
        })
        .collect();
    out.add(Shape::line(arc_pts, Stroke::new(stroke_px, color)));

    // Arrowhead: right-triangle with the apex at the tip and the V1→V2 leg
    // lying on the arc body's *inner* edge (radius - stroke/2).
    let inner_r = radius_px - stroke_px * 0.5;
    let v1 = center + compass_unit(tip_deg) * inner_r;
    let v2 = center + compass_unit(body_end_deg) * inner_r;
    let v3 = v2 + compass_unit(body_end_deg) * head_width_px;
    out.add(Shape::convex_polygon(vec![v1, v2, v3], color, Stroke::NONE));
}

pub(super) fn collect_jog_statics(
    list: &mut ShapeList,
    ctx: &egui::Context,
    layout: &UiScale,
    center: Pos2,
) {
    let radius = layout.sc(JOG_NAV_ARC_RADIUS_REF);
    let arc_len = layout.sc(JOG_NAV_ARC_LEN_REF);
    let head_len = layout.sc(JOG_NAV_ARC_HEAD_LEN_REF);
    let head_width = layout.sc(JOG_NAV_ARC_HEAD_WIDTH_REF);
    let stroke = layout.sc(JOG_NAV_ARC_STROKE_REF);
    let color = COL_BTN_TEXT;

    append_jog_nav_arrow(
        list,
        center,
        JOG_NAV_ARC_TIP_FWD_DEG,
        true,
        radius,
        arc_len,
        head_len,
        head_width,
        stroke,
        color,
    );
    append_jog_nav_arrow(
        list,
        center,
        JOG_NAV_ARC_TIP_REV_DEG,
        false,
        radius,
        arc_len,
        head_len,
        head_width,
        stroke,
        color,
    );

    list.text(
        ctx,
        layout.sp_in_rect(
            JOG_PANEL_REF,
            JOG_NAV_ARROW_LABEL_LEFT_X,
            JOG_NAV_ARROW_LABEL_V,
        ),
        Align2::CENTER_CENTER,
        "   −\nREV",
        FontId::proportional(layout.sc(JOG_NAV_ARROW_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );

    list.text(
        ctx,
        layout.sp_in_rect(
            JOG_PANEL_REF,
            JOG_NAV_ARROW_LABEL_RIGHT_X,
            JOG_NAV_ARROW_LABEL_V,
        ),
        Align2::CENTER_CENTER,
        "   +\nFWD",
        FontId::proportional(layout.sc(JOG_NAV_ARROW_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );
}

/// Shapes drawn *before* the pitchbend grip (outer jog rings).
pub(super) fn build_jog_outer(out: &mut ShapeList, center: egui::Pos2, layout: &UiScale) {
    let r_outer_1 = layout.sc(JOG_OUTER_1_STROKE_RADIUS);
    let r_outer_2 = layout.sc(JOG_OUTER_2_STROKE_RADIUS);

    out.circle_filled(center, r_outer_2, COL_SILVER);
    out.circle_filled(center, r_outer_1, COL_JOG_BODY);

    let cosmetic_border = DoubleBorderSpec::from_strokes_with_gap(
        StrokeSpec {
            width: layout.sc(JOG_TOUCH_BORDER_INNER_WIDTH_REF),
            color: COL_WHITE,
        },
        StrokeSpec {
            width: layout.sc(JOG_TOUCH_BORDER_OUTER_WIDTH_REF),
            color: COL_SILVER,
        },
        layout.sc(JOG_OUTER_2_STROKE_GAP),
    );
    paint_double_circle_ring_collect(
        out,
        center,
        r_outer_2 - layout.sc(JOG_OUTER_2_STROKE_GAP),
        cosmetic_border,
    );
}

/// Shapes drawn *after* the pitchbend grip: `out_mid` is platter + LCD background;
/// `out_over` is LCD chrome and JOG ADJUST (drawn above the live stream texture).
pub(super) fn build_jog_inner(
    out_mid: &mut ShapeList,
    out_over: &mut ShapeList,
    ctx: &egui::Context,
    center: egui::Pos2,
    layout: &UiScale,
    jog_adjust: f32,
) {
    let r_touch = layout.sc(JOG_TOUCH_RADIUS);

    // Touch platter fill (overwrites grip interior).
    out_mid.circle_filled(center, r_touch, COL_DARK);

    // Double border around touch-platter edge.
    let touch_border = DoubleBorderSpec::from_strokes_with_gap(
        StrokeSpec {
            width: layout.sc(JOG_TOUCH_BORDER_INNER_WIDTH_REF),
            color: COL_BTN_TEXT,
        },
        StrokeSpec {
            width: layout.sc(JOG_TOUCH_BORDER_OUTER_WIDTH_REF),
            color: COL_SILVER,
        },
        layout.sc(JOG_TOUCH_BORDER_GAP_REF),
    );
    paint_double_circle_ring_collect(out_mid, center, r_touch, touch_border);

    // Inner LCD disk (background only; stream texture is painted between mid/over).
    build_jog_inner_lcd_background(out_mid, center, layout);
    build_jog_inner_lcd_foreground(out_over, ctx, center, layout);

    // JOG ADJUST knob (gear + ticks + labels) - keyed on jog_adjust.
    build_jog_adjust_static(out_over, ctx, layout, jog_adjust);
}

/// Static geometry for the JOG ADJUST knob: gear, indicator, tick ring, labels.
/// Keyed on `jog_adjust` (rotates gear + indicator).
pub(super) fn build_jog_adjust_static(
    out: &mut ShapeList,
    ctx: &egui::Context,
    layout: &UiScale,
    jog_adjust: f32,
) {
    let center = layout.sp_in_rect(JOG_PANEL_REF, JOG_ADJUST_CENTER_U, JOG_ADJUST_CENTER_V);
    let s = |r: f32| layout.sc(r * JOG_ADJUST_SIZE_SCALE);
    let r_base = s(JOG_ADJUST_KNOB_RADIUS_REF);
    let tooth_amp = s(JOG_ADJUST_TOOTH_AMP_REF);
    let r_outer = r_base + tooth_amp;

    let indicator_angle =
        JOG_ADJUST_TICK_ARC_START_CW_RAD + jog_adjust * JOG_ADJUST_TICK_ARC_SPAN_RAD;

    draw_rotary_control_collect(
        out,
        center,
        RotaryControlSpec {
            gear: RotaryGearSpec {
                base_radius: r_base,
                tooth_count: JOG_ADJUST_TOOTH_COUNT,
                tooth_amplitude: tooth_amp,
                tooth_top_fraction: JOG_ADJUST_TOOTH_TOP_FRAC,
                valley_segments: JOG_ADJUST_VALLEY_SEGMENTS,
                rotation_rad: indicator_angle - std::f32::consts::FRAC_PI_2,
                fill: JOG_ADJUST_FILL_COLOR,
                stroke: StrokeSpec {
                    width: s(JOG_ADJUST_STROKE_REF),
                    color: COL_BTN_TEXT,
                },
            },
            indicator: RotaryIndicatorSpec {
                length: r_base * JOG_ADJUST_INDICATOR_LENGTH_FRAC,
                width: s(JOG_ADJUST_INDICATOR_WIDTH_REF),
                angle_cw_from_top_rad: indicator_angle,
                stroke: StrokeSpec {
                    width: s(JOG_ADJUST_INDICATOR_STROKE_REF),
                    color: COL_BTN_TEXT,
                },
            },
            ticks: RotaryTickSpec {
                count: JOG_ADJUST_TICK_COUNT,
                arc_start_cw_rad: JOG_ADJUST_TICK_ARC_START_CW_RAD,
                arc_span_rad: JOG_ADJUST_TICK_ARC_SPAN_RAD,
                ring_offset: s(JOG_ADJUST_TICK_RING_OFFSET_REF),
                dot_radius: s(JOG_ADJUST_TICK_DOT_RADIUS_REF),
                center_dot_radius: s(JOG_ADJUST_TICK_CENTER_DOT_RADIUS_REF),
                endcap_length: s(JOG_ADJUST_TICK_ENDCAP_LENGTH_REF),
                endcap_stroke: StrokeSpec {
                    width: s(JOG_ADJUST_TICK_ENDCAP_STROKE_REF),
                    color: COL_BTN_TEXT,
                },
            },
            color: COL_BTN_TEXT,
            draw_indicator: true,
            draw_ticks: true,
        },
    );

    // Labels.
    let top_label_pos = center + Vec2::new(0.0, -(r_outer + s(JOG_ADJUST_TOP_LABEL_GAP_REF)));
    out.text(
        ctx,
        top_label_pos,
        Align2::CENTER_BOTTOM,
        "JOG ADJUST",
        FontId::proportional(s(JOG_ADJUST_TOP_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );

    let side_y = r_outer + s(JOG_ADJUST_SIDE_LABEL_GAP_REF);
    let side_x = s(JOG_ADJUST_SIDE_LABEL_X_OFFSET_REF);
    let side_font = FontId::proportional(s(JOG_ADJUST_SIDE_LABEL_FONT_SIZE));
    out.text(
        ctx,
        center + Vec2::new(-side_x, side_y),
        Align2::CENTER_TOP,
        "LIGHT",
        side_font.clone(),
        COL_BTN_TEXT,
    );
    out.text(
        ctx,
        center + Vec2::new(side_x, side_y),
        Align2::CENTER_TOP,
        "HEAVY",
        side_font,
        COL_BTN_TEXT,
    );
}
