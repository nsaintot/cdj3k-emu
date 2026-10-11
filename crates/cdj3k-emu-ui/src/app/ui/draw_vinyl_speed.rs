//! The VINYL SPEED ADJUST knob: a rotary pot with a scroll accumulator, drawn
//! as a gear with an indicator.
//!
//! The CDJ-3000 and CDJ-3000X slates draw the same knob from the shape
//! constants below; each supplies only the knob's position on its panel.

use egui::{Pos2, Rect, Sense, Shape, Stroke, Vec2};

use super::draw_cache::ShapeList;
use crate::app::CdjApp;

use super::{
    draw_rotary_control, rotary_arc_fraction, scroll_detents, RotaryControlSpec, RotaryGearSpec,
    RotaryIndicatorSpec, RotaryTickSpec, StrokeSpec, UiScale, COL_BTN, COL_BTN_TEXT,
};

/// Where a slate puts the knob: the section rect it sits in, and its centre
/// as fractions of that rect.
#[derive(Copy, Clone, Debug)]
pub(in crate::app) struct VinylSpeedPlacement {
    pub col: Rect,
    pub center_u: f32,
    pub center_v: f32,
}

impl VinylSpeedPlacement {
    pub fn center(&self, layout: &UiScale) -> Pos2 {
        layout.sp_in_rect(self.col, self.center_u, self.center_v)
    }
}

pub(in crate::app) const VINYL_SPEED_SIZE_SCALE: f32 = 0.56;
pub(in crate::app) const VINYL_SPEED_KNOB_RADIUS_REF: f32 = 100.0;
pub(in crate::app) const VINYL_SPEED_TOOTH_COUNT: usize = 11;
pub(in crate::app) const VINYL_SPEED_TOOTH_AMP_REF: f32 = 10.0;
pub(in crate::app) const VINYL_SPEED_TOOTH_TOP_FRAC: f32 = 0.35;
pub(in crate::app) const VINYL_SPEED_VALLEY_SEGMENTS: usize = 4;
pub(in crate::app) const VINYL_SPEED_STROKE_REF: f32 = 7.0;
pub(in crate::app) const VINYL_SPEED_INDICATOR_LENGTH_FRAC: f32 = 1.0;
pub(in crate::app) const VINYL_SPEED_INDICATOR_WIDTH_REF: f32 = 14.0;
pub(in crate::app) const VINYL_SPEED_INDICATOR_STROKE_REF: f32 = 10.0;
pub(in crate::app) const VINYL_SPEED_TICK_COUNT: usize = 11;
pub(in crate::app) const VINYL_SPEED_TICK_ARC_START_CW_RAD: f32 = 5.0 * std::f32::consts::PI / 4.0;
pub(in crate::app) const VINYL_SPEED_TICK_ARC_SPAN_RAD: f32 = 3.0 * std::f32::consts::PI / 2.0;
pub(in crate::app) const VINYL_SPEED_TICK_RING_OFFSET_REF: f32 = 40.0;
pub(in crate::app) const VINYL_SPEED_TICK_DOT_RADIUS_REF: f32 = 8.0;
pub(in crate::app) const VINYL_SPEED_TICK_CENTER_DOT_RADIUS_REF: f32 = 8.0;
pub(in crate::app) const VINYL_SPEED_TOP_LABEL_FONT_SIZE: f32 = 32.0;
/// Gap from the top of the knob to the label baseline, in reference units.
pub(in crate::app) const VINYL_SPEED_LABEL_GAP_REF: f32 = 36.0;
/// Size and stroke of the endstop shapes, in reference units.
pub(in crate::app) const VINYL_SPEED_ENDSHAPE_ARM_W: f32 = 50.0;
pub(in crate::app) const VINYL_SPEED_ENDSHAPE_H: f32 = 40.0;
pub(in crate::app) const VINYL_SPEED_ENDSHAPE_STROKE: f32 = 6.0;
/// The fraction of the arm width that is flat at each end of the ramp.
pub(in crate::app) const VINYL_SPEED_RAMP_ARM_FRAC: f32 = 0.35;
/// Horizontal inset of each endstop shape toward the center from its tick-ring x
/// position, in reference units.
pub(in crate::app) const VINYL_SPEED_ENDSHAPE_INSET_X: f32 = -80.0;
/// Radial length of the endcap lines on the tick ring, in reference units.
pub(in crate::app) const VINYL_SPEED_TICK_ENDCAP_LENGTH_REF: f32 = 50.0;
pub(in crate::app) const VINYL_SPEED_TICK_ENDCAP_STROKE_REF: f32 = 8.0;
/// Vertical gap from the tick at the arc's end to the top of the endstop shape, in
/// reference units.
pub(in crate::app) const VINYL_SPEED_ENDSHAPE_GAP_Y: f32 = 30.0;
/// Pixels of raw scroll needed to move the vinyl speed by one step out of 0..255.
pub(in crate::app) const VINYL_SCROLL_PX_PER_STEP: f32 = 5.0;

pub(in crate::app) fn draw_vinyl_speed_adj(
    app: &mut CdjApp,
    ui: &mut egui::Ui,
    p: &egui::Painter,
    layout: &UiScale,
    place: VinylSpeedPlacement,
) {
    let s = |r: f32| layout.sc(r * VINYL_SPEED_SIZE_SCALE);

    let center = place.center(layout);
    let r_base = s(VINYL_SPEED_KNOB_RADIUS_REF);
    let tooth_amp = s(VINYL_SPEED_TOOTH_AMP_REF);
    let r_outer = r_base + tooth_amp;
    let tick_ring_r = r_outer + s(VINYL_SPEED_TICK_RING_OFFSET_REF);

    vinyl_speed_interact(app, ui, center, tick_ring_r);

    let t = app.vinyl_speed as f32 / 255.0;
    let indicator_angle = VINYL_SPEED_TICK_ARC_START_CW_RAD + t * VINYL_SPEED_TICK_ARC_SPAN_RAD;

    draw_rotary_control(
        p,
        center,
        vinyl_speed_spec(&s, r_base, tooth_amp, indicator_angle),
    );

    // The CDJ-3000 and CDJ-3000X slates draw the static labels and endstop
    // brackets in their draw_right/statics.rs.
}

/// Handles drag and scroll on the knob. A drag maps the pointer angle to a
/// speed of 0 to 255, and each scroll detent moves the speed by one.
fn vinyl_speed_interact(app: &mut CdjApp, ui: &mut egui::Ui, center: Pos2, tick_ring_r: f32) {
    let interact_rect = Rect::from_center_size(center, Vec2::splat(tick_ring_r * 2.0));
    let resp = ui.interact(
        interact_rect,
        ui.id().with("vinyl_speed_drag"),
        Sense::drag(),
    );
    if resp.dragged() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let v = pos - center;
            if v.length_sq() <= tick_ring_r * tick_ring_r {
                let t = rotary_arc_fraction(
                    v,
                    VINYL_SPEED_TICK_ARC_START_CW_RAD,
                    VINYL_SPEED_TICK_ARC_SPAN_RAD,
                )
                .clamp(0.0, 1.0);
                let new_speed = (t * 255.0).round() as u8;
                if new_speed != app.vinyl_speed {
                    app.vinyl_speed = new_speed;
                    app.inject_vinyl();
                }
            }
        }
    }
    if resp.hovered() {
        let scroll_y = crate::app::scroll::raw_delta(ui).y;
        if scroll_y != 0.0 {
            let steps = scroll_detents(
                &mut app.vinyl_scroll_accum,
                scroll_y,
                VINYL_SCROLL_PX_PER_STEP,
            );
            for _ in 0..steps.unsigned_abs() {
                app.vinyl_speed = if steps > 0 {
                    app.vinyl_speed.saturating_add(1)
                } else {
                    app.vinyl_speed.saturating_sub(1)
                };
                app.inject_vinyl();
            }
        }
    }
}

/// Builds the knob's gear, indicator and tick ring, sized by `s`.
fn vinyl_speed_spec(
    s: &impl Fn(f32) -> f32,
    r_base: f32,
    tooth_amp: f32,
    indicator_angle: f32,
) -> RotaryControlSpec {
    RotaryControlSpec {
        gear: RotaryGearSpec {
            base_radius: r_base,
            tooth_count: VINYL_SPEED_TOOTH_COUNT,
            tooth_amplitude: tooth_amp,
            tooth_top_fraction: VINYL_SPEED_TOOTH_TOP_FRAC,
            rotation_rad: indicator_angle - std::f32::consts::FRAC_PI_2,
            valley_segments: VINYL_SPEED_VALLEY_SEGMENTS,
            fill: COL_BTN,
            stroke: StrokeSpec {
                width: s(VINYL_SPEED_STROKE_REF),
                color: COL_BTN_TEXT,
            },
        },
        indicator: RotaryIndicatorSpec {
            length: r_base * VINYL_SPEED_INDICATOR_LENGTH_FRAC,
            width: s(VINYL_SPEED_INDICATOR_WIDTH_REF),
            angle_cw_from_top_rad: indicator_angle,
            stroke: StrokeSpec {
                width: s(VINYL_SPEED_INDICATOR_STROKE_REF),
                color: COL_BTN_TEXT,
            },
        },
        ticks: RotaryTickSpec {
            count: VINYL_SPEED_TICK_COUNT,
            arc_start_cw_rad: VINYL_SPEED_TICK_ARC_START_CW_RAD,
            arc_span_rad: VINYL_SPEED_TICK_ARC_SPAN_RAD,
            ring_offset: s(VINYL_SPEED_TICK_RING_OFFSET_REF),
            dot_radius: s(VINYL_SPEED_TICK_DOT_RADIUS_REF),
            center_dot_radius: s(VINYL_SPEED_TICK_CENTER_DOT_RADIUS_REF),
            endcap_length: s(VINYL_SPEED_TICK_ENDCAP_LENGTH_REF),
            endcap_stroke: StrokeSpec {
                width: s(VINYL_SPEED_TICK_ENDCAP_STROKE_REF),
                color: COL_BTN_TEXT,
            },
        },
        color: COL_BTN_TEXT,
        draw_indicator: true,
        draw_ticks: true,
    }
}

/// Collects the endstop brackets at the ends of the tick arc: a stair at the
/// minimum and a ramp at the maximum.
pub(in crate::app) fn collect_vinyl_speed_endstops(
    list: &mut ShapeList,
    layout: &UiScale,
    place: VinylSpeedPlacement,
) {
    let s = |r: f32| layout.sc(r * VINYL_SPEED_SIZE_SCALE);
    let center = place.center(layout);
    let r_outer = s(VINYL_SPEED_KNOB_RADIUS_REF) + s(VINYL_SPEED_TOOTH_AMP_REF);
    let tick_ring_r = r_outer + s(VINYL_SPEED_TICK_RING_OFFSET_REF);

    let min_angle = VINYL_SPEED_TICK_ARC_START_CW_RAD;
    let max_angle = VINYL_SPEED_TICK_ARC_START_CW_RAD + VINYL_SPEED_TICK_ARC_SPAN_RAD;
    let min_tx = center.x + min_angle.sin() * tick_ring_r;
    let min_ty = center.y - min_angle.cos() * tick_ring_r;
    let max_tx = center.x + max_angle.sin() * tick_ring_r;
    let max_ty = center.y - max_angle.cos() * tick_ring_r;

    let aw = s(VINYL_SPEED_ENDSHAPE_ARM_W);
    let eh = s(VINYL_SPEED_ENDSHAPE_H);
    let egap = s(VINYL_SPEED_ENDSHAPE_GAP_Y);
    let estroke = Stroke::new(s(VINYL_SPEED_ENDSHAPE_STROKE), COL_BTN_TEXT);
    let inset = s(VINYL_SPEED_ENDSHAPE_INSET_X);

    // The minimum shape, a stair.
    let sx = min_tx + inset;
    let sy = min_ty + egap;
    list.add(Shape::line(
        vec![
            Pos2::new(sx, sy),
            Pos2::new(sx + aw, sy),
            Pos2::new(sx + aw, sy + eh),
            Pos2::new(sx + aw * 2.0, sy + eh),
        ],
        estroke,
    ));

    // The maximum shape, a ramp.
    let ramp_total = aw * 2.0;
    let ramp_arm = ramp_total * VINYL_SPEED_RAMP_ARM_FRAC;
    let rx = max_tx - ramp_total - inset;
    let ry = max_ty + egap;
    list.add(Shape::line(
        vec![
            Pos2::new(rx, ry),
            Pos2::new(rx + ramp_arm, ry),
            Pos2::new(rx + ramp_total - ramp_arm, ry + eh),
            Pos2::new(rx + ramp_total, ry + eh),
        ],
        estroke,
    ));
}
