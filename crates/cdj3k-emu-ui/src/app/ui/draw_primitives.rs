use std::f32::consts::{FRAC_PI_2, PI, TAU};

use egui::epaint::{PathShape, PathStroke};
use egui::{Color32, Pos2, Rect, Shape, Stroke, Vec2};

use crate::app::ui::{draw_cache::ShapeList, COL_BTN};

mod buttons;
mod glyphs;
pub(in crate::app) use buttons::*;
pub(in crate::app) use glyphs::*;

/// Width and color of a stroked outline, such as a rect border or a ring stroke.
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

/// Default clear distance between the inner and outer strokes, in screen px.
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

/// Two concentric strokes: [`inner`](Self::inner) on the shape's edge, then a clear
/// [`gap`](Self::gap), then [`outer`](Self::outer) farther out.
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct DoubleBorderSpec {
    pub inner: StrokeSpec,
    pub gap: f32,
    pub outer: StrokeSpec,
}

impl DoubleBorderSpec {
    /// Builds the inner and outer strokes with [`DEFAULT_DOUBLE_BORDER_GAP`] between them.
    #[inline]
    pub(in crate::app) fn from_strokes(inner: StrokeSpec, outer: StrokeSpec) -> Self {
        DoubleBorderSpacing::default().apply(inner, outer)
    }

    /// Builds the inner and outer strokes with `gap` between them.
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
    paint_collected(painter, |out| {
        paint_double_circle_ring_collect(out, center, radius, spec)
    });
}

/// Paints what `collect` adds to a fresh [`ShapeList`].
fn paint_collected(painter: &egui::Painter, collect: impl FnOnce(&mut ShapeList)) {
    let mut list = ShapeList::default();
    collect(&mut list);
    painter.extend(list.into_shapes());
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::app) enum ButtonType {
    Basic,
    HotCue,
    /// Like Basic, but the colored fill is drawn in an inset rect around the
    /// label only; the button's outer rect and double border keep their
    /// size. `pad_h` and `pad_v` are paddings in pixels around the label's
    /// glyph rect.
    InsetFill {
        pad_h: f32,
        pad_v: f32,
    },
}

/// An open USB tray, folded up against its casing's top wall: a thin panel
/// with the handle's dome standing on its top edge. Its coordinates are in
/// screen space.
pub(in crate::app) struct UsbTraySpec {
    pub centre_x: f32,
    pub top: f32,
    pub bottom: f32,
    pub half_w: f32,
    /// The dome's chord along the panel's top edge, and its height above it.
    pub dome_chord: f32,
    pub dome_h: f32,
    pub stroke: f32,
}

pub(in crate::app) fn draw_usb_tray(painter: &egui::Painter, tray: UsbTraySpec) {
    use crate::app::ui::{COL_BTN_HOT, COL_SILVER};
    let outline = Stroke::new(tray.stroke, COL_SILVER);
    let half_c = tray.dome_chord * 0.5;
    let sag = tray.dome_h;
    let r = (half_c * half_c + sag * sag) / (2.0 * sag);
    let centre = Pos2::new(tray.centre_x, tray.top - sag + r);
    let half_angle = (half_c / r).asin();
    const DOME_STEPS: usize = 24;
    let dome: Vec<Pos2> = (0..=DOME_STEPS)
        .map(|k| {
            let a = -half_angle + 2.0 * half_angle * k as f32 / DOME_STEPS as f32;
            Pos2::new(centre.x + r * a.sin(), centre.y - r * a.cos())
        })
        .collect();
    let panel = Rect::from_min_max(
        Pos2::new(tray.centre_x - tray.half_w, tray.top),
        Pos2::new(tray.centre_x + tray.half_w, tray.bottom),
    );
    painter.rect_filled(panel, 0.0, COL_BTN_HOT);
    // The handle is a finger recess, so it reads darker than the panel.
    painter.add(Shape::convex_polygon(dome.clone(), COL_BTN, Stroke::NONE));
    let mut edge = vec![panel.left_bottom(), panel.left_top()];
    edge.extend(dome);
    edge.extend([panel.right_top(), panel.right_bottom()]);
    painter.add(Shape::line(edge, outline));
}

/// Draws a rounded-rect section with optional fill and a double border.
/// It works in screen space, so the caller converts `rect` with `layout.sp` or
/// `layout.sr` first.
pub(in crate::app) fn draw_bordered_rect_section(
    painter: &egui::Painter,
    rect: Rect,
    rounding: Option<f32>,
    fill: Option<Color32>,
    border: DoubleBorderSpec,
) {
    paint_collected(painter, |out| {
        collect_bordered_rect_section(out, rect, rounding, fill, border)
    });
}

/// Collects [`draw_bordered_rect_section`] into a [`ShapeList`] for static caching.
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
// Rotary control: a gear knob, an outlined indicator and a tick ring
// ---------------------------------------------------------------------------

/// Returns the position of the pointer at offset `v` from a rotary's centre
/// along its tick arc, which starts `arc_start_cw` clockwise from the top and
/// spans `arc_span`. The result is 0 at the first tick and 1 at the last, and
/// is not clamped.
pub(in crate::app) fn rotary_arc_fraction(v: Vec2, arc_start_cw: f32, arc_span: f32) -> f32 {
    let ang = (v.y.atan2(v.x) + FRAC_PI_2).rem_euclid(TAU);
    let mid = (arc_start_cw + arc_span * 0.5).rem_euclid(TAU);
    let mut rel = ang - mid;
    if rel > PI {
        rel -= TAU;
    } else if rel < -PI {
        rel += TAU;
    }
    0.5 + rel / arc_span
}

/// Adds `scroll` to `accum`, removes every whole detent of `px_per_detent`
/// from it, and returns the number removed; the number is positive for
/// scrolling up.
pub(in crate::app) fn scroll_detents(accum: &mut f32, scroll: f32, px_per_detent: f32) -> i32 {
    *accum += scroll;
    let mut detents = 0;
    while *accum >= px_per_detent {
        *accum -= px_per_detent;
        detents += 1;
    }
    while *accum <= -px_per_detent {
        *accum += px_per_detent;
        detents -= 1;
    }
    detents
}

/// A gear knob outline: flat-topped teeth joined by circular-arc valleys. Each
/// valley lies on the circle through the two adjacent tooth-top corners (at
/// `r_max`) and the valley midpoint (at `r_min`), so its curvature is constant.
/// The arc is drawn as [`valley_segments`](Self::valley_segments) straight
/// chords of equal angular span about the arc's own center.
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct RotaryGearSpec {
    /// The valley-bottom radius, in screen px.
    pub base_radius: f32,
    /// Number of teeth around the gear.
    pub tooth_count: usize,
    /// Radial distance from `base_radius` to the flat tooth top, in screen px.
    pub tooth_amplitude: f32,
    /// The fraction of each slot (`2π / tooth_count`) taken by the flat tooth
    /// top; the rest is the valley between teeth.
    pub tooth_top_fraction: f32,
    /// Number of straight chords that approximate each circular valley arc.
    /// With `2`, the only sample is at the valley bottom and the valley is a
    /// straight V; with 8 or more, the valley is a smooth arc. An even value
    /// places a sample exactly on the valley bottom.
    pub valley_segments: usize,
    /// The rotation of the whole gear in radians, measured with `(cos, sin)`
    /// in screen space, where y points down. Use
    /// `indicator_angle_cw_from_top - FRAC_PI_2` to keep the gear in phase with
    /// the indicator. With `0.0` the gear is not rotated, and tooth 0 is centered
    /// half a slot clockwise of 3 o'clock.
    pub rotation_rad: f32,
    /// Fill color of the gear body inside the outline. Use
    /// [`Color32::TRANSPARENT`] for no fill.
    pub fill: Color32,
    /// Outline stroke.
    pub stroke: StrokeSpec,
}

/// An outlined rectangular indicator whose long axis points outward from
/// `center` at [`angle_cw_from_top_rad`](Self::angle_cw_from_top_rad), measured
/// clockwise from straight up like the tick ring.
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct RotaryIndicatorSpec {
    /// Length of the indicator from the knob center to the outer tip, in
    /// screen px.
    pub length: f32,
    /// Width of the indicator across its long axis, in screen px.
    pub width: f32,
    /// Pointing direction, in radians clockwise from straight up. `0.0` points
    /// up, `π/2` points right, and so on.
    pub angle_cw_from_top_rad: f32,
    /// Outline stroke.
    pub stroke: StrokeSpec,
}

/// The endcap ring's radius relative to the dot ring, so that the minimum and
/// maximum lines sit slightly outside the dots.
const ENDCAP_RADIAL_FACTOR: f32 = 1.2;

/// A tick ring along an arc. The first and last ticks are short radial lines
/// (the endcaps); the others are filled dots, and the middle dot is slightly
/// larger to mark the center.
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct RotaryTickSpec {
    /// Total number of ticks, including the two endcap lines.
    pub count: usize,
    /// Angle of the first tick, in radians clockwise from the top.
    pub arc_start_cw_rad: f32,
    /// Clockwise angular span from the first to the last tick, in radians.
    pub arc_span_rad: f32,
    /// Radial distance from the gear's outer edge to the tick ring, in screen px.
    pub ring_offset: f32,
    /// Radius of the regular dots, in screen px.
    pub dot_radius: f32,
    /// Radius of the center dot, slightly larger than [`dot_radius`](Self::dot_radius).
    pub center_dot_radius: f32,
    /// Radial length of the endcap lines, in screen px.
    pub endcap_length: f32,
    /// Stroke of the endcap lines.
    pub endcap_stroke: StrokeSpec,
}

/// A full rotary control: a gear, an indicator and a tick ring, painted in
/// [`color`](Self::color).
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct RotaryControlSpec {
    pub gear: RotaryGearSpec,
    pub indicator: RotaryIndicatorSpec,
    pub ticks: RotaryTickSpec,
    /// Color of the tick dots; strokes use their own [`StrokeSpec::color`].
    pub color: Color32,
    /// Whether to draw the indicator. It is `false` for a bare gear, such as the
    /// nav rotary.
    pub draw_indicator: bool,
    /// Whether to draw the tick ring. It is `false` for a bare gear.
    pub draw_ticks: bool,
}

impl RotaryControlSpec {
    /// Returns a bare gear in `color`, with no indicator and no tick ring.
    pub(in crate::app) fn gear_only(gear: RotaryGearSpec, color: Color32) -> Self {
        let none = StrokeSpec { width: 0.0, color };
        Self {
            gear,
            indicator: RotaryIndicatorSpec {
                length: 0.0,
                width: 0.0,
                angle_cw_from_top_rad: 0.0,
                stroke: none,
            },
            ticks: RotaryTickSpec {
                count: 0,
                arc_start_cw_rad: 0.0,
                arc_span_rad: 0.0,
                ring_offset: 0.0,
                dot_radius: 0.0,
                center_dot_radius: 0.0,
                endcap_length: 0.0,
                endcap_stroke: none,
            },
            color,
            draw_indicator: false,
            draw_ticks: false,
        }
    }
}

/// Paints a rotary control, with its gear knob, outlined indicator and tick
/// ring, centered on `center`.
pub(in crate::app) fn draw_rotary_control(
    painter: &egui::Painter,
    center: Pos2,
    spec: RotaryControlSpec,
) {
    paint_collected(painter, |out| {
        draw_rotary_control_collect(out, center, spec)
    });
}

/// A decorative arc drawn inside an arc button, between its inner and outer edges.
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct ArcDecorSpec {
    /// Radius of the arc from the button's center point, in screen px.
    pub radius: f32,
    pub stroke: Stroke,
}

/// A notch arc drawn between the bezel boundary and the decorative arc. It has a
/// shorter angular span and a thicker stroke than the decorative arc.
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

// ── Variants that collect into a ShapeList ───────────────────────────────────
// These write into a [`ShapeList`] so that the geometry can be cached and
// replayed; the painter functions above paint what they collect.

/// Collects a filled capsule that encloses the circles of `radius` at
/// `center_left` and `center_right`, with a border of `thickness`.
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

/// Collects a full rotary control, with its gear, optional indicator and optional
/// tick ring, into a [`ShapeList`] for static caching.
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

    // Every valley is congruent, with the same r_min, r_max and angular span.
    // The arc geometry and the interior sample offsets are computed once in a
    // valley-local frame (+x along the valley's midpoint radial, origin at the
    // gear center). Each tooth then rotates these offsets by its midpoint
    // angle, so the inner loop needs only 2-D rotations and no per-tooth
    // `sin_cos` or `acos`.
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
            // A degenerate arc becomes a straight V from r_max to r_min and back.
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

        // The flat tooth top at r_max.
        let (sin_l, cos_l) = a_left.sin_cos();
        pts.push(center + Vec2::new(cos_l, sin_l) * r_max);
        let (sin_r, cos_r) = a_right.sin_cos();
        pts.push(center + Vec2::new(cos_r, sin_r) * r_max);

        // Rotate the valley samples from the valley-local frame to the screen.
        let mid_angle = a_right + valley_half_span;
        let (sin_mid, cos_mid) = mid_angle.sin_cos();
        for &(rx, ry) in &valley_local {
            pts.push(center + Vec2::new(rx * cos_mid - ry * sin_mid, rx * sin_mid + ry * cos_mid));
        }
    }
    // The gear polygon is star-shaped about the center, so egui's tessellator
    // handles the fill correctly despite the concave valleys.
    out.add(Shape::Path(PathShape {
        points: pts,
        closed: true,
        fill: gear.fill,
        stroke: gear.stroke.stroke().into(),
    }));
}

fn collect_rotary_indicator(out: &mut ShapeList, center: Pos2, indicator: &RotaryIndicatorSpec) {
    // `dir` is the outward unit vector along the long axis, and `perp` is the
    // perpendicular used for the width. An angle clockwise from the top maps to
    // screen space (+x right, +y down) as (sin θ, -cos θ); `perp` is `dir`
    // turned 90° clockwise.
    let (sin_a, cos_a) = indicator.angle_cw_from_top_rad.sin_cos();
    let dir = Vec2::new(sin_a, -cos_a);
    let perp = Vec2::new(cos_a, sin_a);
    let half_w = indicator.width * 0.5;

    // The four corners of the rotated rectangle; the outer tip is flat. The
    // inner end sits at `center`, under the gear body, so it needs no rounding.
    let inner_l = center + perp * -half_w;
    let inner_r = center + perp * half_w;
    let outer_r = inner_r + dir * indicator.length;
    let outer_l = inner_l + dir * indicator.length;
    out.add(Shape::closed_line(
        vec![inner_l, inner_r, outer_r, outer_l],
        indicator.stroke.stroke(),
    ));
}

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
            // Convert the clockwise angle from the top to a screen-space unit
            // vector.
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
