//! Jog wheel assembly: concentric rings (inner LCD, touch platter, pitchbend grip band,
//! cosmetic outer border) centered in [`layout::JOG_REF_*`].
//!
//! Rings are drawn as concentric filled disks, from outermost to innermost, so each
//! disk naturally overwrites the interior of the previous one and produces a clean
//! annulus without relying on `circle_stroke` width behavior.

use egui::{
    emath::Rot2,
    epaint::{Mesh, TextShape, Vertex},
    Color32, FontFamily, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke, Vec2,
};
use std::f32::consts::TAU;

use crate::app::ui::{
    draw_cache::{JogCacheKey, JogStaticCache, ShapeList},
    layout, COL_BTN, COL_BTN_TEXT, COL_DARK, COL_LCD_BG, COL_SILVER, COL_WHITE,
};
use cdj3k_emu_streams::jog_stream::{JOG_FB_H, JOG_FB_W};
use cdj3k_emu_subucom::mosi_frame;

use super::{CdjApp, UiScale};

/// Reference rect for the full jog panel (same extent used by the wheel assembly).
mod geometry;
mod statics;

pub(super) const JOG_PANEL_REF: Rect = Rect::from_min_max(
    Pos2::new(layout::JOG_REF_LEFT, layout::JOG_REF_TOP),
    Pos2::new(layout::JOG_REF_RIGHT, layout::JOG_REF_BOT),
);

// --- Jog wheel radii (reference units, concentric from center outward) ---
/// Reference radius for scaling the jog **stream** framebuffer (slightly larger than
/// the physical c1 bezel so the live LCD reads a bit bigger on screen).
pub(super) const JOG_INNER_LCD_C0_RADIUS: f32 = 410.0;
pub(super) const JOG_INNER_LCD_C1_RADIUS: f32 = 340.0;
/// Inner LCD silhouette: rectangle half-width as a fraction of c1 (JOG_INNER_LCD_C1_RADIUS).
/// The rectangle height is derived to keep corners on c1.
pub(super) const JOG_INNER_LCD_RECT_HALF_WIDTH_FRAC: f32 = 0.4;
/// Inner LCD silhouette: c2 radius as a fraction of c1 (JOG_INNER_LCD_C1_RADIUS).
pub(super) const JOG_INNER_LCD_C2_RADIUS_FRAC: f32 = 0.7;
/// Label ring: c3 radius as a fraction of c1 (must be > [`JOG_INNER_LCD_C2_RADIUS_FRAC`]).
pub(super) const JOG_INNER_LCD_C3_RADIUS_FRAC: f32 = 0.81;
/// Font size for inner LCD curved labels (reference units).
pub(super) const JOG_INNER_LCD_LABEL_FONT_REF: f32 = 27.0;
/// Stroke width for c3 separator arcs (reference units).
pub(super) const JOG_INNER_LCD_C3_ARC_STROKE_REF: f32 = 5.0;
/// Polyline samples per c3 arc span.
pub(super) const JOG_INNER_LCD_C3_ARC_SEGMENTS: usize = 48;
/// Stub “cutout” outline offset (reference units) for dark rim behind LCD-colored glyphs.
pub(super) const JOG_INNER_LCD_CUTOUT_OUTLINE_REF: f32 = 1.1;
/// Small overlap added at line/arc joints so there is no 1px seam.
pub(super) const JOG_INNER_LCD_JOIN_OVERLAP_REF: f32 = 2.0;
/// Samples per circular arc when building the inner-LCD silhouette mesh.
pub(super) const JOG_INNER_LCD_MESH_ARC_SEGMENTS: usize = 48;
/// When `false`, draw the full jog LCD rectangle with no silhouette clip (debug / layout check).
pub(super) const JOG_LCD_USE_SILHOUETTE_MASK: bool = true;
/// When `true`, add π to glyph rotation for bottom-arc labels (SYNC / MASTER and static “BEAT SYNC”) so they read upright.
pub(super) const JOG_INNER_LCD_INVERT_BOTTOM_CORNER_LABELS: bool = true;
pub(super) const JOG_TOUCH_RADIUS: f32 = 835.0;
pub(super) const JOG_OUTER_1_STROKE_RADIUS: f32 = 1000.0;
pub(super) const JOG_OUTER_2_STROKE_RADIUS: f32 = 1060.0;
pub(super) const JOG_OUTER_2_STROKE_GAP: f32 = 0.0;

// --- Pitchbend band - ring of grip ovals ---
/// Number of grip ovals evenly spaced around the pitchbend band.
pub(super) const JOG_GRIP_OVAL_COUNT: usize = 24;
/// Oval radial half-length as a fraction of the pitchbend band width.
pub(super) const JOG_GRIP_OVAL_RADIAL_FRAC: f32 = 0.32;
/// Oval tangential half-width as a fraction of the per-slot arc length.
pub(super) const JOG_GRIP_OVAL_TANGENT_FRAC: f32 = 0.32;
/// Polygon resolution used to approximate each oval.
pub(super) const JOG_GRIP_POLYGON_SEGMENTS: usize = 32;
/// Outline stroke width for each oval (reference units).
pub(super) const JOG_GRIP_OVAL_STROKE_REF: f32 = 6.0;

// --- Position indicator on r_outer_2 ring ---
/// Half-width (tangential) of the slim white position indicator rectangle (reference units).
pub(super) const JOG_POS_INDICATOR_HALF_WIDTH_REF: f32 = 8.0;

// --- 3-dot decoration between consecutive grip ovals ---
/// Number of decorative dots between each pair of consecutive ovals.
pub(super) const JOG_GRIP_DOT_COUNT: usize = 3;
/// Filled dot radius (reference units).
pub(super) const JOG_GRIP_DOT_RADIUS_REF: f32 = 10.0;
/// Total radial spread of the dot cluster (outer-most to inner-most), reference units.
pub(super) const JOG_GRIP_DOT_RADIAL_SPREAD_REF: f32 = 80.0;
/// Total tangential shift of the dot cluster (outer -> inner = left -> right), reference units.
pub(super) const JOG_GRIP_DOT_TANGENT_SPREAD_REF: f32 = -32.0;

// --- Touch platter edge - double border ---
/// Inner stroke width sitting exactly on [`JOG_TOUCH_RADIUS`] (reference units).
pub(super) const JOG_TOUCH_BORDER_INNER_WIDTH_REF: f32 = 5.0;
/// Outer stroke width, offset outward by [`JOG_TOUCH_BORDER_GAP_REF`] (reference units).
pub(super) const JOG_TOUCH_BORDER_OUTER_WIDTH_REF: f32 = 3.0;
/// Clear distance between inner and outer strokes (reference units).
pub(super) const JOG_TOUCH_BORDER_GAP_REF: f32 = 6.0;

// --- JOG ADJUST rotary knob (top-right of jog panel) ---
/// Global size multiplier for the entire rotary control (knob, indicator, tick
/// ring, and labels). `1.0` = the default sizing in reference units; scale up
/// or down to resize the whole assembly uniformly without touching individual
/// dimensions. Counts, angles, and fractions (already proportional) are not
/// affected.
pub(super) const JOG_ADJUST_SIZE_SCALE: f32 = 0.5;
/// Knob center fractional position within [`JOG_PANEL_REF`] (u, v in [0, 1]).
pub(super) const JOG_ADJUST_CENTER_U: f32 = 0.91;
pub(super) const JOG_ADJUST_CENTER_V: f32 = 0.11;
/// Valley (tooth-bottom) radius of the gear knob (reference units).
pub(super) const JOG_ADJUST_KNOB_RADIUS_REF: f32 = 105.0;
/// Number of gear teeth around the knob edge.
pub(super) const JOG_ADJUST_TOOTH_COUNT: usize = 12;
/// Radial amplitude from valley to tooth top (reference units).
pub(super) const JOG_ADJUST_TOOTH_AMP_REF: f32 = 14.0;
/// Fraction of each angular slot occupied by the flat tooth top; the remainder is
/// the cosine-dip valley between teeth. Lower = narrower teeth, wider valleys.
pub(super) const JOG_ADJUST_TOOTH_TOP_FRAC: f32 = 0.35;
/// sample exactly on the valley bottom.
pub(super) const JOG_ADJUST_VALLEY_SEGMENTS: usize = 4;
/// Outline stroke width of the gear (reference units).
pub(super) const JOG_ADJUST_STROKE_REF: f32 = 7.0;
/// Fill color of the gear body (area enclosed by the outline).
pub(super) const JOG_ADJUST_FILL_COLOR: Color32 = COL_BTN;
/// Length of the indicator pill as a fraction of the knob's base radius.
pub(super) const JOG_ADJUST_INDICATOR_LENGTH_FRAC: f32 = 1.2;
/// Width of the indicator pill (reference units).
pub(super) const JOG_ADJUST_INDICATOR_WIDTH_REF: f32 = 14.0;
/// Outline stroke width of the indicator pill (reference units).
pub(super) const JOG_ADJUST_INDICATOR_STROKE_REF: f32 = 10.0;
/// Total number of ticks on the detent ring (including the two min/max endcap lines).
pub(super) const JOG_ADJUST_TICK_COUNT: usize = 13;
/// Min-tick angle, in radians measured clockwise from the top.
/// 225° = 5π/4 places "LIGHT" at the bottom-left (7:30 position).
pub(super) const JOG_ADJUST_TICK_ARC_START_CW_RAD: f32 = 5.0 * std::f32::consts::PI / 4.0;
/// Clockwise span from min to max tick. 270° = 3π/2 sweeps over the top.
pub(super) const JOG_ADJUST_TICK_ARC_SPAN_RAD: f32 = 3.0 * std::f32::consts::PI / 2.0;
/// Pixels of raw scroll needed to advance one jog-adjust detent step. Tune this.
pub(super) const JOG_ADJUST_SCROLL_PX_PER_STEP: f32 = 10.0;
/// Pixels of raw vertical wheel scroll that map to one full jog revolution.
pub(super) const JOG_SCROLL_PX_PER_REV: f32 = 1280.0;
/// Extra sensitivity multiplier for jog wheel scroll control.
/// Lower values reduce response (e.g. 0.25 = 4x less sensitive).
pub(super) const JOG_SCROLL_SENSITIVITY_MULT: f32 = 0.25;

// ── Slingshot drag (Ctrl + mousedown + drag → impulse on release) ────────────
/// Modifier that arms slingshot mode at drag start. Held-then-released during
/// a drag is fine — the mode latches on `drag_started()`.
pub(super) const JOG_SLINGSHOT_MODIFIER: egui::Modifiers = egui::Modifiers::CTRL;
/// rad/s of platter spin per screen-pixel of tangential pull. Tune to taste.
/// A 200 px purely-tangential pull at this gain → ~3 rev/s spin.
pub(super) const JOG_SLINGSHOT_GAIN: f32 = 0.094;
/// Hard cap on impulse magnitude (rad/s) so a runaway pull can't fire absurd
/// spin. ~4.8 rev/s — well past anything musically useful.
pub(super) const JOG_SLINGSHOT_MAX_OMEGA: f32 = 60.0;
/// Stroke width of the slingshot indicator line (reference units).
pub(super) const JOG_SLINGSHOT_LINE_STROKE_REF: f32 = 6.0;
/// Slingshot indicator line color at zero pull (weakest spin).
pub(super) const JOG_SLINGSHOT_LINE_COLOR_MIN: Color32 = Color32::from_rgb(60, 220, 80);
/// Slingshot indicator line color at saturated pull (clamped to `JOG_SLINGSHOT_MAX_OMEGA`).
pub(super) const JOG_SLINGSHOT_LINE_COLOR_MAX: Color32 = Color32::from_rgb(240, 60, 60);
// ── Jog ring arc lights ───────────────────────────────────────────────────────
/// Half-angle of each glow arc (degrees from its center).
pub(super) const JOG_RING_LIGHT_HALF_ANGLE_DEG: f32 = 25.0;
/// Polyline sample count per arc light.
pub(super) const JOG_RING_LIGHT_ARC_SEGMENTS: usize = 15;
/// Core stroke width at the touch radius (reference units).
pub(super) const JOG_RING_LIGHT_CORE_STROKE_REF: f32 = 10.0;
/// Number of outward glow layers.
pub(super) const JOG_RING_LIGHT_GLOW_LAYERS: usize = 25;
/// Total radial spread of the outer glow (reference units).
pub(super) const JOG_RING_LIGHT_GLOW_SPREAD_REF: f32 = 30.0;
/// Total radial spread of the inner (platter) glow (reference units).
pub(super) const JOG_RING_LIGHT_INNER_SPREAD_REF: f32 = 100.0;

/// Rotation motion-blur intensity (0 = disabled). Increase for a longer/stronger trail.
pub(super) const JOG_ROTATION_MOTION_BLUR_INTENSITY: f32 = 3.0;
/// Number of trailing copies used to fake rotational blur.
pub(super) const JOG_ROTATION_MOTION_BLUR_STEPS: usize = 64;
/// Maximum angular trail length at peak speed (radians).
pub(super) const JOG_ROTATION_MOTION_BLUR_MAX_TRAIL_RAD: f32 = 1.0;
/// Radial distance of the tick ring from the knob's outer edge (reference units).
pub(super) const JOG_ADJUST_TICK_RING_OFFSET_REF: f32 = 40.0;
/// Radius of the regular tick dots (reference units).
pub(super) const JOG_ADJUST_TICK_DOT_RADIUS_REF: f32 = 6.0;
/// Radius of the center tick dot, slightly larger than the regular dots (reference units).
pub(super) const JOG_ADJUST_TICK_CENTER_DOT_RADIUS_REF: f32 = 9.0;
/// Radial length of the min/max endcap lines (reference units).
pub(super) const JOG_ADJUST_TICK_ENDCAP_LENGTH_REF: f32 = 50.0;
/// Stroke width of the min/max endcap lines (reference units).
pub(super) const JOG_ADJUST_TICK_ENDCAP_STROKE_REF: f32 = 10.0;
/// Font size for the "JOG ADJUST" title label.
pub(super) const JOG_ADJUST_TOP_LABEL_FONT_SIZE: f32 = 66.0;
/// Distance from the top of the knob to the title baseline (reference units).
pub(super) const JOG_ADJUST_TOP_LABEL_GAP_REF: f32 = 99.0;
/// Font size for the "LIGHT" / "HEAVY" side labels.
pub(super) const JOG_ADJUST_SIDE_LABEL_FONT_SIZE: f32 = 40.0;
/// Distance from the bottom of the knob to the side labels (reference units).
pub(super) const JOG_ADJUST_SIDE_LABEL_GAP_REF: f32 = 28.0;
/// Horizontal offset from the knob's vertical axis to each side label (reference units).
pub(super) const JOG_ADJUST_SIDE_LABEL_X_OFFSET_REF: f32 = 140.0;

impl CdjApp {
    pub(super) fn draw_jog_wheel_section(
        &mut self,
        ui: &mut egui::Ui,
        p: &egui::Painter,
        layout: &UiScale,
    ) {
        puffin::profile_function!();
        let jog_panel = Rect::from_min_max(
            layout.sp(layout::JOG_REF_LEFT, layout::JOG_REF_TOP),
            layout.sp(layout::JOG_REF_RIGHT, layout::JOG_REF_BOT),
        );
        let center = jog_panel.center();
        let r_touch = layout.sc(JOG_TOUCH_RADIUS);
        let r_outer_1 = layout.sc(JOG_OUTER_1_STROKE_RADIUS);

        // ── Interaction (must run every frame) ────────────────────────────────
        // Interact rect covers the full grip band (up to r_outer_1) so both
        // the center platter and the grip ring are captured by a single drag.
        let drag_rect = Rect::from_center_size(center, Vec2::splat(r_outer_1 * 2.0));
        let dt = ui.input(|i| i.stable_dt.max(1.0e-4));
        let resp = ui.interact(drag_rect, ui.id().with("jog_touch_drag"), Sense::drag());
        if resp.drag_started() {
            // Determine zone at the moment the drag begins.
            if let Some(pos) = resp.interact_pointer_pos() {
                let dist_sq = (pos - center).length_sq();
                let slingshot_armed =
                    ui.input(|i| i.modifiers.matches_logically(JOG_SLINGSHOT_MODIFIER));
                if slingshot_armed && dist_sq <= r_outer_1 * r_outer_1 {
                    self.jog_slingshot_anchor = Some(pos);
                    self.set_jog_grip_drag(true);
                } else {
                    self.set_jog_grip_drag(dist_sq > r_touch * r_touch);
                }
            }
        }
        if resp.dragged() {
            if self.jog_slingshot_anchor.is_some() {
                // Slingshot mode: swallow drag motion. Release-time math
                // reads pointer position via resp.interact_pointer_pos().
            } else if let Some(pos) = resp.interact_pointer_pos() {
                let v = pos - center;
                // Accept drag anywhere inside the grip band (or if already dragging).
                if v.length_sq() <= r_outer_1 * r_outer_1 || self.jog_is_dragging() {
                    self.jog_drag_sample(v.y.atan2(v.x), dt);
                }
            }
        } else if let Some(anchor) = self.jog_slingshot_anchor.take() {
            // Slingshot release: project pull vector onto tangent at anchor.
            // Real-slingshot polarity: spin direction opposite the pull.
            // Anchor zone selects the release character: scratch (one-tick
            // touch pulse so firmware logs a flick) vs bend (pure impulse).
            let release_pos = resp
                .interact_pointer_pos()
                .or_else(|| ui.ctx().input(|i| i.pointer.latest_pos()))
                .unwrap_or(anchor);
            let pull = release_pos - anchor;
            let radial = anchor - center;
            let radial_len = radial.length();
            if radial_len > f32::EPSILON {
                let tangent = Vec2::new(-radial.y, radial.x) / radial_len;
                let omega = (-(tangent.dot(pull)) * JOG_SLINGSHOT_GAIN)
                    .clamp(-JOG_SLINGSHOT_MAX_OMEGA, JOG_SLINGSHOT_MAX_OMEGA);
                let scratch_mode = radial_len <= r_touch;
                self.jog_apply_impulse(omega, scratch_mode);
            }
            self.set_jog_grip_drag(false);
        } else if self.jog_is_dragging() {
            self.jog_drag_release();
            self.set_jog_grip_drag(false);
        }

        // Slingshot visual: line from anchor → current pointer, painted on a
        // foreground layer. Color lerps green → red with the same magnitude the release would impart,
        // giving a live strength indicator that saturates at JOG_SLINGSHOT_MAX_OMEGA.
        if let Some(anchor) = self.jog_slingshot_anchor {
            if let Some(pos) = ui.ctx().input(|i| i.pointer.latest_pos()) {
                let pull = pos - anchor;
                let radial = anchor - center;
                let radial_len = radial.length();
                let strength = if radial_len > f32::EPSILON {
                    let tangent = Vec2::new(-radial.y, radial.x) / radial_len;
                    (tangent.dot(pull).abs() * JOG_SLINGSHOT_GAIN / JOG_SLINGSHOT_MAX_OMEGA)
                        .clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let lerp_u8 = |a: u8, b: u8| -> u8 {
                    (a as f32 + (b as f32 - a as f32) * strength).round() as u8
                };
                let color = Color32::from_rgb(
                    lerp_u8(
                        JOG_SLINGSHOT_LINE_COLOR_MIN.r(),
                        JOG_SLINGSHOT_LINE_COLOR_MAX.r(),
                    ),
                    lerp_u8(
                        JOG_SLINGSHOT_LINE_COLOR_MIN.g(),
                        JOG_SLINGSHOT_LINE_COLOR_MAX.g(),
                    ),
                    lerp_u8(
                        JOG_SLINGSHOT_LINE_COLOR_MIN.b(),
                        JOG_SLINGSHOT_LINE_COLOR_MAX.b(),
                    ),
                );
                let fg = ui.ctx().layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("jog_slingshot_overlay"),
                ));
                fg.line_segment(
                    [anchor, pos],
                    Stroke::new(layout.sc(JOG_SLINGSHOT_LINE_STROKE_REF), color),
                );
            }
        }
        if resp.hovered() && !resp.dragged() {
            let scroll_y = ui.input(|i| i.raw_scroll_delta.y);
            if scroll_y != 0.0 {
                if let Some(pos) = ui.ctx().input(|i| i.pointer.hover_pos()) {
                    let v = pos - center;
                    let dist_sq = v.length_sq();
                    if dist_sq <= r_outer_1 * r_outer_1 {
                        let grip = dist_sq > r_touch * r_touch;
                        // Tangential scroll mapping:
                        // - Left half:  scroll up => forward, scroll down => backward
                        // - Right half: scroll up => backward, scroll down => forward
                        let side_sign = if v.x < 0.0 { -1.0 } else { 1.0 };
                        let delta_rad = side_sign
                            * (scroll_y / JOG_SCROLL_PX_PER_REV)
                            * TAU
                            * JOG_SCROLL_SENSITIVITY_MULT;
                        self.jog_scroll_sample(delta_rad, dt, grip);
                    }
                }
            }
        }
        // JOG ADJUST drag interaction (see paint_jog_adjust_interact below).
        self.paint_jog_adjust_interact(ui, layout);

        // ── Static cache: rebuild when layout or jog_adjust changes ──────────
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
            .map_or(true, |c| c.key != cache_key)
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
            );
            self.jog_static_cache = Some(JogStaticCache {
                key: cache_key,
                outer: outer.into_shapes(),
                inner_mid: inner_mid.into_shapes(),
                inner_over: inner_over.into_shapes(),
            });
        }

        // ── Render: static outer → dynamic grip → static inner ───────────────
        let cache = self.jog_static_cache.as_ref().unwrap();
        self.frame_shape_count += cache.outer.len() as u64;
        p.extend(cache.outer.clone());

        // Rotation motion blur: applied only to the slim outer position indicator.
        if JOG_ROTATION_MOTION_BLUR_INTENSITY > 0.0 && JOG_ROTATION_MOTION_BLUR_STEPS > 0 {
            let omega = self.jog_angular_velocity();
            // Blur should become visible around "normal" interaction speeds (~1 rev/s),
            // not only near absolute max encoder speed.
            let speed01 = (omega.abs() / std::f32::consts::TAU).clamp(0.0, 1.0);
            let blur_strength = (JOG_ROTATION_MOTION_BLUR_INTENSITY * speed01).clamp(0.0, 1.0);
            if blur_strength > 0.001 {
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
                    let (sin_a, cos_a) = trail_angle.sin_cos();
                    let radial = Vec2::new(cos_a, sin_a);
                    let tangent = Vec2::new(-sin_a, cos_a);
                    let r1 = layout.sc(JOG_OUTER_1_STROKE_RADIUS);
                    let r2 = layout.sc(JOG_OUTER_2_STROKE_RADIUS);
                    let hw = layout.sc(JOG_POS_INDICATOR_HALF_WIDTH_REF);
                    let pts = vec![
                        center + radial * r1 + tangent * hw,
                        center + radial * r2 + tangent * hw,
                        center + radial * r2 - tangent * hw,
                        center + radial * r1 - tangent * hw,
                    ];
                    p.add(Shape::convex_polygon(
                        pts,
                        COL_WHITE.gamma_multiply(trail_alpha),
                        Stroke::NONE,
                    ));
                }
            }
        }

        // Draw pitchbend grip - cached by layout + quantized rotation angle.
        // Quantize to 4096 steps/rev (~0.088° resolution): zero rebuilds when jog is idle.
        {
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
            let grip_shapes = self.grip_cache.get_or_build(key, |out| {
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

        self.frame_shape_count += cache.inner_mid.len() as u64;
        p.extend(cache.inner_mid.clone());
        if let Some(r) = self.paint_jog_lcd_stream_texture(p, center, layout) {
            self.bloom_excludes.push(r);
        }
        self.frame_shape_count += cache.inner_over.len() as u64;
        p.extend(cache.inner_over.clone());

        // Pure-static overlays (labels, fixed circles) - rebuilt only on resize.
        let (ox, oy, scale) = layout.cache_key();
        let ctx = ui.ctx().clone();
        let ppp = ui.ctx().pixels_per_point();
        let jog_statics = self
            .jog_statics_cache
            .get_or_build(ox, oy, scale, ppp, |list| {
                statics::collect_jog_statics(list, &ctx, layout, center);
            });
        self.frame_shape_count += jog_statics.len() as u64;
        p.extend(jog_statics.iter().cloned());

        self.paint_jog_inner_lcd_corner_labels(
            p,
            ui.ctx(),
            center,
            layout,
            self.jog_corner_label_colors,
        );

        // Jog ring arc lights - cached per LED state
        {
            let brt = self.led_state.frame[mosi_frame::LED_JOG_BRT_BYTE] & 0x0c;
            let white_alpha: f32 = match brt {
                v if v == mosi_frame::LED_JOG_BRT_2 => 1.0,
                v if v == mosi_frame::LED_JOG_BRT_1 => 0.6,
                0 => 0.0,
                _ => 0.7,
            };
            let red_on =
                self.led_state.frame[mosi_frame::LED_JOG_RED.0] & mosi_frame::LED_JOG_RED.1 != 0;
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
                let shapes = self.jog_ring_lights_cache.get_or_build(key, |out| {
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

        // Position indicator: slim white rectangle on r_outer_2, rotates with jog.
        {
            let angle = self.jog_display_angle();
            let (sin_a, cos_a) = angle.sin_cos();
            let radial = Vec2::new(cos_a, sin_a);
            let tangent = Vec2::new(-sin_a, cos_a);
            let r1 = layout.sc(JOG_OUTER_1_STROKE_RADIUS);
            let r2 = layout.sc(JOG_OUTER_2_STROKE_RADIUS);
            let hw = layout.sc(JOG_POS_INDICATOR_HALF_WIDTH_REF);
            let pts = vec![
                center + radial * r1 + tangent * hw,
                center + radial * r2 + tangent * hw,
                center + radial * r2 - tangent * hw,
                center + radial * r1 - tangent * hw,
            ];
            p.add(Shape::convex_polygon(pts, COL_WHITE, Stroke::NONE));
        }
    }

    /// Jog LCD stream: axis-aligned rect scaled from **c0** only; pixels drawn only inside the
    /// c1/c2 silhouette mesh (see [`inner_lcd_silhouette_polygon`]).
    ///
    /// Returns the on-screen rect mapping the logical 320×240 framebuffer when a texture was
    /// drawn (for debug overlays).
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
        let Some(tex_id) = self.jog_tex_id else {
            return None;
        };
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
        // Silhouette bulges past the c0 4:3 `tex_rect` at the c1 caps. Without clipping, fan
        // triangles cover those bezel pixels with UV < 0 or > 1; clamp-to-edge then samples the
        // framebuffer border (often solid LCD color) instead of leaving `inner_mid` black.
        let boundary = geometry::clip_polygon_to_rect(
            &geometry::inner_lcd_silhouette_polygon(center, layout),
            tex_rect,
        );
        if boundary.len() < 3 {
            return Some(tex_rect);
        }
        let mut mesh = Mesh::with_texture(tex_id);
        // Linear map from the full c0-scaled 4:3 rect - do **not** clamp vertex UVs to [0,1]:
        // clamping breaks affine interpolation in the fan (horizontal stretching). Clipping the
        // boundary to `tex_rect` above keeps every fragment in-range instead.
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

    /// SLIP / VINYL / SYNC / MASTER - colors from last jog frame corner samples (above chrome).
    fn paint_jog_inner_lcd_corner_labels(
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
        let shapes = self.jog_corner_labels_cache.get_or_build(key, |out| {
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
    geometry::inner_lcd_push_curved_label(
        out,
        ctx,
        center,
        c3,
        315.0,
        "SLIP",
        label_font.clone(),
        colors[0],
        true,
        cutout_outline_px,
        false,
    );
    geometry::inner_lcd_push_curved_label(
        out,
        ctx,
        center,
        c3,
        45.0,
        "VINYL",
        label_font.clone(),
        colors[1],
        true,
        cutout_outline_px,
        false,
    );
    geometry::inner_lcd_push_curved_label(
        out,
        ctx,
        center,
        c3,
        136.0,
        "MASTER",
        label_font.clone(),
        colors[3],
        true,
        cutout_outline_px,
        JOG_INNER_LCD_INVERT_BOTTOM_CORNER_LABELS,
    );
    geometry::inner_lcd_push_curved_label(
        out,
        ctx,
        center,
        c3,
        225.0,
        "SYNC",
        label_font,
        colors[2],
        true,
        cutout_outline_px,
        JOG_INNER_LCD_INVERT_BOTTOM_CORNER_LABELS,
    );
}

/// Paints the pitchbend band grip pattern: `JOG_GRIP_OVAL_COUNT` outlined ovals
/// evenly spaced on the band mean radius, with `JOG_GRIP_DOT_COUNT` dots stepping
/// diagonally (outer → inner == left → right) between each consecutive pair.
///
/// `rotation_rad` is added to every element's base angle so the whole pattern
/// rotates as a rigid ring. Positive values rotate the pattern clockwise on
/// screen (matching `jog_angle`'s clockwise-from-top convention, since screen
/// y points down and `angle = i·slot` is measured via `(cos, sin)` in screen
/// space).
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

impl CdjApp {
    /// JOG ADJUST drag interaction - separated from drawing so it can run every
    /// frame while the static geometry is served from [`JogStaticCache`].
    fn paint_jog_adjust_interact(&mut self, ui: &mut egui::Ui, layout: &UiScale) {
        let center = layout.sp_in_rect(JOG_PANEL_REF, JOG_ADJUST_CENTER_U, JOG_ADJUST_CENTER_V);
        let s = |r: f32| layout.sc(r * JOG_ADJUST_SIZE_SCALE);
        let r_base = s(JOG_ADJUST_KNOB_RADIUS_REF);
        let tooth_amp = s(JOG_ADJUST_TOOTH_AMP_REF);
        let r_outer = r_base + tooth_amp;
        let tick_r = r_outer + s(JOG_ADJUST_TICK_RING_OFFSET_REF);

        let rect = Rect::from_center_size(center, Vec2::splat(tick_r * 2.0));
        let resp = ui.interact(rect, ui.id().with("jog_adjust_drag"), Sense::drag());
        if resp.dragged() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let v = pos - center;
                if v.length_sq() <= tick_r * tick_r {
                    let ang_cw_from_top =
                        (v.y.atan2(v.x) + std::f32::consts::FRAC_PI_2).rem_euclid(TAU);
                    let mid = (JOG_ADJUST_TICK_ARC_START_CW_RAD
                        + JOG_ADJUST_TICK_ARC_SPAN_RAD * 0.5)
                        .rem_euclid(TAU);
                    let mut rel = ang_cw_from_top - mid;
                    if rel > std::f32::consts::PI {
                        rel -= TAU;
                    } else if rel < -std::f32::consts::PI {
                        rel += TAU;
                    }
                    let t = 0.5 + rel / JOG_ADJUST_TICK_ARC_SPAN_RAD;
                    let n = (JOG_ADJUST_TICK_COUNT.saturating_sub(1)).max(1) as f32;
                    self.set_jog_adjust((t.clamp(0.0, 1.0) * n).round() / n);
                }
            }
        }
        if resp.hovered() {
            let scroll_y = ui.input(|i| i.raw_scroll_delta.y);
            if scroll_y != 0.0 {
                self.jog_adjust_scroll_accum += scroll_y;
                let n = (JOG_ADJUST_TICK_COUNT.saturating_sub(1)).max(1) as f32;
                let step = 1.0 / n;
                while self.jog_adjust_scroll_accum >= JOG_ADJUST_SCROLL_PX_PER_STEP {
                    self.jog_adjust_scroll_accum -= JOG_ADJUST_SCROLL_PX_PER_STEP;
                    self.set_jog_adjust((self.jog_adjust() + step).min(1.0));
                }
                while self.jog_adjust_scroll_accum <= -JOG_ADJUST_SCROLL_PX_PER_STEP {
                    self.jog_adjust_scroll_accum += JOG_ADJUST_SCROLL_PX_PER_STEP;
                    self.set_jog_adjust((self.jog_adjust() - step).max(0.0));
                }
            }
        }
    }
}

// ── Static shape builders ─────────────────────────────────────────────────────

/// Pure-static jog overlays (labels, fixed-color circles) that depend only on layout.
/// Lives in [`CdjApp::jog_statics_cache`] - rebuilt only on window resize.
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

pub(super) fn build_jog_inner_lcd_background(
    out: &mut ShapeList,
    center: egui::Pos2,
    layout: &UiScale,
) {
    let r_inner_lcd = layout.sc(JOG_INNER_LCD_C1_RADIUS);
    out.circle_filled(center, r_inner_lcd, COL_LCD_BG);
}

pub(super) fn build_jog_inner_lcd_foreground(
    out: &mut ShapeList,
    ctx: &egui::Context,
    center: egui::Pos2,
    layout: &UiScale,
) {
    let r_inner_lcd = layout.sc(JOG_INNER_LCD_C1_RADIUS);
    let stroke = Stroke::new(layout.sc(7.0), COL_BTN);
    let join_overlap = layout.sc(JOG_INNER_LCD_JOIN_OVERLAP_REF);

    // c1: already-known inner LCD cutout circle.
    let c1 = r_inner_lcd;
    // Rectangle centered at O with corners on c1 (diagonals cross at O).
    let mut rect_half_w = c1 * JOG_INNER_LCD_RECT_HALF_WIDTH_FRAC;
    let mut rect_half_h = (c1 * c1 - rect_half_w * rect_half_w).sqrt();
    // Keep the rectangle "length" horizontal (major axis on X).
    if rect_half_h > rect_half_w {
        std::mem::swap(&mut rect_half_w, &mut rect_half_h);
    }
    // c2: smaller concentric circle used for side bulges.
    let c2 = c1 * JOG_INNER_LCD_C2_RADIUS_FRAC;

    let line_segments = 256usize;
    let mut draw_clipped_circle_arcs = |radius: f32, keep: &dyn Fn(f32, f32) -> bool| {
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
    };

    // c1 arcs between the rectangle side-width points (left + right side arcs).
    draw_clipped_circle_arcs(c1, &|x, _y| x.abs() >= rect_half_w - 1.0e-3);
    // c2 arcs outside the rectangle (left/right + corner bridge arcs).
    draw_clipped_circle_arcs(c2, &|x, y| {
        x.abs() >= rect_half_w - 1.0e-3 || y.abs() >= rect_half_h - 1.0e-3
    });

    // Rectangle segments between c1 and c2 intersections.
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

    // c3: label ring - separator arcs between adjacent labels in each half.
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

    // Top group - SLIP / VINYL painted dynamically over the stream (see `paint_jog_inner_lcd_corner_labels`).
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

    // Bottom group - SYNC / MASTER painted dynamically.
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
