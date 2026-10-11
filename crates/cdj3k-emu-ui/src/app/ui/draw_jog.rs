//! The jog wheel assembly: concentric rings (the inner LCD, the touch platter, the
//! pitchbend grip band and the outer border) centered in the slate's jog zone
//! `jog_ref`, in reference units. Every slate uses it. The radii below are absolute
//! reference units, so a slate's jog zone must be `2 * JOG_OUTER_2_STROKE_RADIUS`
//! wide for the rings to fit.
//!
//! The rings are drawn as filled disks from the outermost to the innermost, so each
//! disk covers the inside of the previous one and leaves a clean annulus without
//! `circle_stroke`.

use egui::{
    epaint::{Mesh, Vertex},
    Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2,
};
use std::f32::consts::TAU;

use crate::app::ui::{
    draw_cache::{JogCacheKey, JogStaticCache, ShapeList},
    COL_BTN, COL_WHITE,
};
use cdj3k_emu_streams::jog_stream::{JOG_FB_H, JOG_FB_W};

use super::{CdjApp, UiScale};

mod geometry;
mod interact;
mod overlays;
mod statics;

/// The parts of the jog assembly that differ between slates.
#[derive(Clone, Copy)]
pub(in crate::app) struct JogChrome {
    /// The legend above the jog-feel rotary, such as "JOG ADJUST" or "JOG FEEL".
    pub adjust_label: &'static str,
    /// The − REV / + FWD arrows around the platter's lower half.
    pub nav_arrows: bool,
}

// --- Jog wheel radii, in reference units, from the center outward ---
/// The reference radius for scaling the jog stream framebuffer. It is slightly larger
/// than the c1 bezel, so the live LCD looks a little bigger on screen.
pub(super) const JOG_INNER_LCD_C0_RADIUS: f32 = 410.0;
pub(super) const JOG_INNER_LCD_C1_RADIUS: f32 = 340.0;
/// The half-width of the inner LCD silhouette's rectangle as a fraction of c1
/// (JOG_INNER_LCD_C1_RADIUS). The rectangle's height puts its corners on c1.
pub(super) const JOG_INNER_LCD_RECT_HALF_WIDTH_FRAC: f32 = 0.4;
/// The c2 radius of the inner LCD silhouette as a fraction of c1.
pub(super) const JOG_INNER_LCD_C2_RADIUS_FRAC: f32 = 0.7;
/// The c3 radius of the label ring as a fraction of c1. It must be larger than
/// [`JOG_INNER_LCD_C2_RADIUS_FRAC`].
pub(super) const JOG_INNER_LCD_C3_RADIUS_FRAC: f32 = 0.81;
/// Font size of the curved labels on the inner LCD, in reference units.
pub(super) const JOG_INNER_LCD_LABEL_FONT_REF: f32 = 27.0;
/// Stroke width of the c3 separator arcs, in reference units.
pub(super) const JOG_INNER_LCD_C3_ARC_STROKE_REF: f32 = 5.0;
/// Polyline samples per c3 arc span.
pub(super) const JOG_INNER_LCD_C3_ARC_SEGMENTS: usize = 48;
/// The offset, in reference units, of the dark outline drawn behind LCD-colored glyphs.
pub(super) const JOG_INNER_LCD_CUTOUT_OUTLINE_REF: f32 = 1.1;
/// Small overlap added at line/arc joints so there is no 1px seam.
pub(super) const JOG_INNER_LCD_JOIN_OVERLAP_REF: f32 = 2.0;
/// Samples per circular arc when building the inner-LCD silhouette mesh.
pub(super) const JOG_INNER_LCD_MESH_ARC_SEGMENTS: usize = 48;
/// When `false`, the full jog LCD rectangle is drawn without the silhouette clip, for
/// layout checks.
pub(super) const JOG_LCD_USE_SILHOUETTE_MASK: bool = true;
/// When `true`, π is added to the glyph rotation of the bottom-arc labels (SYNC, MASTER
/// and BEAT SYNC) so that they read upright.
pub(super) const JOG_INNER_LCD_INVERT_BOTTOM_CORNER_LABELS: bool = true;
pub(super) const JOG_TOUCH_RADIUS: f32 = 835.0;
pub(super) const JOG_OUTER_1_STROKE_RADIUS: f32 = 1000.0;
pub(super) const JOG_OUTER_2_STROKE_RADIUS: f32 = 1060.0;
pub(super) const JOG_OUTER_2_STROKE_GAP: f32 = 0.0;

// --- Position indicator across the outer band ---
/// The tangential half-width of the slim white position indicator, in reference units.
pub(super) const JOG_POS_INDICATOR_HALF_WIDTH_REF: f32 = 8.0;

// --- Double border on the touch platter edge ---
/// Width of the inner stroke, which sits on [`JOG_TOUCH_RADIUS`], in reference units.
pub(super) const JOG_TOUCH_BORDER_INNER_WIDTH_REF: f32 = 5.0;
/// Width of the outer stroke, offset outward by [`JOG_TOUCH_BORDER_GAP_REF`], in
/// reference units.
pub(super) const JOG_TOUCH_BORDER_OUTER_WIDTH_REF: f32 = 3.0;
/// The clear distance between the inner and outer strokes, in reference units.
pub(super) const JOG_TOUCH_BORDER_GAP_REF: f32 = 6.0;

// --- JOG ADJUST rotary knob, at the top right of the jog panel ---
/// A size multiplier for the whole rotary control: the knob, indicator, tick ring
/// and labels. At `1.0` the sizes below apply as they are. Counts, angles and
/// fractions are not affected.
pub(super) const JOG_ADJUST_SIZE_SCALE: f32 = 0.5;
/// The knob's center as fractions (u, v) of the jog zone, each in [0, 1].
pub(super) const JOG_ADJUST_CENTER_U: f32 = 0.91;
pub(super) const JOG_ADJUST_CENTER_V: f32 = 0.11;
/// The valley (tooth-bottom) radius of the gear knob, in reference units.
pub(super) const JOG_ADJUST_KNOB_RADIUS_REF: f32 = 105.0;
/// Number of gear teeth around the knob edge.
pub(super) const JOG_ADJUST_TOOTH_COUNT: usize = 12;
/// The radial height from a valley to a tooth top, in reference units.
pub(super) const JOG_ADJUST_TOOTH_AMP_REF: f32 = 14.0;
/// The fraction of each angular slot taken by the flat tooth top; the rest is the
/// circular-arc valley between teeth. A lower value gives narrower teeth and wider
/// valleys.
pub(super) const JOG_ADJUST_TOOTH_TOP_FRAC: f32 = 0.35;
/// Chords per valley between two teeth. An even value places a sample exactly on
/// the valley bottom.
pub(super) const JOG_ADJUST_VALLEY_SEGMENTS: usize = 4;
/// Outline stroke width of the gear (reference units).
pub(super) const JOG_ADJUST_STROKE_REF: f32 = 7.0;
/// Fill color of the gear body inside the outline.
pub(super) const JOG_ADJUST_FILL_COLOR: Color32 = COL_BTN;
/// Length of the indicator pill as a fraction of the knob's base radius.
pub(super) const JOG_ADJUST_INDICATOR_LENGTH_FRAC: f32 = 1.2;
/// Width of the indicator pill (reference units).
pub(super) const JOG_ADJUST_INDICATOR_WIDTH_REF: f32 = 14.0;
/// Outline stroke width of the indicator pill (reference units).
pub(super) const JOG_ADJUST_INDICATOR_STROKE_REF: f32 = 10.0;
/// Total number of ticks on the detent ring, including the two endcap lines.
pub(super) const JOG_ADJUST_TICK_COUNT: usize = 13;
/// The angle of the minimum tick, in radians clockwise from the top. 225° (5π/4)
/// places "LIGHT" at the bottom left, at 7:30.
pub(super) const JOG_ADJUST_TICK_ARC_START_CW_RAD: f32 = 5.0 * std::f32::consts::PI / 4.0;
/// The clockwise span from the minimum to the maximum tick. 270° (3π/2) sweeps over
/// the top.
pub(super) const JOG_ADJUST_TICK_ARC_SPAN_RAD: f32 = 3.0 * std::f32::consts::PI / 2.0;
/// The rotation motion blur intensity. 0 disables it; a larger value gives a longer,
/// stronger trail.
pub(super) const JOG_ROTATION_MOTION_BLUR_INTENSITY: f32 = 3.0;
/// Number of trailing copies drawn for the rotation blur.
pub(super) const JOG_ROTATION_MOTION_BLUR_STEPS: usize = 64;
/// The longest angular trail at peak speed, in radians.
pub(super) const JOG_ROTATION_MOTION_BLUR_MAX_TRAIL_RAD: f32 = 1.0;
/// Radial distance of the tick ring from the knob's outer edge, in reference units.
pub(super) const JOG_ADJUST_TICK_RING_OFFSET_REF: f32 = 40.0;
/// Radius of the regular tick dots, in reference units.
pub(super) const JOG_ADJUST_TICK_DOT_RADIUS_REF: f32 = 6.0;
/// Radius of the center tick dot, slightly larger than the regular dots, in reference
/// units.
pub(super) const JOG_ADJUST_TICK_CENTER_DOT_RADIUS_REF: f32 = 9.0;
/// Radial length of the minimum and maximum endcap lines, in reference units.
pub(super) const JOG_ADJUST_TICK_ENDCAP_LENGTH_REF: f32 = 50.0;
/// Stroke width of the minimum and maximum endcap lines, in reference units.
pub(super) const JOG_ADJUST_TICK_ENDCAP_STROKE_REF: f32 = 10.0;
/// Font size for the "JOG ADJUST" title label.
pub(super) const JOG_ADJUST_TOP_LABEL_FONT_SIZE: f32 = 66.0;
/// Distance from the top of the knob to the title baseline, in reference units.
pub(super) const JOG_ADJUST_TOP_LABEL_GAP_REF: f32 = 99.0;
/// Font size for the "LIGHT" / "HEAVY" side labels.
pub(super) const JOG_ADJUST_SIDE_LABEL_FONT_SIZE: f32 = 40.0;
/// Distance from the bottom of the knob to the side labels, in reference units.
pub(super) const JOG_ADJUST_SIDE_LABEL_GAP_REF: f32 = 28.0;
/// Horizontal offset from the knob's vertical axis to each side label, in reference
/// units.
pub(super) const JOG_ADJUST_SIDE_LABEL_X_OFFSET_REF: f32 = 140.0;

impl CdjApp {
    /// Draws the jog wheel. `jog_ref` is the slate's jog zone in reference units.
    pub(super) fn draw_jog_wheel_section(
        &mut self,
        ui: &mut egui::Ui,
        p: &egui::Painter,
        layout: &UiScale,
        jog_ref: Rect,
        chrome: JogChrome,
    ) {
        puffin::profile_function!();
        let jog_panel = Rect::from_min_max(
            layout.sp(jog_ref.left(), jog_ref.top()),
            layout.sp(jog_ref.right(), jog_ref.bottom()),
        );
        let center = jog_panel.center();
        let r_touch = layout.sc(JOG_TOUCH_RADIUS);
        let r_outer_1 = layout.sc(JOG_OUTER_1_STROKE_RADIUS);

        self.jog_interact(ui, layout, center, r_touch, r_outer_1);

        self.paint_jog_adjust_interact(ui, layout, jog_ref);

        self.refresh_jog_static_cache(ui, layout, center, jog_ref, chrome);

        // Draw the static outer rings, then the dynamic grip, then the static
        // inner rings.
        let (outer, inner_mid, inner_over) = {
            let cache = self.jog_static_cache.as_ref().unwrap();
            (
                cache.outer.clone(),
                cache.inner_mid.clone(),
                cache.inner_over.clone(),
            )
        };
        self.frame_shape_count += outer.len() as u64;
        p.extend(outer);

        self.paint_motion_blur(p, layout, center);
        self.paint_grip(p, layout, center, r_touch, r_outer_1);

        self.frame_shape_count += inner_mid.len() as u64;
        p.extend(inner_mid);
        if let Some(r) = self.paint_jog_lcd_stream_texture(p, center, layout) {
            self.bloom_excludes.push(r);
        }
        self.frame_shape_count += inner_over.len() as u64;
        p.extend(inner_over);

        // The static overlays (labels and fixed circles) are rebuilt only on a
        // resize.
        let ctx = ui.ctx().clone();
        let ppp = ui.ctx().pixels_per_point();
        self.frame_shape_count += self.caches.jog_statics.paint(p, layout, ppp, |list| {
            statics::collect_jog_statics(list, &ctx, layout, center, jog_ref, chrome);
        });

        self.paint_jog_inner_lcd_corner_labels(
            p,
            ui.ctx(),
            center,
            layout,
            self.jog_corner_label_colors,
        );
        self.paint_ring_lights(p, layout, center, r_touch);

        // The position indicator is a slim white quad across the outer band,
        // from r_outer_1 to r_outer_2, that rotates with the jog.
        let pts = indicator_quad(center, self.jog_display_angle(), layout);
        p.add(Shape::convex_polygon(pts, COL_WHITE, Stroke::NONE));
    }

    /// Rebuilds the static cache when the layout or jog_adjust changes.
    fn refresh_jog_static_cache(
        &mut self,
        ui: &egui::Ui,
        layout: &UiScale,
        center: Pos2,
        jog_ref: Rect,
        chrome: JogChrome,
    ) {
        let cache_key = JogCacheKey::new(
            layout.ox,
            layout.oy,
            layout.scale,
            self.jog_adjust(),
            ui.ctx().pixels_per_point(),
        );
        if self
            .jog_static_cache
            .as_ref()
            .is_none_or(|c| c.key != cache_key)
        {
            let mut outer = ShapeList::default();
            let mut inner_mid = ShapeList::default();
            let mut inner_over = ShapeList::default();
            statics::build_jog_outer(&mut outer, center, layout);
            statics::build_jog_inner(
                &mut inner_mid,
                &mut inner_over,
                ui.ctx(),
                center,
                layout,
                self.jog_adjust(),
                jog_ref,
                chrome,
            );
            self.jog_static_cache = Some(JogStaticCache {
                key: cache_key,
                outer: outer.into_shapes(),
                inner_mid: inner_mid.into_shapes(),
                inner_over: inner_over.into_shapes(),
            });
        }
    }

    /// Draws the rotation motion blur, which applies only to the slim outer position
    /// indicator.
    fn paint_motion_blur(&self, p: &egui::Painter, layout: &UiScale, center: Pos2) {
        if JOG_ROTATION_MOTION_BLUR_INTENSITY <= 0.0 || JOG_ROTATION_MOTION_BLUR_STEPS == 0 {
            return;
        }
        let omega = self.jog_angular_velocity();
        // Scale the blur by speed, with 1 rev/s as full speed, so that it shows at
        // normal interaction speeds.
        let speed01 = (omega.abs() / std::f32::consts::TAU).clamp(0.0, 1.0);
        let blur_strength = (JOG_ROTATION_MOTION_BLUR_INTENSITY * speed01).clamp(0.0, 1.0);
        if blur_strength <= 0.001 {
            return;
        }
        let base_angle = self.jog_display_angle();
        let trail_sign = omega.signum();
        let trail_span = JOG_ROTATION_MOTION_BLUR_MAX_TRAIL_RAD * blur_strength;
        for step in 1..=JOG_ROTATION_MOTION_BLUR_STEPS {
            let t = step as f32 / JOG_ROTATION_MOTION_BLUR_STEPS as f32;
            let trail_alpha = (0.9 * (1.0 - t) * blur_strength).clamp(0.0, 1.0);
            if trail_alpha <= 0.001 {
                continue;
            }
            let trail_angle = base_angle - trail_sign * (trail_span * t);
            p.add(Shape::convex_polygon(
                indicator_quad(center, trail_angle, layout),
                COL_WHITE.gamma_multiply(trail_alpha),
                Stroke::NONE,
            ));
        }
    }

    /// Draws the jog LCD stream as an axis-aligned rect scaled from c0 alone, with pixels
    /// drawn only inside the c1/c2 silhouette mesh (see
    /// [`inner_lcd_silhouette_polygon`](geometry::inner_lcd_silhouette_polygon)).
    ///
    /// Returns the on-screen rect of the logical 320×240 framebuffer when a texture was
    /// drawn, for debug overlays.
    fn paint_jog_lcd_stream_texture(
        &self,
        p: &Painter,
        center: Pos2,
        layout: &UiScale,
    ) -> Option<Rect> {
        if self.jog_screen_popped {
            return None;
        }
        if self.lcds_blanked {
            return None;
        }
        let tex_id = self.textures.jog_id?;
        let r_c0 = layout.sc(JOG_INNER_LCD_C0_RADIUS);
        let w = JOG_FB_W as f32;
        let h = JOG_FB_H as f32;
        let diag = (w * w + h * h).sqrt();
        if !diag.is_finite() || diag <= 0.0 {
            return None;
        }
        let scale = (2.0 * r_c0) / diag;
        let tex_rect = Rect::from_center_size(center, Vec2::new(w * scale, h * scale));
        if !JOG_LCD_USE_SILHOUETTE_MASK {
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            p.image(tex_id, tex_rect, uv, Color32::WHITE);
            return Some(tex_rect);
        }
        // The silhouette extends past the c0 4:3 `tex_rect` at the c1 caps. Unclipped, the fan
        // triangles there get UVs outside 0..1, and clamp-to-edge fills them with the
        // framebuffer's border pixels instead of leaving `inner_mid` black.
        let boundary = geometry::clip_polygon_to_rect(
            &geometry::inner_lcd_silhouette_polygon(center, layout),
            tex_rect,
        );
        if boundary.len() < 3 {
            return Some(tex_rect);
        }
        let mut mesh = Mesh::with_texture(tex_id);
        // Map UVs linearly from the full c0-scaled 4:3 rect. Do not clamp vertex UVs to [0,1]:
        // clamping breaks affine interpolation in the fan and stretches the image
        // horizontally. Clipping the boundary to `tex_rect` above keeps every fragment in
        // range.
        let inv_w = 1.0 / tex_rect.width();
        let inv_h = 1.0 / tex_rect.height();
        let to_uv = |pos: Pos2| {
            Pos2::new(
                (pos.x - tex_rect.left()) * inv_w,
                (pos.y - tex_rect.top()) * inv_h,
            )
        };
        let cidx = 0u32;
        mesh.vertices.push(Vertex {
            pos: center,
            uv: to_uv(center),
            color: Color32::WHITE,
        });
        for q in &boundary {
            mesh.vertices.push(Vertex {
                pos: *q,
                uv: to_uv(*q),
                color: Color32::WHITE,
            });
        }
        let n = boundary.len() as u32;
        for i in 0..n {
            mesh.add_triangle(cidx, cidx + 1 + i, cidx + 1 + (i + 1) % n);
        }
        p.add(Shape::mesh(mesh));
        Some(tex_rect)
    }
}

/// The slim position indicator at `angle`: a quad across the outer band,
/// from r_outer_1 to r_outer_2.
fn indicator_quad(center: Pos2, angle: f32, layout: &UiScale) -> Vec<Pos2> {
    let (sin_a, cos_a) = angle.sin_cos();
    let radial = Vec2::new(cos_a, sin_a);
    let tangent = Vec2::new(-sin_a, cos_a);
    let r1 = layout.sc(JOG_OUTER_1_STROKE_RADIUS);
    let r2 = layout.sc(JOG_OUTER_2_STROKE_RADIUS);
    let hw = layout.sc(JOG_POS_INDICATOR_HALF_WIDTH_REF);
    vec![
        center + radial * r1 + tangent * hw,
        center + radial * r2 + tangent * hw,
        center + radial * r2 - tangent * hw,
        center + radial * r1 - tangent * hw,
    ]
}
