//! Pointer input on the jog: drag, scroll and the slingshot on the platter,
//! and the JOG ADJUST knob.

use egui::{Color32, Pos2, Rect, Response, Sense, Stroke, Vec2};
use std::f32::consts::TAU;

use super::super::{rotary_arc_fraction, scroll_detents, CdjApp, UiScale};
use super::*;

/// Pixels of raw scroll needed to move the jog-adjust knob by one detent.
pub(super) const JOG_ADJUST_SCROLL_PX_PER_STEP: f32 = 10.0;
/// Pixels of raw vertical wheel scroll that map to one full jog revolution.
pub(super) const JOG_SCROLL_PX_PER_REV: f32 = 1280.0;
/// A sensitivity multiplier for scrolling the jog wheel. A lower value makes it
/// less sensitive; 0.25 makes it four times less sensitive.
pub(super) const JOG_SCROLL_SENSITIVITY_MULT: f32 = 0.25;

// ── Slingshot drag: a Ctrl-drag applies an impulse on release ────────────────
/// The modifier that arms slingshot mode when a drag starts. Releasing it during
/// the drag has no effect, because the mode latches on `drag_started()`.
pub(super) const JOG_SLINGSHOT_MODIFIER: egui::Modifiers = egui::Modifiers::CTRL;
/// Platter spin in rad/s per screen pixel of tangential pull. At this gain, a
/// 200 px tangential pull spins the platter at about 3 rev/s.
pub(super) const JOG_SLINGSHOT_GAIN: f32 = 0.094;
/// The largest impulse in rad/s, so that a long pull cannot spin the platter
/// too fast. It is about 9.5 rev/s, above any musically useful speed.
pub(super) const JOG_SLINGSHOT_MAX_OMEGA: f32 = 60.0;
/// Stroke width of the slingshot indicator line (reference units).
pub(super) const JOG_SLINGSHOT_LINE_STROKE_REF: f32 = 6.0;
/// Slingshot indicator line color at zero pull (weakest spin).
pub(super) const JOG_SLINGSHOT_LINE_COLOR_MIN: Color32 = Color32::from_rgb(60, 220, 80);
/// Slingshot indicator line color at full pull, which reaches `JOG_SLINGSHOT_MAX_OMEGA`.
pub(super) const JOG_SLINGSHOT_LINE_COLOR_MAX: Color32 = Color32::from_rgb(240, 60, 60);

impl CdjApp {
    /// Drives the platter from the pointer with drag, scroll and the
    /// slingshot. The platter is at `center`, its touch surface ends at
    /// `r_touch` and its grip band ends at `r_outer_1`, in screen units. Every
    /// slate calls it on every frame.
    pub(in crate::app) fn jog_interact(
        &mut self,
        ui: &mut egui::Ui,
        layout: &UiScale,
        center: Pos2,
        r_touch: f32,
        r_outer_1: f32,
    ) {
        // This must run on every frame. The interact rect covers the full grip
        // band up to r_outer_1, so one drag captures both the platter and the
        // grip ring.
        let drag_rect = Rect::from_center_size(center, Vec2::splat(r_outer_1 * 2.0));
        let dt = ui.input(|i| i.stable_dt.max(1.0e-4));
        let resp = ui.interact(drag_rect, ui.id().with("jog_touch_drag"), Sense::drag());
        if resp.drag_started() {
            self.jog_drag_start(ui, &resp, center, r_touch, r_outer_1);
        }
        if resp.dragged() {
            if self.jog_slingshot_anchor.is_some() {
                // In slingshot mode the drag motion is ignored; the release
                // reads the pointer position from resp.interact_pointer_pos().
            } else if let Some(pos) = resp.interact_pointer_pos() {
                let v = pos - center;
                // Accept a drag anywhere inside the grip band, or one already
                // in progress.
                if v.length_sq() <= r_outer_1 * r_outer_1 || self.jog_is_dragging() {
                    self.jog_drag_sample(v.y.atan2(v.x), dt);
                }
            }
        } else if let Some(anchor) = self.jog_slingshot_anchor.take() {
            self.jog_slingshot_release(ui, &resp, anchor, center, r_touch);
        } else if self.jog_is_dragging() {
            self.jog_drag_release();
            self.set_jog_grip_drag(false);
        }

        self.paint_slingshot_line(ui, layout, center);
        if resp.hovered() && !resp.dragged() {
            self.jog_scroll(ui, center, r_touch, r_outer_1, dt);
        }
    }

    /// Picks the drag mode when the drag begins: slingshot, grip band or
    /// platter.
    fn jog_drag_start(
        &mut self,
        ui: &egui::Ui,
        resp: &Response,
        center: Pos2,
        r_touch: f32,
        r_outer_1: f32,
    ) {
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

    /// Releases the slingshot. The pull is projected onto the platter tangent
    /// at the anchor, and the platter spins opposite to the pull. An anchor on
    /// the platter gives a scratch, with a short touch pulse so that the
    /// firmware registers a flick; an anchor on the grip band gives a bend,
    /// which is an impulse alone.
    fn jog_slingshot_release(
        &mut self,
        ui: &egui::Ui,
        resp: &Response,
        anchor: Pos2,
        center: Pos2,
        r_touch: f32,
    ) {
        let release_pos = resp
            .interact_pointer_pos()
            .or_else(|| ui.ctx().input(|i| i.pointer.latest_pos()))
            .unwrap_or(anchor);
        if let Some((along, radial_len)) = tangential_pull(center, anchor, release_pos) {
            let omega = (-along * JOG_SLINGSHOT_GAIN)
                .clamp(-JOG_SLINGSHOT_MAX_OMEGA, JOG_SLINGSHOT_MAX_OMEGA);
            let scratch_mode = radial_len <= r_touch;
            self.jog_apply_impulse(omega, scratch_mode);
        }
        self.set_jog_grip_drag(false);
    }

    /// Draws the slingshot line from the anchor to the pointer on a foreground
    /// layer. Its color goes from green to red with the strength the release
    /// would apply, and saturates at JOG_SLINGSHOT_MAX_OMEGA.
    fn paint_slingshot_line(&self, ui: &egui::Ui, layout: &UiScale, center: Pos2) {
        let Some(anchor) = self.jog_slingshot_anchor else {
            return;
        };
        let Some(pos) = ui.ctx().input(|i| i.pointer.latest_pos()) else {
            return;
        };
        let strength = tangential_pull(center, anchor, pos).map_or(0.0, |(along, _)| {
            (along.abs() * JOG_SLINGSHOT_GAIN / JOG_SLINGSHOT_MAX_OMEGA).clamp(0.0, 1.0)
        });
        let lerp_u8 =
            |a: u8, b: u8| -> u8 { (a as f32 + (b as f32 - a as f32) * strength).round() as u8 };
        let (lo, hi) = (JOG_SLINGSHOT_LINE_COLOR_MIN, JOG_SLINGSHOT_LINE_COLOR_MAX);
        let color = Color32::from_rgb(
            lerp_u8(lo.r(), hi.r()),
            lerp_u8(lo.g(), hi.g()),
            lerp_u8(lo.b(), hi.b()),
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

    /// Wheel scroll over the platter or the grip band turns it.
    fn jog_scroll(&mut self, ui: &egui::Ui, center: Pos2, r_touch: f32, r_outer_1: f32, dt: f32) {
        let scroll_y = crate::app::scroll::raw_delta(ui).y;
        if scroll_y == 0.0 {
            return;
        }
        let Some(pos) = ui.ctx().input(|i| i.pointer.hover_pos()) else {
            return;
        };
        let v = pos - center;
        let dist_sq = v.length_sq();
        if dist_sq <= r_outer_1 * r_outer_1 {
            let grip = dist_sq > r_touch * r_touch;
            // Scrolling turns the platter tangentially: on the left half,
            // scrolling up turns it forward; on the right half, scrolling up
            // turns it backward.
            let side_sign = if v.x < 0.0 { -1.0 } else { 1.0 };
            let delta_rad =
                side_sign * (scroll_y / JOG_SCROLL_PX_PER_REV) * TAU * JOG_SCROLL_SENSITIVITY_MULT;
            self.jog_scroll_sample(delta_rad, dt, grip);
        }
    }

    /// Handles drag and scroll on the JOG ADJUST knob. It is separate from the
    /// drawing so that it runs on every frame while the static geometry comes
    /// from [`JogStaticCache`].
    pub(super) fn paint_jog_adjust_interact(
        &mut self,
        ui: &mut egui::Ui,
        layout: &UiScale,
        jog_ref: Rect,
    ) {
        let center = layout.sp_in_rect(jog_ref, JOG_ADJUST_CENTER_U, JOG_ADJUST_CENTER_V);
        let s = |r: f32| layout.sc(r * JOG_ADJUST_SIZE_SCALE);
        let r_base = s(JOG_ADJUST_KNOB_RADIUS_REF);
        let tooth_amp = s(JOG_ADJUST_TOOTH_AMP_REF);
        let r_outer = r_base + tooth_amp;
        let tick_r = r_outer + s(JOG_ADJUST_TICK_RING_OFFSET_REF);

        let rect = Rect::from_center_size(center, Vec2::splat(tick_r * 2.0));
        let resp = ui.interact(rect, ui.id().with("jog_adjust_drag"), Sense::drag());
        let n = (JOG_ADJUST_TICK_COUNT.saturating_sub(1)).max(1) as f32;
        if resp.dragged() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let v = pos - center;
                if v.length_sq() <= tick_r * tick_r {
                    let t = rotary_arc_fraction(
                        v,
                        JOG_ADJUST_TICK_ARC_START_CW_RAD,
                        JOG_ADJUST_TICK_ARC_SPAN_RAD,
                    );
                    self.set_jog_adjust((t.clamp(0.0, 1.0) * n).round() / n);
                }
            }
        }
        if resp.hovered() {
            let scroll_y = crate::app::scroll::raw_delta(ui).y;
            if scroll_y != 0.0 {
                let step = 1.0 / n;
                let steps = scroll_detents(
                    &mut self.jog_adjust_scroll_accum,
                    scroll_y,
                    JOG_ADJUST_SCROLL_PX_PER_STEP,
                );
                for _ in 0..steps.unsigned_abs() {
                    let next = if steps > 0 {
                        (self.jog_adjust() + step).min(1.0)
                    } else {
                        (self.jog_adjust() - step).max(0.0)
                    };
                    self.set_jog_adjust(next);
                }
            }
        }
    }
}

/// Returns the length of the pull from `anchor` to `pos` along the platter's
/// tangent at `anchor`, and the anchor's distance from `center`. It returns
/// `None` when the anchor is at the centre.
fn tangential_pull(center: Pos2, anchor: Pos2, pos: Pos2) -> Option<(f32, f32)> {
    let pull = pos - anchor;
    let radial = anchor - center;
    let radial_len = radial.length();
    if radial_len > f32::EPSILON {
        let tangent = Vec2::new(-radial.y, radial.x) / radial_len;
        Some((tangent.dot(pull), radial_len))
    } else {
        None
    }
}
