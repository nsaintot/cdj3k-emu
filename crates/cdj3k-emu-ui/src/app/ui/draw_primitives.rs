use std::f32::consts::TAU;

use egui::epaint::{PathShape, PathStroke};
use egui::{Color32, Pos2, Rect, Shape, Stroke, Vec2};

use crate::app::ui::{draw_cache::ShapeList, COL_BTN};

mod buttons;
pub(in crate::app) use buttons::*;

/// Width and color for stroked outlines (rect borders, ring strokes, etc.).
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct StrokeSpec {
    pub width: f32,
    pub color: Color32,
}

impl StrokeSpec {
    #[inline]
    pub(in crate::app) fn stroke(self) -> Stroke {
        Stroke::new(self.width, self.color)
    }
}

/// Default clear distance between inner and outer strokes (screen px).
pub(in crate::app) const DEFAULT_DOUBLE_BORDER_GAP: f32 = 2.0;

/// Spacing shared by chrome controls; use [`Default`] for [`DEFAULT_DOUBLE_BORDER_GAP`].
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct DoubleBorderSpacing {
    pub gap: f32,
}

impl Default for DoubleBorderSpacing {
    fn default() -> Self {
        Self {
            gap: DEFAULT_DOUBLE_BORDER_GAP,
        }
    }
}

impl DoubleBorderSpacing {
    #[inline]
    pub(in crate::app) fn apply(self, inner: StrokeSpec, outer: StrokeSpec) -> DoubleBorderSpec {
        DoubleBorderSpec {
            inner,
            gap: self.gap,
            outer,
        }
    }
}

/// Two concentric strokes: [`inner`](Self::inner) on the shape edge, then a clear [`gap`](Self::gap),
/// then [`outer`](Self::outer) farther out.
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct DoubleBorderSpec {
    pub inner: StrokeSpec,
    pub gap: f32,
    pub outer: StrokeSpec,
}

impl DoubleBorderSpec {
    /// Inner + outer strokes with [`DEFAULT_DOUBLE_BORDER_GAP`].
    #[inline]
    pub(in crate::app) fn from_strokes(inner: StrokeSpec, outer: StrokeSpec) -> Self {
        DoubleBorderSpacing::default().apply(inner, outer)
    }

    /// Inner + outer strokes with a specified `gap` value.
    #[inline]
    pub(in crate::app) fn from_strokes_with_gap(
        inner: StrokeSpec,
        outer: StrokeSpec,
        gap: f32,
    ) -> Self {
        DoubleBorderSpacing { gap }.apply(inner, outer)
    }
}

pub(in crate::app) fn paint_double_circle_ring(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    spec: DoubleBorderSpec,
) {
    let r_outer = radius + spec.inner.width * 0.5 + spec.gap.max(0.0) + spec.outer.width * 0.5;

    painter.circle_stroke(center, r_outer, spec.outer.stroke());
    painter.circle_stroke(center, radius, spec.inner.stroke());
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::app) enum ButtonType {
    Basic,
    HotCue,
    /// Like Basic, but the colored fill is drawn in an *inset* rect that
    /// envelops the label only - the button's outer rect (and double
    /// border) keep their original size.  `pad_h` / `pad_v` are absolute
    /// pixel paddings around the label glyph rect.
    InsetFill {
        pad_h: f32,
        pad_v: f32,
    },
}

fn paint_double_rect_border(
    painter: &egui::Painter,
    rect: Rect,
    rounding: Option<f32>,
    spec: DoubleBorderSpec,
) {
    let rounding = rounding.unwrap_or(0.0);
    let expand = spec.inner.width * 0.5 + spec.gap.max(0.0) + spec.outer.width * 0.5;
    let rect_outer = rect.expand(expand);
    let max_r = 0.5 * rect_outer.width().min(rect_outer.height());
    let rounding_outer = (rounding + expand).min(max_r);

    if rounding == 0.0 {
        painter.rect_stroke(rect_outer, 0.0, spec.outer.stroke());
        painter.rect_stroke(rect, 0.0, spec.inner.stroke());
    } else {
        painter.rect_stroke(rect_outer, rounding_outer, spec.outer.stroke());
        painter.rect_stroke(rect, rounding, spec.inner.stroke());
    }
}

/// Draws a rounded-rect section with optional fill and a double border.
/// Works in **screen space** - the caller must already have converted `rect` via `layout.sp` / `layout.sr`.
pub(in crate::app) fn draw_bordered_rect_section(
    painter: &egui::Painter,
    rect: Rect,
    rounding: Option<f32>,
    fill: Option<Color32>,
    border: DoubleBorderSpec,
) {
    if let Some(fill_color) = fill {
        painter.rect_filled(rect, rounding.unwrap_or(0.0), fill_color);
    }

    paint_double_rect_border(painter, rect, rounding, border);
}

/// ShapeList version of [`draw_bordered_rect_section`] for static caching.
pub(in crate::app) fn collect_bordered_rect_section(
    out: &mut ShapeList,
    rect: Rect,
    rounding: Option<f32>,
    fill: Option<Color32>,
    border: DoubleBorderSpec,
) {
    if let Some(fill_color) = fill {
        out.rect_filled(rect, rounding.unwrap_or(0.0), fill_color);
    }
    collect_double_rect_border(out, rect, rounding, border);
}

// ---------------------------------------------------------------------------
// Rotary control (gear-style knob + outlined indicator + tick ring)
// ---------------------------------------------------------------------------

/// Gear-style knob outline: flat-topped teeth connected by true **circular arc**
/// valleys. Each valley is the unique circle passing through the two adjacent
/// tooth-top corners (at `r_max`) and the valley midpoint (at `r_min`), giving
/// a constant curvature along the entire valley - no straight walls, no sharp
/// midpoint. The arc is approximated by [`valley_segments`](Self::valley_segments)
/// straight chords of equal angular span about the arc's own center.
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct RotaryGearSpec {
    /// Valley-bottom radius, screen px.
    pub base_radius: f32,
    /// Number of teeth around the gear.
    pub tooth_count: usize,
    /// Radial distance from `base_radius` to the flat tooth top, screen px.
    pub tooth_amplitude: f32,
    /// Fraction of each slot (`2π / tooth_count`) occupied by the flat tooth top;
    /// the remainder is the cosine-dip valley curve between teeth.
    pub tooth_top_fraction: f32,
    /// Number of straight chords used to approximate each circular valley arc.
    /// `2` = single point at the valley bottom → linear V; `8+` = smooth arc
    /// with constant curvature. Even values place a sample exactly on the
    /// valley bottom.
    pub valley_segments: usize,
    /// Rotation offset applied to the entire gear, in radians (standard math
    /// convention, counter-clockwise positive in screen space where y is down).
    /// Use `indicator_angle_cw_from_top - FRAC_PI_2` to keep the gear phase
    /// locked to the indicator. `0.0` = no rotation (tooth 0 starts at 3 o'clock).
    pub rotation_rad: f32,
    /// Fill color for the gear body (area enclosed by the outline). Use
    /// [`Color32::TRANSPARENT`] for no fill.
    pub fill: Color32,
    /// Outline stroke.
    pub stroke: StrokeSpec,
}

/// Outlined rectangular indicator whose long axis points outward from `center`
/// at [`angle_cw_from_top_rad`](Self::angle_cw_from_top_rad) (measured clockwise
/// from the screen's +y-up axis, matching the tick-ring convention).
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct RotaryIndicatorSpec {
    /// Length of the indicator (distance from the knob center to the outer tip),
    /// screen px.
    pub length: f32,
    /// Width of the indicator perpendicular to its long axis, screen px.
    pub width: f32,
    /// Pointing direction, in radians clockwise from straight up. `0.0` points
    /// up, `π/2` points right, and so on.
    pub angle_cw_from_top_rad: f32,
    /// Outline stroke.
    pub stroke: StrokeSpec,
}

/// Radial multiplier applied to the endcap ring relative to the dot ring, so
/// the min/max lines sit slightly outside the dots.
const ENDCAP_RADIAL_FACTOR: f32 = 1.2;

/// Tick ring distributed along an arc. The first and last ticks render as short
/// radial **lines** (min/max endcaps); all other ticks render as filled **dots**,
/// with the single middle tick drawn slightly larger as a center marker.
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct RotaryTickSpec {
    /// Total number of ticks (including the two endcap lines).
    pub count: usize,
    /// Angle of the first tick, in radians measured **clockwise from the top**.
    pub arc_start_cw_rad: f32,
    /// Clockwise angular span from the first to the last tick, in radians.
    pub arc_span_rad: f32,
    /// Radial distance from the gear's outer edge to the tick ring, screen px.
    pub ring_offset: f32,
    /// Radius of the regular dots, screen px.
    pub dot_radius: f32,
    /// Radius of the center dot (slightly larger than [`dot_radius`](Self::dot_radius)).
    pub center_dot_radius: f32,
    /// Radial length of the min/max endcap lines, screen px.
    pub endcap_length: f32,
    /// Stroke for the min/max endcap lines.
    pub endcap_stroke: StrokeSpec,
}

/// Full rotary control spec: gear + indicator + tick ring, painted in [`color`](Self::color).
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct RotaryControlSpec {
    pub gear: RotaryGearSpec,
    pub indicator: RotaryIndicatorSpec,
    pub ticks: RotaryTickSpec,
    /// Base color for tick dots (strokes use their own [`StrokeSpec::color`] fields).
    pub color: Color32,
    /// Whether to draw the indicator pill. Set to `false` for bare-gear rotaries (e.g. nav).
    pub draw_indicator: bool,
    /// Whether to draw the tick ring. Set to `false` for bare-gear rotaries.
    pub draw_ticks: bool,
}

/// Paints a rotary control assembly - gear knob, outlined indicator pill, and tick
/// ring - centered on `center`.
pub(in crate::app) fn draw_rotary_control(
    painter: &egui::Painter,
    center: Pos2,
    spec: RotaryControlSpec,
) {
    paint_gear_outline(painter, center, &spec.gear);
    if spec.draw_ticks {
        paint_rotary_tick_ring(
            painter,
            center,
            spec.gear.base_radius + spec.gear.tooth_amplitude,
            &spec.ticks,
            spec.color,
        );
    }
    if spec.draw_indicator {
        paint_rotary_indicator(painter, center, &spec.indicator);
    }
}

fn paint_gear_outline(painter: &egui::Painter, center: Pos2, gear: &RotaryGearSpec) {
    if gear.tooth_count == 0 {
        return;
    }
    let r_min = gear.base_radius;
    let r_max = gear.base_radius + gear.tooth_amplitude;
    let slot = TAU / gear.tooth_count as f32;
    let tooth_half = slot * gear.tooth_top_fraction.clamp(0.0, 1.0) * 0.5;
    let valley_segments = gear.valley_segments.max(1);

    // Every valley is congruent - same r_min, r_max, angular span. Resolve the
    // circular-arc geometry once, then resolve the interior sample offsets in a
    // valley-local frame (+x along the valley's midpoint radial, origin at the
    // gear center). At render time each tooth just rotates these local offsets
    // by its own midpoint angle: no per-tooth `sin_cos` / `acos` in the inner
    // loop, only 2-D rotations.
    let valley_half_span = 0.5 * (slot - 2.0 * tooth_half);
    let cos_half = valley_half_span.cos();
    let denom = r_max * cos_half - r_min;
    let use_arc = denom.abs() > 1.0e-4;
    let (x_c, arc_r, half_delta) = if use_arc {
        let x_c = (r_max * r_max - r_min * r_min) / (2.0 * denom);
        let arc_r = (r_min - x_c).abs();
        let half_delta = ((x_c - r_max * cos_half) / arc_r).clamp(-1.0, 1.0).acos();
        (x_c, arc_r, half_delta)
    } else {
        (0.0, 0.0, 0.0)
    };

    let interior = valley_segments.saturating_sub(1);
    let mut valley_local: Vec<(f32, f32)> = Vec::with_capacity(interior);
    for k in 1..valley_segments {
        let t = k as f32 / valley_segments as f32;
        if use_arc {
            // Sweep the arc's own center angle uniformly from π+δ (endpoint A)
            // through π (valley bottom) to π−δ (endpoint B).
            let u = 2.0 * t - 1.0;
            let phi = std::f32::consts::PI - half_delta * u;
            let (sin_phi, cos_phi) = phi.sin_cos();
            valley_local.push((x_c + arc_r * cos_phi, arc_r * sin_phi));
        } else {
            // Degenerate arc → linear V with r_min at midpoint, r_max at endpoints.
            let r = r_min + (r_max - r_min) * (2.0 * t - 1.0).abs();
            let local_angle = (t - 0.5) * 2.0 * valley_half_span;
            let (sin_a, cos_a) = local_angle.sin_cos();
            valley_local.push((r * cos_a, r * sin_a));
        }
    }

    let mut pts: Vec<Pos2> = Vec::with_capacity(gear.tooth_count * (2 + interior));
    for i in 0..gear.tooth_count {
        let tooth_center = (i as f32 + 0.5) * slot + gear.rotation_rad;
        let a_left = tooth_center - tooth_half;
        let a_right = tooth_center + tooth_half;

        // Flat tooth top at r_max.
        let (sin_l, cos_l) = a_left.sin_cos();
        pts.push(center + Vec2::new(cos_l, sin_l) * r_max);
        let (sin_r, cos_r) = a_right.sin_cos();
        pts.push(center + Vec2::new(cos_r, sin_r) * r_max);

        // Rotate precomputed valley samples from valley-local (+x along mid) into world.
        let mid_angle = a_right + valley_half_span;
        let (sin_mid, cos_mid) = mid_angle.sin_cos();
        for &(rx, ry) in &valley_local {
            pts.push(center + Vec2::new(rx * cos_mid - ry * sin_mid, rx * sin_mid + ry * cos_mid));
        }
    }
    // The gear polygon is star-shaped about the center, so egui's tessellator
    // handles the fill correctly despite the concave valleys.
    painter.add(Shape::Path(PathShape {
        points: pts,
        closed: true,
        fill: gear.fill,
        stroke: gear.stroke.stroke().into(),
    }));
}

fn paint_rotary_indicator(painter: &egui::Painter, center: Pos2, indicator: &RotaryIndicatorSpec) {
    // Long-axis unit vector (pointing outward) and perpendicular (for width).
    // Clockwise-from-top maps to screen-space (+x right, +y down) via
    // (sin(θ), -cos(θ)); the perpendicular is its 90°-cw rotation.
    let (sin_a, cos_a) = indicator.angle_cw_from_top_rad.sin_cos();
    let dir = Vec2::new(sin_a, -cos_a);
    let perp = Vec2::new(cos_a, sin_a);
    let half_w = indicator.width * 0.5;

    // Four corners of the rotated rectangle; outer tip is flat. The inner end
    // sits at `center`, which is hidden under the gear body - no rounding needed.
    let inner_l = center + perp * -half_w;
    let inner_r = center + perp * half_w;
    let outer_r = inner_r + dir * indicator.length;
    let outer_l = inner_l + dir * indicator.length;
    painter.add(Shape::closed_line(
        vec![inner_l, inner_r, outer_r, outer_l],
        indicator.stroke.stroke(),
    ));
}

fn paint_rotary_tick_ring(
    painter: &egui::Painter,
    center: Pos2,
    gear_outer_radius: f32,
    ticks: &RotaryTickSpec,
    color: Color32,
) {
    if ticks.count < 2 {
        return;
    }
    let ring_r = gear_outer_radius + ticks.ring_offset;
    let last = ticks.count - 1;
    let middle = last / 2;
    for i in 0..ticks.count {
        let cw = ticks.arc_start_cw_rad + (i as f32 / last as f32) * ticks.arc_span_rad;
        let (sin_cw, cos_cw) = cw.sin_cos();
        if i == 0 || i == last {
            let dir = Vec2::new(sin_cw, -cos_cw);
            let outer_r = ring_r * ENDCAP_RADIAL_FACTOR;
            let tick_pos = center + dir * outer_r;
            let inward = center + dir * (outer_r - ticks.endcap_length);
            painter.line_segment([tick_pos, inward], ticks.endcap_stroke.stroke());
        } else {
            // Convert "clockwise from top" to screen-coord unit vector.
            let dir = Vec2::new(sin_cw, -cos_cw);
            let radius = if i == middle {
                ticks.center_dot_radius
            } else {
                ticks.dot_radius
            };
            painter.circle_filled(center + dir * ring_r, radius, color);
        }
    }
}

/// Decorative arc drawn inside an arc button (visible border between inner and outer edges).
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct ArcDecorSpec {
    /// Radius of the arc from the button's center point, in screen px.
    pub radius: f32,
    pub stroke: Stroke,
}

/// Notch arc drawn between the bezel boundary and the decorative arc.
/// Shorter angular span, thicker stroke.
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct ArcNotchSpec {
    /// Radius from the button's center point, in screen px.
    pub radius: f32,
    /// Half-angle: the notch spans `[mid - half_angle, mid + half_angle]`.
    pub half_angle: f32,
    pub stroke: Stroke,
}

// Number of line segments used to approximate the inner concave arc.
const ARC_SEGMENTS: usize = 20;

// ── ShapeList-collecting variants ─────────────────────────────────────────────
// These mirror the painter-based functions above but write into a [`ShapeList`]
// so the geometry can be cached and replayed without recomputation.

/// ShapeList variant of [`draw_back_double_circle_border`] for static caching.
pub(in crate::app) fn collect_back_double_circle_border(
    out: &mut ShapeList,
    center_left: Pos2,
    center_right: Pos2,
    radius: f32,
    thickness: f32,
    color: Color32,
    border_color: Color32,
) {
    let min_x = center_left.x.min(center_right.x) - radius;
    let max_x = center_left.x.max(center_right.x) + radius;
    let min_y = center_left.y.min(center_right.y) - radius;
    let max_y = center_left.y.max(center_right.y) + radius;
    let capsule = Rect::from_min_max(Pos2::new(min_x, min_y), Pos2::new(max_x, max_y));
    out.rect_filled(capsule, radius, color);
    out.rect_stroke(capsule, radius, Stroke::new(thickness, border_color));
}

pub(in crate::app) fn paint_double_circle_ring_collect(
    out: &mut ShapeList,
    center: Pos2,
    radius: f32,
    spec: DoubleBorderSpec,
) {
    let r_outer = radius + spec.inner.width * 0.5 + spec.gap.max(0.0) + spec.outer.width * 0.5;
    out.circle_stroke(center, r_outer, spec.outer.stroke());
    out.circle_stroke(center, radius, spec.inner.stroke());
}

/// Collect a full rotary control (gear + optional indicator + optional tick ring)
/// into a [`ShapeList`] for static caching.
pub(in crate::app) fn draw_rotary_control_collect(
    out: &mut ShapeList,
    center: Pos2,
    spec: RotaryControlSpec,
) {
    collect_gear_outline(out, center, &spec.gear);
    if spec.draw_ticks {
        collect_rotary_tick_ring(
            out,
            center,
            spec.gear.base_radius + spec.gear.tooth_amplitude,
            &spec.ticks,
            spec.color,
        );
    }
    if spec.draw_indicator {
        collect_rotary_indicator(out, center, &spec.indicator);
    }
}

fn collect_gear_outline(out: &mut ShapeList, center: Pos2, gear: &RotaryGearSpec) {
    if gear.tooth_count == 0 {
        return;
    }
    let r_min = gear.base_radius;
    let r_max = gear.base_radius + gear.tooth_amplitude;
    let slot = TAU / gear.tooth_count as f32;
    let tooth_half = slot * gear.tooth_top_fraction.clamp(0.0, 1.0) * 0.5;
    let valley_segments = gear.valley_segments.max(1);

    let valley_half_span = 0.5 * (slot - 2.0 * tooth_half);
    let cos_half = valley_half_span.cos();
    let denom = r_max * cos_half - r_min;
    let use_arc = denom.abs() > 1.0e-4;
    let (x_c, arc_r, half_delta) = if use_arc {
        let x_c = (r_max * r_max - r_min * r_min) / (2.0 * denom);
        let arc_r = (r_min - x_c).abs();
        let half_delta = ((x_c - r_max * cos_half) / arc_r).clamp(-1.0, 1.0).acos();
        (x_c, arc_r, half_delta)
    } else {
        (0.0, 0.0, 0.0)
    };

    let interior = valley_segments.saturating_sub(1);
    let mut valley_local: Vec<(f32, f32)> = Vec::with_capacity(interior);
    for k in 1..valley_segments {
        let t = k as f32 / valley_segments as f32;
        if use_arc {
            let u = 2.0 * t - 1.0;
            let phi = std::f32::consts::PI - half_delta * u;
            let (sin_phi, cos_phi) = phi.sin_cos();
            valley_local.push((x_c + arc_r * cos_phi, arc_r * sin_phi));
        } else {
            let r = r_min + (r_max - r_min) * (2.0 * t - 1.0).abs();
            let local_angle = (t - 0.5) * 2.0 * valley_half_span;
            let (sin_a, cos_a) = local_angle.sin_cos();
            valley_local.push((r * cos_a, r * sin_a));
        }
    }

    let mut pts: Vec<Pos2> = Vec::with_capacity(gear.tooth_count * (2 + interior));
    for i in 0..gear.tooth_count {
        let tooth_center = (i as f32 + 0.5) * slot + gear.rotation_rad;
        let a_left = tooth_center - tooth_half;
        let a_right = tooth_center + tooth_half;
        let (sin_l, cos_l) = a_left.sin_cos();
        pts.push(center + Vec2::new(cos_l, sin_l) * r_max);
        let (sin_r, cos_r) = a_right.sin_cos();
        pts.push(center + Vec2::new(cos_r, sin_r) * r_max);
        let mid_angle = a_right + valley_half_span;
        let (sin_mid, cos_mid) = mid_angle.sin_cos();
        for &(rx, ry) in &valley_local {
            pts.push(center + Vec2::new(rx * cos_mid - ry * sin_mid, rx * sin_mid + ry * cos_mid));
        }
    }
    out.add(Shape::Path(PathShape {
        points: pts,
        closed: true,
        fill: gear.fill,
        stroke: gear.stroke.stroke().into(),
    }));
}

fn collect_rotary_indicator(out: &mut ShapeList, center: Pos2, indicator: &RotaryIndicatorSpec) {
    let (sin_a, cos_a) = indicator.angle_cw_from_top_rad.sin_cos();
    let dir = Vec2::new(sin_a, -cos_a);
    let perp = Vec2::new(cos_a, sin_a);
    let half_w = indicator.width * 0.5;
    let inner_l = center + perp * -half_w;
    let inner_r = center + perp * half_w;
    let outer_r = inner_r + dir * indicator.length;
    let outer_l = inner_l + dir * indicator.length;
    out.add(Shape::closed_line(
        vec![inner_l, inner_r, outer_r, outer_l],
        indicator.stroke.stroke(),
    ));
}

// ── Button collect variants ───────────────────────────────────────────────────
// Mirror of the draw_button / draw_circle_button / draw_arc_quad_button API but
// writing into a ShapeList instead of directly into the egui Painter.
// The caller is responsible for calling ui.interact() separately and computing
// `is_pressed = response.is_pointer_button_down_on() || latched`.

fn collect_double_rect_border(
    out: &mut ShapeList,
    rect: Rect,
    rounding: Option<f32>,
    spec: DoubleBorderSpec,
) {
    let rounding = rounding.unwrap_or(0.0);
    let expand = spec.inner.width * 0.5 + spec.gap.max(0.0) + spec.outer.width * 0.5;
    let rect_outer = rect.expand(expand);
    let max_r = 0.5 * rect_outer.width().min(rect_outer.height());
    let rounding_outer = (rounding + expand).min(max_r);
    if rounding == 0.0 {
        out.rect_stroke(rect_outer, 0.0, spec.outer.stroke());
        out.rect_stroke(rect, 0.0, spec.inner.stroke());
    } else {
        out.rect_stroke(rect_outer, rounding_outer, spec.outer.stroke());
        out.rect_stroke(rect, rounding, spec.inner.stroke());
    }
}

/// Collect a basic rectangular button into `out`. No interaction - caller provides `is_pressed`.
fn collect_rotary_tick_ring(
    out: &mut ShapeList,
    center: Pos2,
    gear_outer_radius: f32,
    ticks: &RotaryTickSpec,
    color: Color32,
) {
    if ticks.count < 2 {
        return;
    }
    let ring_r = gear_outer_radius + ticks.ring_offset;
    let last = ticks.count - 1;
    let middle = last / 2;
    for i in 0..ticks.count {
        let cw = ticks.arc_start_cw_rad + (i as f32 / last as f32) * ticks.arc_span_rad;
        let (sin_cw, cos_cw) = cw.sin_cos();
        if i == 0 || i == last {
            let dir = Vec2::new(sin_cw, -cos_cw);
            let outer_r = ring_r * ENDCAP_RADIAL_FACTOR;
            let tick_pos = center + dir * outer_r;
            let inward = center + dir * (outer_r - ticks.endcap_length);
            out.line_segment([tick_pos, inward], ticks.endcap_stroke.stroke());
        } else {
            let dir = Vec2::new(sin_cw, -cos_cw);
            let radius = if i == middle {
                ticks.center_dot_radius
            } else {
                ticks.dot_radius
            };
            out.circle_filled(center + dir * ring_r, radius, color);
        }
    }
}
