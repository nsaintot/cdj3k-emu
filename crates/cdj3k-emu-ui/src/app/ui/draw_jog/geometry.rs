//! Polygon clipping, ring and arc samplers, and the inner LCD silhouette
//! polygon. The jog renderer uses them to clip the LCD texture mesh and to lay
//! out arc spans.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use egui::{emath::Rot2, epaint::TextShape, FontId, Pos2, Rect, Vec2};

use super::super::UiScale;
use super::*;

fn line_x_intersection(a: Pos2, b: Pos2, x: f32) -> Option<Pos2> {
    let denom = b.x - a.x;
    if denom.abs() < 1e-9 {
        return None;
    }
    let t = (x - a.x) / denom;
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    Some(a.lerp(b, t))
}

fn line_y_intersection(a: Pos2, b: Pos2, y: f32) -> Option<Pos2> {
    let denom = b.y - a.y;
    if denom.abs() < 1e-9 {
        return None;
    }
    let t = (y - a.y) / denom;
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    Some(a.lerp(b, t))
}

fn clip_polygon_half_plane(
    poly: &[Pos2],
    inside: impl Fn(Pos2) -> bool,
    intersect: impl Fn(Pos2, Pos2) -> Option<Pos2>,
) -> Vec<Pos2> {
    if poly.is_empty() {
        return Vec::new();
    }
    let n = poly.len();
    let mut out = Vec::with_capacity(poly.len() + 4);
    let mut s = poly[n - 1];
    let mut s_in = inside(s);
    for &e in poly {
        let e_in = inside(e);
        if e_in {
            if !s_in {
                if let Some(i) = intersect(s, e) {
                    out.push(i);
                }
            }
            out.push(e);
        } else if s_in {
            if let Some(i) = intersect(s, e) {
                out.push(i);
            }
        }
        s = e;
        s_in = e_in;
    }
    out
}

/// Clips `poly` to the axis-aligned `rect` with the Sutherland–Hodgman algorithm. It drops the
/// silhouette caps outside the jog stream's rect, so that the mesh UVs stay in [0, 1].
pub(super) fn clip_polygon_to_rect(poly: &[Pos2], rect: Rect) -> Vec<Pos2> {
    let xmin = rect.left();
    let xmax = rect.right();
    let ymin = rect.top();
    let ymax = rect.bottom();

    let mut cur = poly.to_vec();
    cur = clip_polygon_half_plane(
        &cur,
        |p| p.x >= xmin,
        |a, b| line_x_intersection(a, b, xmin),
    );
    cur = clip_polygon_half_plane(
        &cur,
        |p| p.x <= xmax,
        |a, b| line_x_intersection(a, b, xmax),
    );
    cur = clip_polygon_half_plane(
        &cur,
        |p| p.y >= ymin,
        |a, b| line_y_intersection(a, b, ymin),
    );
    cur = clip_polygon_half_plane(
        &cur,
        |p| p.y <= ymax,
        |a, b| line_y_intersection(a, b, ymax),
    );

    if cur.len() >= 2 {
        let a = cur[0];
        let z = *cur.last().unwrap();
        if (a - z).length_sq() < 1.0e-6 {
            cur.pop();
        }
    }

    let mut deduped: Vec<Pos2> = Vec::with_capacity(cur.len());
    for q in cur {
        if let Some(p) = deduped.last() {
            if p.distance_sq(q) < 1.0e-6 {
                continue;
            }
        }
        deduped.push(q);
    }
    deduped
}

/// Returns the closed polygon that matches the inner LCD stroke in
/// [`build_jog_inner_lcd_foreground`](super::statics::build_jog_inner_lcd_foreground), for
/// texturing.
///
/// The middle of the top and bottom edges follows the outer c2 circle, through `(0,-c2)` and
/// `(0,c2)`.
pub(super) fn inner_lcd_silhouette_polygon(center: Pos2, layout: &UiScale) -> Vec<Pos2> {
    let c1 = layout.sc(JOG_INNER_LCD_C1_RADIUS);
    let mut w = c1 * JOG_INNER_LCD_RECT_HALF_WIDTH_FRAC;
    let mut h = (c1 * c1 - w * w).sqrt();
    if h > w {
        std::mem::swap(&mut w, &mut h);
    }
    let c2 = c1 * JOG_INNER_LCD_C2_RADIUS_FRAC;
    let c2x = (c2 * c2 - h * h).max(0.0).sqrt();

    let p = |dx: f32, dy: f32| center + Vec2::new(dx, dy);
    let mut out = Vec::new();
    let seg = JOG_INNER_LCD_MESH_ARC_SEGMENTS;

    // 1. The c2 left bulge, from (-c2x,-h) to (-c2x,h).
    push_c2_left_arc(&mut out, center, c2, c2x, h, seg);
    // 2. The bottom-left flat.
    push_line_samples(&mut out, p(-c2x, h), p(-w, h), 3);
    // 3. The c1 left cap, from (-w,h) to (-w,-h).
    push_c1_left_cap(&mut out, center, c1, w, h, seg);
    // 4. The top-left flat, if there is one.
    if w > c2x + 1.0e-3 {
        push_line_samples(&mut out, p(-w, -h), p(-c2x, -h), 3);
    }
    // 5. The c2 top outer arc, from (-c2x,-h) to (c2x,-h) through (0,-c2).
    push_c2_top_outer_arc(&mut out, center, c2, c2x, h, seg);
    // 6. The top-right flat, if there is one.
    if w > c2x + 1.0e-3 {
        push_line_samples(&mut out, p(c2x, -h), p(w, -h), 3);
    }
    // 7. The c1 right cap, from (w,-h) to (w,h).
    push_circle_arc_short(
        &mut out,
        center,
        c1,
        f32::atan2(-h, w),
        f32::atan2(h, w),
        seg,
    );
    // 8. The bottom-right flat.
    push_line_samples(&mut out, p(w, h), p(c2x, h), 3);
    // 9. The c2 bottom outer arc, from (c2x,h) to (-c2x,h) through (0,c2).
    push_c2_bottom_outer_arc(&mut out, center, c2, c2x, h, seg);

    if out.len() >= 2 {
        let a = out[0];
        let z = *out.last().unwrap();
        if (a - z).length_sq() < 1.0e-5 {
            out.pop();
        }
    }

    out
}

fn push_pt_ring(out: &mut Vec<Pos2>, q: Pos2) {
    if let Some(p) = out.last() {
        if (*p - q).length_sq() < 1.0e-6 {
            return;
        }
    }
    out.push(q);
}

fn push_line_samples(out: &mut Vec<Pos2>, a: Pos2, b: Pos2, n: usize) {
    let n = n.max(2);
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32;
        push_pt_ring(out, a.lerp(b, t));
    }
}

/// Pushes the shorter circle arc from `a0` to `a1`, in radians; x = r cos a, y = r sin a.
fn push_circle_arc_short(out: &mut Vec<Pos2>, center: Pos2, r: f32, a0: f32, a1: f32, seg: usize) {
    let mut d = a1 - a0;
    while d > TAU {
        d -= TAU;
    }
    while d < -TAU {
        d += TAU;
    }
    if d > PI {
        d -= TAU;
    } else if d < -PI {
        d += TAU;
    }
    let seg = seg.max(4);
    for i in 0..=seg {
        let t = i as f32 / seg as f32;
        let a = a0 + d * t;
        push_pt_ring(out, center + Vec2::new(r * a.cos(), r * a.sin()));
    }
}

/// Pushes the c1 left cap, from (-w, h) to (-w, -h), along the outer c1 circle where x ≤ -w.
fn push_c1_left_cap(out: &mut Vec<Pos2>, center: Pos2, r: f32, w: f32, h: f32, seg: usize) {
    let a0 = f32::atan2(h, -w);
    let mut a1 = f32::atan2(-h, -w);
    if a1 < a0 {
        a1 += TAU;
    }
    let span = a1 - a0;
    let seg = seg.max(4);
    for i in 0..=seg {
        let t = i as f32 / seg as f32;
        let mut a = a0 + span * t;
        if a > TAU {
            a -= TAU;
        }
        push_pt_ring(out, center + Vec2::new(r * a.cos(), r * a.sin()));
    }
}

/// Pushes the c2 left bulge from (-c2x, -h) to (-c2x, h) through (-c2, 0); x = r cos a,
/// y = r sin a.
fn push_c2_left_arc(out: &mut Vec<Pos2>, center: Pos2, r: f32, c2x: f32, h: f32, seg: usize) {
    let seg = seg.max(8);
    let c2x_e = c2x.max(1.0e-4);
    if h <= r - 1.0e-3 {
        let alpha = f32::atan2(h, c2x_e);
        let a_top = PI + alpha;
        let a_bot = PI - alpha;
        for i in 0..=seg {
            let t = i as f32 / seg as f32;
            let a = a_top + (a_bot - a_top) * t;
            push_pt_ring(out, center + Vec2::new(r * a.cos(), r * a.sin()));
        }
    } else {
        // When h > c2, trace the outer left semicircle from the top (3π/2) through π to the
        // bottom (π/2).
        let a_top = 3.0 * FRAC_PI_2;
        let a_bot = FRAC_PI_2;
        for i in 0..=seg {
            let t = i as f32 / seg as f32;
            let a = a_top + (a_bot - a_top) * t;
            push_pt_ring(out, center + Vec2::new(r * a.cos(), r * a.sin()));
        }
    }
}

/// Pushes the outer c2 arc along the top, from `(-c2x,-h)` to `(c2x,-h)` through `(0,-c2)`.
fn push_c2_top_outer_arc(out: &mut Vec<Pos2>, center: Pos2, c2: f32, c2x: f32, h: f32, seg: usize) {
    let cx = c2x.max(1.0e-5);
    let a_l = f32::atan2(-h, -cx);
    let a_r = f32::atan2(-h, cx);
    let span = a_r - a_l;
    if span.abs() < 0.02 {
        // When the chord is degenerate, trace the upper half of c2 through angle -π/2.
        push_arc_monotone(out, center, c2, PI, TAU, seg.max(24));
    } else {
        push_circle_arc_short(out, center, c2, a_l, a_r, seg);
    }
}

/// Pushes the outer c2 arc along the bottom, from `(c2x,h)` to `(-c2x,h)` through `(0,c2)`.
fn push_c2_bottom_outer_arc(
    out: &mut Vec<Pos2>,
    center: Pos2,
    c2: f32,
    c2x: f32,
    h: f32,
    seg: usize,
) {
    let cx = c2x.max(1.0e-5);
    let a_r = f32::atan2(h, cx);
    let a_l = f32::atan2(h, -cx);
    let span = a_l - a_r;
    if span.abs() < 0.02 {
        push_arc_monotone(out, center, c2, 0.0, PI, seg.max(24));
    } else {
        push_circle_arc_short(out, center, c2, a_r, a_l, seg);
    }
}

pub(super) fn push_arc_monotone(
    out: &mut Vec<Pos2>,
    center: Pos2,
    r: f32,
    a0: f32,
    a1: f32,
    seg: usize,
) {
    let seg = seg.max(8);
    for i in 0..=seg {
        let t = i as f32 / seg as f32;
        let a = a0 + (a1 - a0) * t;
        push_pt_ring(out, center + Vec2::new(r * a.cos(), r * a.sin()));
    }
}

pub(super) fn inner_lcd_point_on_ring(
    center: Pos2,
    radius: f32,
    angle_cw_from_top_rad: f32,
) -> Pos2 {
    let (sin_a, cos_a) = angle_cw_from_top_rad.sin_cos();
    center + Vec2::new(radius * sin_a, -radius * cos_a)
}

pub(super) fn inner_lcd_add_c3_arc_span(
    out: &mut ShapeList,
    center: Pos2,
    radius: f32,
    angle_start_deg: f32,
    angle_end_deg: f32,
    stroke: Stroke,
    segments: usize,
) {
    if segments < 2 {
        return;
    }
    let mut pts = Vec::with_capacity(segments + 1);
    for i in 0..=segments {
        let t = i as f32 / segments as f32;
        let a = angle_start_deg.to_radians()
            + (angle_end_deg.to_radians() - angle_start_deg.to_radians()) * t;
        pts.push(inner_lcd_point_on_ring(center, radius, a));
    }
    out.add(Shape::line(pts, stroke));
}

/// Draws a curved label on the ring of `radius`; `angle` is the label center in degrees
/// clockwise from 12 o'clock.
///
/// With `invert_glyph_orientation`, used on the bottom arc, the label is flipped along both the
/// arc tangent and the normal so that it reads upright: the tangential spread is negated and π is
/// added to the rotation, which is a 180° turn in the tangent-normal frame.
pub(super) fn inner_lcd_push_curved_label(
    out: &mut ShapeList,
    ctx: &egui::Context,
    center: Pos2,
    radius: f32,
    angle: f32,
    text: &str,
    font: FontId,
    label_color: Color32,
    cutout_stub: bool,
    cutout_outline_px: f32,
    invert_glyph_orientation: bool,
) {
    let galley =
        ctx.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font.clone(), Color32::PLACEHOLDER));
    if galley.is_empty() {
        return;
    }
    let ppp = ctx.pixels_per_point();
    let snap_to_pixel =
        |p: Pos2| -> Pos2 { Pos2::new((p.x * ppp).round() / ppp, (p.y * ppp).round() / ppp) };

    let mut total_width = 0.0_f32;
    for row in &galley.rows {
        for g in &row.glyphs {
            total_width += g.advance_width;
        }
    }
    if total_width < 1.0e-3 {
        return;
    }

    let max_span = 0.9 * FRAC_PI_2;
    let arc_span = (total_width / radius).min(max_span);

    // Every glyph uses the same Y anchor: the center of the row's line box in
    // the full-word galley. A per-character layout fits its rect to the glyph
    // ink, so anchoring on it would shift letters radially.
    let row_center_y = galley
        .rows
        .first()
        .map(|r| r.rect().center().y)
        .unwrap_or(galley.rect.center().y);

    let mut cum = 0.0_f32;
    for glyph in galley.rows.iter().flat_map(|row| &row.glyphs) {
        let adv = glyph.advance_width;
        if adv < 1.0e-4 {
            continue;
        }
        let mid_s = cum + adv * 0.5;
        cum += adv;
        let spread = (mid_s / total_width - 0.5) * arc_span;
        let theta = angle.to_radians()
            + if invert_glyph_orientation {
                -spread
            } else {
                spread
            };
        let orient = theta + if invert_glyph_orientation { PI } else { 0.0 };

        let chr = glyph.chr.to_string();
        if chr.chars().all(|c| c.is_whitespace()) {
            continue;
        }

        let g_galley = ctx.fonts_mut(|f| f.layout_no_wrap(chr, font.clone(), Color32::PLACEHOLDER));
        if g_galley.is_empty() {
            continue;
        }
        let anchor = Pos2::new(g_galley.mesh_bounds.center().x, row_center_y);
        let rot = Rot2::from_angle(orient);
        let base_on_circle = inner_lcd_point_on_ring(center, radius, theta);
        let galley_pos = snap_to_pixel(base_on_circle - rot * anchor.to_vec2());
        let stub = cutout_stub.then_some(cutout_outline_px);
        push_turned_glyph(out, galley_pos, g_galley, orient, label_color, stub);
    }
}

/// Draws one glyph at `pos`, rotated to `orient`. With a `stub` width, four
/// copies in the button colour, offset by that width, are drawn first to form a
/// dark rim behind LCD-coloured glyphs.
fn push_turned_glyph(
    out: &mut ShapeList,
    pos: Pos2,
    galley: std::sync::Arc<egui::Galley>,
    orient: f32,
    color: Color32,
    stub: Option<f32>,
) {
    if let Some(w) = stub {
        for d in [(1.0_f32, 0.0_f32), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            let off = Vec2::new(d.0 * w, d.1 * w);
            let ts = TextShape::new(pos + off, galley.clone(), COL_BTN)
                .with_override_text_color(COL_BTN)
                .with_angle(orient);
            out.add(Shape::Text(ts));
        }
    }
    let ts = TextShape::new(pos, galley, color)
        .with_override_text_color(color)
        .with_angle(orient);
    out.add(Shape::Text(ts));
}
