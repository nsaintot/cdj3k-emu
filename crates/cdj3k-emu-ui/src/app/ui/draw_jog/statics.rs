//! The static jog wheel geometry that the jog static cache holds: the outer
//! rings, the inner disk, and the JOG ADJUST gear and indicator.

use egui::{Align2, Color32, FontFamily, FontId, Pos2, Rect, Shape, Stroke, Vec2};

use super::super::draw_cache::ShapeList;
use super::super::{
    draw_rotary_control_collect, paint_double_circle_ring_collect, DoubleBorderSpec,
    RotaryControlSpec, RotaryGearSpec, RotaryIndicatorSpec, RotaryTickSpec, StrokeSpec, UiScale,
    COL_BTN_TEXT, COL_DARK, COL_JOG_BODY, COL_LCD_BG, COL_SILVER, COL_WHITE,
};
use super::*;

// ── FWD and REV nav arrows ───────────────────────────────────────────────────
// Two short curved arrows just outside the jog ring, with their tips at compass
// 144.5° (FWD, clockwise) and 215.5° (REV, counterclockwise). Both lie on one
// circle of radius `JOG_NAV_ARC_RADIUS_REF` around the jog center, and each is
// about `JOG_NAV_ARC_LEN_REF` long.
pub(super) const JOG_NAV_ARC_RADIUS_REF: f32 = 1115.0;
/// Length of the stroked arc body, without the arrowhead.
pub(super) const JOG_NAV_ARC_LEN_REF: f32 = 275.0;
/// Arrowhead length along the arc tangent, from the apex V1 to the base V2 on the arc.
pub(super) const JOG_NAV_ARC_HEAD_LEN_REF: f32 = 50.0;
/// Arrowhead width perpendicular to the arc, outward from the jog, from V2 to V3.
pub(super) const JOG_NAV_ARC_HEAD_WIDTH_REF: f32 = 14.0;
pub(super) const JOG_NAV_ARC_STROKE_REF: f32 = 4.0;

pub(super) const JOG_NAV_ARC_TIP_FWD_DEG: f32 = 144.5;
pub(super) const JOG_NAV_ARC_TIP_REV_DEG: f32 = 215.5;

// Placement of the REV and FWD labels under the arrows.
pub(super) const JOG_NAV_ARROW_LABEL_LEFT_X: f32 = 0.13;
pub(super) const JOG_NAV_ARROW_LABEL_RIGHT_X: f32 = 0.865;
pub(super) const JOG_NAV_ARROW_LABEL_V: f32 = 0.902;
pub(super) const JOG_NAV_ARROW_LABEL_FONT_SIZE: f32 = 34.0;

/// Returns the unit vector for a compass bearing in egui screen space, where Y points
/// down. 0° is north, and the bearing increases clockwise.
#[inline]
fn compass_unit(deg: f32) -> Vec2 {
    let r = deg.to_radians();
    Vec2::new(r.sin(), -r.cos())
}

/// Draws one curved nav arrow: an arc body with an asymmetric arrowhead at the tip.
/// `tip_deg` is the compass bearing of the tip, and `motion_cw` selects the
/// rotation the arrow shows. For clockwise the body extends toward smaller compass
/// angles; for counterclockwise it extends toward larger ones.
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
    // The body extends backward from the tip, against the indicated rotation,
    // and stops at the arrowhead's base so that the apex stays pointed.
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

    // The arrowhead is a right triangle with its apex at the tip and its V1-V2
    // leg on the inner edge of the arc body (radius - stroke/2).
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
    jog_ref: Rect,
    chrome: JogChrome,
) {
    if !chrome.nav_arrows {
        return;
    }
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
        layout.sp_in_rect(jog_ref, JOG_NAV_ARROW_LABEL_LEFT_X, JOG_NAV_ARROW_LABEL_V),
        Align2::CENTER_CENTER,
        "   −\nREV",
        FontId::proportional(layout.sc(JOG_NAV_ARROW_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );

    list.text(
        ctx,
        layout.sp_in_rect(jog_ref, JOG_NAV_ARROW_LABEL_RIGHT_X, JOG_NAV_ARROW_LABEL_V),
        Align2::CENTER_CENTER,
        "   +\nFWD",
        FontId::proportional(layout.sc(JOG_NAV_ARROW_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );
}

/// Builds the shapes drawn before the pitchbend grip: the outer jog rings.
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

/// Builds the shapes drawn after the pitchbend grip. `out_mid` gets the platter and
/// the LCD background; `out_over` gets the LCD chrome and JOG ADJUST, which are drawn
/// above the live stream texture.
pub(super) fn build_jog_inner(
    out_mid: &mut ShapeList,
    out_over: &mut ShapeList,
    ctx: &egui::Context,
    center: egui::Pos2,
    layout: &UiScale,
    jog_adjust: f32,
    jog_ref: Rect,
    chrome: JogChrome,
) {
    let r_touch = layout.sc(JOG_TOUCH_RADIUS);

    // The touch platter fill covers the inside of the grip.
    out_mid.circle_filled(center, r_touch, COL_DARK);

    // A double border around the touch platter's edge.
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

    // The inner LCD disk is only the background; the stream texture is painted
    // between `out_mid` and `out_over`.
    build_jog_inner_lcd_background(out_mid, center, layout);
    build_jog_inner_lcd_foreground(out_over, ctx, center, layout);

    // Build the JOG ADJUST knob with its gear, ticks and labels; its angle
    // follows jog_adjust.
    build_jog_adjust_static(out_over, ctx, layout, jog_adjust, jog_ref, chrome);
}

/// Builds the static geometry for the JOG ADJUST knob: the gear, indicator, tick
/// ring and labels. It is keyed on `jog_adjust`, which rotates the gear and the
/// indicator.
pub(super) fn build_jog_adjust_static(
    out: &mut ShapeList,
    ctx: &egui::Context,
    layout: &UiScale,
    jog_adjust: f32,
    jog_ref: Rect,
    chrome: JogChrome,
) {
    let center = layout.sp_in_rect(jog_ref, JOG_ADJUST_CENTER_U, JOG_ADJUST_CENTER_V);
    let s = |r: f32| layout.sc(r * JOG_ADJUST_SIZE_SCALE);
    let r_base = s(JOG_ADJUST_KNOB_RADIUS_REF);
    let tooth_amp = s(JOG_ADJUST_TOOTH_AMP_REF);
    let r_outer = r_base + tooth_amp;

    let indicator_angle =
        JOG_ADJUST_TICK_ARC_START_CW_RAD + jog_adjust * JOG_ADJUST_TICK_ARC_SPAN_RAD;

    draw_rotary_control_collect(
        out,
        center,
        jog_adjust_spec(&s, r_base, tooth_amp, indicator_angle),
    );
    collect_jog_adjust_labels(out, ctx, center, &s, r_outer, chrome);
}

/// Builds the knob's gear, indicator and tick ring, sized by `s`.
fn jog_adjust_spec(
    s: &impl Fn(f32) -> f32,
    r_base: f32,
    tooth_amp: f32,
    indicator_angle: f32,
) -> RotaryControlSpec {
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
    }
}

/// Builds the title above the knob and the LIGHT and HEAVY labels under it.
fn collect_jog_adjust_labels(
    out: &mut ShapeList,
    ctx: &egui::Context,
    center: Pos2,
    s: &impl Fn(f32) -> f32,
    r_outer: f32,
    chrome: JogChrome,
) {
    let top_label_pos = center + Vec2::new(0.0, -(r_outer + s(JOG_ADJUST_TOP_LABEL_GAP_REF)));
    out.text(
        ctx,
        top_label_pos,
        Align2::CENTER_BOTTOM,
        chrome.adjust_label,
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

/// Builds the inner LCD disk, which is only the background; the stream texture is
/// painted over it.
pub(super) fn build_jog_inner_lcd_background(
    out: &mut ShapeList,
    center: egui::Pos2,
    layout: &UiScale,
) {
    let r_inner_lcd = layout.sc(JOG_INNER_LCD_C1_RADIUS);
    out.circle_filled(center, r_inner_lcd, COL_LCD_BG);
}

/// Builds the inner LCD's chrome over the stream: its c1/c2 silhouette, the c3
/// separator arcs and the MODE and BEAT SYNC labels.
pub(super) fn build_jog_inner_lcd_foreground(
    out: &mut ShapeList,
    ctx: &egui::Context,
    center: egui::Pos2,
    layout: &UiScale,
) {
    let r_inner_lcd = layout.sc(JOG_INNER_LCD_C1_RADIUS);
    let stroke = Stroke::new(layout.sc(7.0), COL_BTN);
    let join_overlap = layout.sc(JOG_INNER_LCD_JOIN_OVERLAP_REF);

    // c1 is the inner LCD cutout circle.
    let c1 = r_inner_lcd;
    // A rectangle centered at O with its corners on c1.
    let mut rect_half_w = c1 * JOG_INNER_LCD_RECT_HALF_WIDTH_FRAC;
    let mut rect_half_h = (c1 * c1 - rect_half_w * rect_half_w).sqrt();
    // Keep the rectangle's long axis horizontal.
    if rect_half_h > rect_half_w {
        std::mem::swap(&mut rect_half_w, &mut rect_half_h);
    }
    // c2 is a smaller concentric circle that forms the side bulges.
    let c2 = c1 * JOG_INNER_LCD_C2_RADIUS_FRAC;

    // Draw the c1 arcs on the left and right of the rectangle.
    collect_clipped_circle_arcs(out, center, c1, stroke, &|x, _y| {
        x.abs() >= rect_half_w - 1.0e-3
    });
    // Draw the c2 arcs outside the rectangle: the left and right arcs and the
    // corner bridge arcs.
    collect_clipped_circle_arcs(out, center, c2, stroke, &|x, y| {
        x.abs() >= rect_half_w - 1.0e-3 || y.abs() >= rect_half_h - 1.0e-3
    });

    // Draw the rectangle's edges between the c1 and c2 intersections.
    let top_y = rect_half_h;
    let bot_y = -rect_half_h;
    let right_x = rect_half_w;
    let left_x = -rect_half_w;

    let c2_x_at_top = (c2 * c2 - top_y * top_y).max(0.0).sqrt();
    let segs = [
        (
            center + Vec2::new(c2_x_at_top - join_overlap, top_y),
            center + Vec2::new(right_x + join_overlap, top_y),
        ),
        (
            center + Vec2::new(left_x - join_overlap, top_y),
            center + Vec2::new(-c2_x_at_top + join_overlap, top_y),
        ),
        (
            center + Vec2::new(c2_x_at_top - join_overlap, bot_y),
            center + Vec2::new(right_x + join_overlap, bot_y),
        ),
        (
            center + Vec2::new(left_x - join_overlap, bot_y),
            center + Vec2::new(-c2_x_at_top + join_overlap, bot_y),
        ),
    ];
    for (a, b) in segs {
        out.line_segment([a, b], stroke);
    }

    collect_inner_lcd_ring(out, ctx, center, layout, c1);
}

/// Strokes the circle of `radius` around `center`, sampled at 256 points,
/// wherever `keep(x, y)` holds; x and y are offsets from the centre. Each
/// contiguous run becomes one line, including a run that wraps past 0/2π.
fn collect_clipped_circle_arcs(
    out: &mut ShapeList,
    center: Pos2,
    radius: f32,
    stroke: Stroke,
    keep: &dyn Fn(f32, f32) -> bool,
) {
    let line_segments = 256usize;
    let mut pts: Vec<Pos2> = Vec::with_capacity(line_segments);
    let mut mask: Vec<bool> = Vec::with_capacity(line_segments);
    for i in 0..line_segments {
        let t = i as f32 / line_segments as f32;
        let a = t * TAU;
        let (sin_a, cos_a) = a.sin_cos();
        let x = radius * cos_a;
        let y = radius * sin_a;
        pts.push(center + Vec2::new(x, y));
        mask.push(keep(x, y));
    }

    // Emit contiguous runs in cyclic order, including a run crossing 0/2π.
    let mut i = 0usize;
    while i < line_segments {
        while i < line_segments && !mask[i] {
            i += 1;
        }
        if i >= line_segments {
            break;
        }
        let start = i;
        while i < line_segments && mask[i] {
            i += 1;
        }
        let end = i; // exclusive

        let mut run: Vec<Pos2> = pts[start..end].to_vec();
        if start == 0 && end < line_segments && mask[line_segments - 1] {
            let mut tail_start = line_segments - 1;
            while tail_start > 0 && mask[tail_start - 1] {
                tail_start -= 1;
            }
            let mut merged: Vec<Pos2> = pts[tail_start..line_segments].to_vec();
            merged.extend(run);
            run = merged;
        }
        if run.len() >= 2 {
            out.add(Shape::line(run, stroke));
        }
    }
}

/// Builds the c3 label ring: the separator arcs between adjacent labels in each
/// half, and the MODE and BEAT SYNC labels. SLIP, VINYL, SYNC and MASTER are
/// painted over the stream on every frame.
fn collect_inner_lcd_ring(
    out: &mut ShapeList,
    ctx: &egui::Context,
    center: Pos2,
    layout: &UiScale,
    c1: f32,
) {
    let c3 = c1 * JOG_INNER_LCD_C3_RADIUS_FRAC;
    let c3_stroke = Stroke::new(layout.sc(JOG_INNER_LCD_C3_ARC_STROKE_REF), COL_SILVER);
    let arc_seg = JOG_INNER_LCD_C3_ARC_SEGMENTS;

    geometry::inner_lcd_add_c3_arc_span(out, center, c3, 325.0, 349.0, c3_stroke, arc_seg);
    geometry::inner_lcd_add_c3_arc_span(out, center, c3, 11.0, 35.0, c3_stroke, arc_seg);
    geometry::inner_lcd_add_c3_arc_span(out, center, c3, 150.0, 162.0, c3_stroke, arc_seg);
    geometry::inner_lcd_add_c3_arc_span(out, center, c3, 200.0, 212.0, c3_stroke, arc_seg);

    let label_font = FontId::new(
        layout.sc(JOG_INNER_LCD_LABEL_FONT_REF),
        FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS_BOLD.into()),
    );
    let cutout_outline_px = layout.sc(JOG_INNER_LCD_CUTOUT_OUTLINE_REF);

    // The top group. SLIP and VINYL are painted over the stream on every frame
    // (see `paint_jog_inner_lcd_corner_labels`).
    geometry::inner_lcd_push_curved_label(
        out,
        ctx,
        center,
        c3,
        0.0,
        "MODE",
        label_font.clone(),
        COL_SILVER,
        false,
        cutout_outline_px,
        false,
    );

    // The bottom group. SYNC and MASTER are painted over the stream on every
    // frame.
    geometry::inner_lcd_push_curved_label(
        out,
        ctx,
        center,
        c3,
        180.0,
        "BEAT SYNC",
        label_font,
        COL_SILVER,
        false,
        cutout_outline_px,
        JOG_INNER_LCD_INVERT_BOTTOM_CORNER_LABELS,
    );
}
