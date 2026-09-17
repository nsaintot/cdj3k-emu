//! VINYL SPEED ADJUST knob - rotary pot with scroll accumulator + visual
//! gear/indicator.
//!
//! One knob, drawn for every player: the shape constants below are the
//! control itself, and a slate supplies only where on its panel it sits.

use egui::{Pos2, Rect, Sense, Vec2};

use crate::app::CdjApp;

use super::{
    draw_rotary_control, RotaryControlSpec, RotaryGearSpec, RotaryIndicatorSpec, RotaryTickSpec,
    StrokeSpec, UiScale, COL_BTN, COL_BTN_TEXT,
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
/// Gap from knob top to the label baseline (ref units).
pub(in crate::app) const VINYL_SPEED_LABEL_GAP_REF: f32 = 36.0;
/// Endstop shape dimensions and stroke (ref units).
pub(in crate::app) const VINYL_SPEED_ENDSHAPE_ARM_W: f32 = 50.0;
pub(in crate::app) const VINYL_SPEED_ENDSHAPE_H: f32 = 40.0;
pub(in crate::app) const VINYL_SPEED_ENDSHAPE_STROKE: f32 = 6.0;
/// Fraction of arm width used as flat horizontals at each end of the ramp.
pub(in crate::app) const VINYL_SPEED_RAMP_ARM_FRAC: f32 = 0.35;
/// Horizontal inset of each shape toward center from its tick-ring x position (ref units).
pub(in crate::app) const VINYL_SPEED_ENDSHAPE_INSET_X: f32 = -80.0;
/// Radial length of the min/max endcap lines on the tick ring (ref units).
pub(in crate::app) const VINYL_SPEED_TICK_ENDCAP_LENGTH_REF: f32 = 50.0;
pub(in crate::app) const VINYL_SPEED_TICK_ENDCAP_STROKE_REF: f32 = 8.0;
/// Vertical gap from the arc endpoint tick dot to the shape top (ref units).
pub(in crate::app) const VINYL_SPEED_ENDSHAPE_GAP_Y: f32 = 30.0;
/// Pixels of raw scroll needed to advance one vinyl-speed step (0..255).
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

    // Drag + scroll interaction - pointer angle snapped to 11 detent positions.
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
                let ang = (v.y.atan2(v.x) + std::f32::consts::FRAC_PI_2)
                    .rem_euclid(std::f32::consts::TAU);
                let mid = (VINYL_SPEED_TICK_ARC_START_CW_RAD + VINYL_SPEED_TICK_ARC_SPAN_RAD * 0.5)
                    .rem_euclid(std::f32::consts::TAU);
                let mut rel = ang - mid;
                if rel > std::f32::consts::PI {
                    rel -= std::f32::consts::TAU;
                } else if rel < -std::f32::consts::PI {
                    rel += std::f32::consts::TAU;
                }
                let t = (0.5 + rel / VINYL_SPEED_TICK_ARC_SPAN_RAD).clamp(0.0, 1.0);
                let new_speed = (t * 255.0).round() as u8;
                if new_speed != app.vinyl_speed {
                    app.vinyl_speed = new_speed;
                    app.inject_vinyl();
                }
            }
        }
    }
    if resp.hovered() {
        let scroll_y = ui.input(|i| i.raw_scroll_delta.y);
        if scroll_y != 0.0 {
            app.vinyl_scroll_accum += scroll_y;
            while app.vinyl_scroll_accum >= VINYL_SCROLL_PX_PER_STEP {
                app.vinyl_scroll_accum -= VINYL_SCROLL_PX_PER_STEP;
                app.vinyl_speed = app.vinyl_speed.saturating_add(1);
                app.inject_vinyl();
            }
            while app.vinyl_scroll_accum <= -VINYL_SCROLL_PX_PER_STEP {
                app.vinyl_scroll_accum += VINYL_SCROLL_PX_PER_STEP;
                app.vinyl_speed = app.vinyl_speed.saturating_sub(1);
                app.inject_vinyl();
            }
        }
    }

    // Indicator angle derived from current vinyl_speed.
    let t = app.vinyl_speed as f32 / 255.0;
    let indicator_angle = VINYL_SPEED_TICK_ARC_START_CW_RAD + t * VINYL_SPEED_TICK_ARC_SPAN_RAD;

    draw_rotary_control(
        p,
        center,
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
        },
    );

    // Static labels and endstop brackets live in right/statics.rs.
}
