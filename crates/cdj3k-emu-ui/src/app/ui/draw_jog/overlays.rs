//! The jog's live overlays: the pitchbend grip turning with the platter, the
//! ring lights the deck drives, and the inner LCD's corner labels in the
//! stream's colors.

use cdj3k_emu_panel::Lamp;
use egui::{epaint::Mesh, Color32, FontFamily, FontId, Pos2, Shape, Stroke, Vec2};
use std::f32::consts::TAU;

use super::super::draw_cache::ShapeList;
use super::super::{CdjApp, UiScale, COL_BTN_TEXT};
use super::*;

// --- Pitchbend band: a ring of grip ovals ---
/// Number of grip ovals evenly spaced around the pitchbend band.
pub(super) const JOG_GRIP_OVAL_COUNT: usize = 24;
/// The oval's radial half-length as a fraction of the pitchbend band width.
pub(super) const JOG_GRIP_OVAL_RADIAL_FRAC: f32 = 0.32;
/// The oval's tangential half-width as a fraction of one slot's arc length.
pub(super) const JOG_GRIP_OVAL_TANGENT_FRAC: f32 = 0.32;
/// Number of polygon segments that approximate each oval.
pub(super) const JOG_GRIP_POLYGON_SEGMENTS: usize = 32;
/// Outline stroke width of each oval, in reference units.
pub(super) const JOG_GRIP_OVAL_STROKE_REF: f32 = 6.0;

// --- The three dots between consecutive grip ovals ---
/// Number of decorative dots between each pair of consecutive ovals.
pub(super) const JOG_GRIP_DOT_COUNT: usize = 3;
/// Radius of each outlined dot, in reference units.
pub(super) const JOG_GRIP_DOT_RADIUS_REF: f32 = 10.0;
/// Total radial spread of the dot cluster from the outermost to the innermost dot, in
/// reference units.
pub(super) const JOG_GRIP_DOT_RADIAL_SPREAD_REF: f32 = 80.0;
/// Total tangential shift of the dot cluster in reference units; the dots step from left to
/// right as they go from the outer edge to the inner edge.
pub(super) const JOG_GRIP_DOT_TANGENT_SPREAD_REF: f32 = -32.0;

// ── Jog ring arc lights ───────────────────────────────────────────────────────
/// Half-angle of each glow arc, in degrees from its center.
pub(super) const JOG_RING_LIGHT_HALF_ANGLE_DEG: f32 = 25.0;
/// Number of polyline samples per arc light.
pub(super) const JOG_RING_LIGHT_ARC_SEGMENTS: usize = 15;
/// Core stroke width at the touch radius, in reference units.
pub(super) const JOG_RING_LIGHT_CORE_STROKE_REF: f32 = 10.0;
/// Number of outward glow layers.
pub(super) const JOG_RING_LIGHT_GLOW_LAYERS: usize = 25;
/// Total radial spread of the outer glow, in reference units.
pub(super) const JOG_RING_LIGHT_GLOW_SPREAD_REF: f32 = 30.0;
/// Total radial spread of the inner (platter) glow, in reference units.
pub(super) const JOG_RING_LIGHT_INNER_SPREAD_REF: f32 = 100.0;

impl CdjApp {
    /// Draws the pitchbend grip, cached by layout and by the rotation angle
    /// quantized to 4096 steps per revolution (about 0.088°). An idle jog causes
    /// no rebuilds.
    pub(super) fn paint_grip(
        &mut self,
        p: &egui::Painter,
        layout: &UiScale,
        center: Pos2,
        r_touch: f32,
        r_outer_1: f32,
    ) {
        let (ox, oy, scale) = layout.cache_key();
        let q = |v: f32| (v * 1000.0).round() as i32;
        let angle = self.jog_display_angle();
        let angle_q = (angle.rem_euclid(TAU) * 4096.0 / TAU).round() as i32;
        let r_inner = r_touch;
        let r_outer = r_outer_1;
        let oval_stroke = layout.sc(JOG_GRIP_OVAL_STROKE_REF);
        let dot_r = layout.sc(JOG_GRIP_DOT_RADIUS_REF);
        let dot_rad = layout.sc(JOG_GRIP_DOT_RADIAL_SPREAD_REF);
        let dot_tan = layout.sc(JOG_GRIP_DOT_TANGENT_SPREAD_REF);
        let key = (q(ox), q(oy), q(scale), angle_q);
        let grip_shapes = self.caches.grip.get_or_build(key, |out| {
            collect_pitchbend_grip(
                out,
                center,
                r_inner,
                r_outer,
                oval_stroke,
                dot_r,
                dot_rad,
                dot_tan,
                angle,
                COL_BTN_TEXT,
            );
        });
        self.frame_shape_count += grip_shapes.len() as u64;
        p.extend(grip_shapes.iter().cloned());
    }

    /// Draws the jog ring arc lights, cached per LED state.
    pub(super) fn paint_ring_lights(
        &mut self,
        p: &egui::Painter,
        layout: &UiScale,
        center: Pos2,
        r_touch: f32,
    ) {
        // Read the ring's brightness level and its red light from the MOSI frame.
        let mosi = self.mosi();
        let brt = mosi.step(Lamp::JogRing).level();
        let white_alpha: f32 = match brt {
            0 => 0.0,
            1 => 0.6,
            _ => 1.0,
        };
        let red_on = mosi.lit(Lamp::JogRed);
        let (light_color, light_alpha, glow_outer_mult, glow_inner_mult) = if red_on {
            (Color32::from_rgb(255, 70, 20), 1.0_f32, 0.35_f32, 0.2_f32)
        } else {
            (Color32::WHITE, white_alpha, 0.6_f32, 0.38_f32)
        };
        if light_alpha > 0.001 {
            let (ox, oy, scale) = layout.cache_key();
            let q = |v: f32| (v * 1000.0).round() as i32;
            let key = (q(ox), q(oy), q(scale), brt, red_on);
            let r_touch_sc = r_touch;
            let core_stroke = layout.sc(JOG_RING_LIGHT_CORE_STROKE_REF);
            let glow_spread = layout.sc(JOG_RING_LIGHT_GLOW_SPREAD_REF);
            let inner_spread = layout.sc(JOG_RING_LIGHT_INNER_SPREAD_REF);
            let shapes = self.caches.jog_ring_lights.get_or_build(key, |out| {
                collect_jog_ring_lights(
                    out,
                    center,
                    r_touch_sc,
                    core_stroke,
                    glow_spread,
                    inner_spread,
                    light_color,
                    light_alpha,
                    glow_outer_mult,
                    glow_inner_mult,
                );
            });
            self.frame_shape_count += shapes.len() as u64;
            p.extend(shapes.iter().cloned());
        }
    }

    /// Draws the SLIP, VINYL, SYNC and MASTER labels above the chrome, in the
    /// colors sampled from the corners of the last jog frame.
    pub(super) fn paint_jog_inner_lcd_corner_labels(
        &mut self,
        p: &Painter,
        ctx: &egui::Context,
        center: Pos2,
        layout: &UiScale,
        colors: [Color32; 4],
    ) {
        let (ox, oy, scale) = layout.cache_key();
        let q = |v: f32| (v * 1000.0).round() as i32;
        let mut color_bytes = [0u8; 16];
        for (i, c) in colors.iter().enumerate() {
            let b = c.to_array();
            color_bytes[i * 4..i * 4 + 4].copy_from_slice(&b);
        }
        let ppp_q = (ctx.pixels_per_point() * 100.0).round() as i32;
        let key = (q(ox), q(oy), q(scale), ppp_q, color_bytes);
        let shapes = self.caches.jog_corner_labels.get_or_build(key, |out| {
            collect_jog_corner_labels(out, ctx, center, layout, colors);
        });
        self.frame_shape_count += shapes.len() as u64;
        p.extend(shapes.iter().cloned());
    }
}

fn collect_jog_corner_labels(
    out: &mut ShapeList,
    ctx: &egui::Context,
    center: Pos2,
    layout: &UiScale,
    colors: [Color32; 4],
) {
    let r_inner_lcd = layout.sc(JOG_INNER_LCD_C1_RADIUS);
    let c3 = r_inner_lcd * JOG_INNER_LCD_C3_RADIUS_FRAC;
    let label_font = FontId::new(
        layout.sc(JOG_INNER_LCD_LABEL_FONT_REF),
        FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS_BOLD.into()),
    );
    let cutout_outline_px = layout.sc(JOG_INNER_LCD_CUTOUT_OUTLINE_REF);
    let bottom = JOG_INNER_LCD_INVERT_BOTTOM_CORNER_LABELS;
    for (angle, text, color, invert) in [
        (315.0, "SLIP", colors[0], false),
        (45.0, "VINYL", colors[1], false),
        (136.0, "MASTER", colors[3], bottom),
        (225.0, "SYNC", colors[2], bottom),
    ] {
        geometry::inner_lcd_push_curved_label(
            out,
            ctx,
            center,
            c3,
            angle,
            text,
            label_font.clone(),
            color,
            true,
            cutout_outline_px,
            invert,
        );
    }
}

/// Paints the pitchbend band grip pattern: `JOG_GRIP_OVAL_COUNT` outlined ovals
/// evenly spaced on the band's mean radius, with `JOG_GRIP_DOT_COUNT` dots between
/// each pair that step diagonally from the outer left to the inner right.
///
/// `rotation_rad` is added to every element's base angle, so the pattern rotates
/// as a rigid ring. A positive value rotates it clockwise on screen, like
/// `jog_angle`, because screen y points down and each angle is measured with
/// `(cos, sin)` in screen space.
fn collect_pitchbend_grip(
    out: &mut ShapeList,
    center: Pos2,
    r_inner: f32,
    r_outer: f32,
    oval_stroke_w: f32,
    dot_radius: f32,
    dot_radial_spread: f32,
    dot_tangent_spread: f32,
    rotation_rad: f32,
    stroke_color: Color32,
) {
    let band_mid = (r_inner + r_outer) * 0.5;
    let band_width = r_outer - r_inner;
    let slot_arc = TAU / JOG_GRIP_OVAL_COUNT as f32;

    let oval_radial_half = band_width * JOG_GRIP_OVAL_RADIAL_FRAC;
    let oval_tangent_half = band_mid * slot_arc * JOG_GRIP_OVAL_TANGENT_FRAC;
    let oval_stroke = Stroke::new(oval_stroke_w, stroke_color);

    for i in 0..JOG_GRIP_OVAL_COUNT {
        let angle = rotation_rad + i as f32 * slot_arc;
        let (sin_a, cos_a) = angle.sin_cos();
        let radial = Vec2::new(cos_a, sin_a);
        let tangent = Vec2::new(-sin_a, cos_a);

        let oval_center = center + radial * band_mid;
        let pts: Vec<Pos2> = (0..JOG_GRIP_POLYGON_SEGMENTS)
            .map(|j| {
                let t = j as f32 / JOG_GRIP_POLYGON_SEGMENTS as f32 * TAU;
                let (sin_t, cos_t) = t.sin_cos();
                oval_center
                    + radial * (oval_radial_half * cos_t)
                    + tangent * (oval_tangent_half * sin_t)
            })
            .collect();
        out.add(Shape::closed_line(pts, oval_stroke));

        let gap_angle = angle + slot_arc * 0.5;
        let (sin_g, cos_g) = gap_angle.sin_cos();
        let gap_radial = Vec2::new(cos_g, sin_g);
        let gap_tangent = Vec2::new(-sin_g, cos_g);

        for k in 0..JOG_GRIP_DOT_COUNT {
            let u = if JOG_GRIP_DOT_COUNT <= 1 {
                0.0
            } else {
                0.5 - k as f32 / (JOG_GRIP_DOT_COUNT - 1) as f32
            };
            let dot_center = center
                + gap_radial * (band_mid + u * dot_radial_spread)
                + gap_tangent * (-u * dot_tangent_spread);
            out.circle_stroke(
                dot_center,
                dot_radius,
                Stroke::new(oval_stroke_w, stroke_color),
            );
        }
    }
}

/// Collects the four arc lights around the touch platter's edge at `alpha`. Each
/// light is a core stroke with outer and inner (platter) glow layers.
fn collect_jog_ring_lights(
    out: &mut ShapeList,
    center: Pos2,
    r: f32,
    core_stroke: f32,
    glow_spread: f32,
    inner_spread: f32,
    color: Color32,
    alpha: f32,
    glow_outer_mult: f32,
    glow_inner_mult: f32,
) {
    let half_rad = JOG_RING_LIGHT_HALF_ANGLE_DEG.to_radians();
    let n = JOG_RING_LIGHT_ARC_SEGMENTS;

    let arc_fade = |t: f32| -> f32 {
        let x = (t - 0.5) * 2.0;
        (1.0 - x * x).max(0.0).powf(1.4)
    };

    for cw_deg in [45.0_f32, 135.0, 225.0, 315.0] {
        let center_angle = (cw_deg - 90.0).to_radians();

        let draw_arc_mesh = |out: &mut ShapeList, radius: f32, stroke_w: f32, base_alpha: f32| {
            if base_alpha < 0.004 {
                return;
            }
            let half_w = stroke_w * 0.5;
            let mut mesh = Mesh::default();
            for i in 0..=n {
                let t = i as f32 / n as f32;
                let a = center_angle - half_rad + 2.0 * half_rad * t;
                let (sa, ca) = a.sin_cos();
                let dir = Vec2::new(ca, sa);
                let vert_col = color.gamma_multiply(base_alpha * arc_fade(t));
                mesh.colored_vertex(center + dir * (radius - half_w), Color32::TRANSPARENT);
                mesh.colored_vertex(center + dir * radius, vert_col);
                mesh.colored_vertex(center + dir * (radius + half_w), Color32::TRANSPARENT);
                if i > 0 {
                    let b = ((i - 1) * 3) as u32;
                    mesh.add_triangle(b, b + 3, b + 1);
                    mesh.add_triangle(b + 1, b + 3, b + 4);
                    mesh.add_triangle(b + 1, b + 4, b + 2);
                    mesh.add_triangle(b + 2, b + 4, b + 5);
                }
            }
            out.add(Shape::mesh(mesh));
        };

        draw_arc_mesh(out, r, core_stroke, alpha);
        for layer in 1..=JOG_RING_LIGHT_GLOW_LAYERS {
            let t = layer as f32 / JOG_RING_LIGHT_GLOW_LAYERS as f32;
            draw_arc_mesh(
                out,
                r + glow_spread * t,
                core_stroke * (1.0 + t * 3.0),
                alpha * (1.0 - t) * glow_outer_mult,
            );
        }
        for layer in 1..=3usize {
            let t = layer as f32 / 3.0;
            draw_arc_mesh(
                out,
                (r - inner_spread * t).max(0.0),
                core_stroke * (1.0 + t * 1.5),
                alpha * (1.0 - t) * glow_inner_mult,
            );
        }
    }
}
