//! NAV rotary - large outer ring + inner press button + scroll detents that
//! drive the menu cursor on the LCD.

use egui::{Color32, FontFamily, Sense, Stroke, Vec2};

use crate::app::CdjApp;

use super::super::{
    draw_rotary_control, paint_double_circle_ring, ArcDecorSpec, ArcNotchSpec, DoubleBorderSpec,
    RotaryControlSpec, RotaryGearSpec, RotaryIndicatorSpec, RotaryTickSpec, StrokeSpec, UiScale,
    COL_BLACK, COL_BTN, COL_BTN_TEXT, COL_DARK, COL_SILVER,
};
use super::vinyl_speed::{
    VINYL_SPEED_KNOB_RADIUS_REF, VINYL_SPEED_STROKE_REF, VINYL_SPEED_TOOTH_TOP_FRAC,
    VINYL_SPEED_VALLEY_SEGMENTS,
};
use super::*;

// ── Navigation rotary (owned by this module) ─────────────────────────────────
/// Vertical position of the nav rotary center within MODES_COL_REF (0=top,1=bottom).
pub(super) const NAV_CENTER_V: f32 = 0.135;
/// Horizontal position of the nav rotary center within MODES_COL_REF.
pub(super) const NAV_CENTER_U: f32 = 0.365;
/// Scale factor relative to VINYL_SPEED_SIZE_SCALE (×1.2 as requested).
pub(super) const NAV_SIZE_SCALE: f32 = 1.00;
/// Gear radius (ref units) - same as vinyl speed knob.
pub(super) const NAV_KNOB_RADIUS_REF: f32 = VINYL_SPEED_KNOB_RADIUS_REF;
/// Gear tooth count.
pub(super) const NAV_TOOTH_COUNT: usize = 12;
/// Gear tooth amplitude (ref units).
pub(super) const NAV_TOOTH_AMP_REF: f32 = 15.0;
pub(super) const NAV_TOOTH_TOP_FRAC: f32 = VINYL_SPEED_TOOTH_TOP_FRAC;
pub(super) const NAV_VALLEY_SEGMENTS: usize = VINYL_SPEED_VALLEY_SEGMENTS;
pub(super) const NAV_STROKE_REF: f32 = VINYL_SPEED_STROKE_REF;
/// Gap (ref units) between the gear outer radius and the inner edge of the bezel ring.
pub(super) const NAV_BEZEL_INNER_GAP_REF: f32 = 12.0;
/// Width (ref units) of the filled bezel ring between gear and buttons.
pub(super) const NAV_BEZEL_WIDTH_REF: f32 = 70.0;
/// Stroke width for the two circle outlines bounding the bezel ring (ref units).
pub(super) const NAV_BEZEL_STROKE_REF: f32 = 6.0;
/// Outer radius of the arc buttons (ref units) - same for all 4 buttons.
pub(super) const NAV_BTN_OUTER_R_REF: f32 = 363.0;
/// Half-angle (radians) each button spans from 12 o'clock (top) or 6 o'clock (bottom).
pub(super) const NAV_BTN_OUTER_ANGLE_RAD: f32 = 50.0 * std::f32::consts::PI / 180.0;
/// Inner stroke width for the double border on arc buttons (ref units).
pub(super) const NAV_BTN_BORDER_INNER_W_REF: f32 = 9.0;
/// Outer stroke width for the double border on arc buttons (ref units).
pub(super) const NAV_BTN_BORDER_OUTER_W_REF: f32 = 6.0;
/// Gap between inner and outer border strokes (ref units).
pub(super) const NAV_BTN_BORDER_GAP_REF: f32 = 1.5;
/// Decorative arc inside each button: radius from nav center (ref units).
pub(super) const NAV_BTN_DEC_ARC_R_REF: f32 = 255.0;
/// Stroke width of the decorative arc (ref units).
pub(super) const NAV_BTN_DEC_ARC_STROKE_REF: f32 = 6.0;
/// Notch arc radius (ref units) - sits between the bezel and the decorative arc.
pub(super) const NAV_BTN_NOTCH_ARC_R_REF: f32 = 265.0;
/// Half-angle of the notch arc (degrees).
pub(super) const NAV_BTN_NOTCH_HALF_ANGLE_DEG: f32 = 12.0;
/// Stroke width of the notch arc - thicker than the decorative arc (ref units).
pub(super) const NAV_BTN_NOTCH_STROKE_REF: f32 = 10.0;
/// Decorative inner circle inside the nav rotary knob: radius (ref units).
pub(super) const NAV_INNER_CIRCLE_R_REF: f32 = 85.0;
/// Inner stroke width for the inner circle ring (ref units).
pub(super) const NAV_INNER_CIRCLE_INNER_W_REF: f32 = 3.0;
/// Outer stroke width for the inner circle ring (ref units).
pub(super) const NAV_INNER_CIRCLE_OUTER_W_REF: f32 = 5.0;
/// Gap for the inner circle double ring (ref units).
pub(super) const NAV_INNER_CIRCLE_GAP_REF: f32 = 2.0;
/// Font size for quad button labels (ref units).
pub(super) const NAV_BTN_FONT_SIZE_REF: f32 = 28.0;
/// Per-button label nudge (ref units, dx/dy).
pub(super) const NAV_BTN_LABEL_NUDGE_BACK: (f32, f32) = (0.0, -30.0);
pub(super) const NAV_BTN_LABEL_NUDGE_TAG: (f32, f32) = (45.0, -40.0);
pub(super) const NAV_BTN_LABEL_NUDGE_FILTER: (f32, f32) = (50.0, 30.0);
pub(super) const NAV_BTN_LABEL_NUDGE_SHORT: (f32, f32) = (40.0, 30.0);
/// Line height (ref units) for multi-line button labels.
pub(super) const NAV_BTN_LINE_HEIGHT_REF: f32 = 28.0;
/// Number of detent positions per full revolution.
pub(crate) const NAV_DETENT_COUNT: f32 = 18.0;
/// Pixels of smooth scroll needed to advance one rotary detent.
pub(crate) const NAV_SCROLL_PX_PER_TICK: f32 = 50.0;

// ── Encoder LED glow ring ─────────────────────────────────────────────────────
/// Number of outward halo layers (fattening the ring stroke per layer).
pub(super) const NAV_ENCODER_GLOW_LAYERS: usize = 8;
/// Maximum total widening of the ring stroke for the outermost glow layer (ref units).
pub(super) const NAV_ENCODER_GLOW_SPREAD_REF: f32 = 25.0;
/// Width of the main fill stroke at full alpha (ref units).
pub(super) const NAV_ENCODER_MAIN_W_REF: f32 = 8.0;

impl CdjApp {
    pub(super) fn draw_nav_rotary(
        &mut self,
        ui: &mut egui::Ui,
        p: &egui::Painter,
        layout: &UiScale,
    ) {
        let col = MODES_COL_REF;
        let s = |r: f32| layout.sc(r * NAV_SIZE_SCALE);

        let center = layout.sp_in_rect(col, NAV_CENTER_U, NAV_CENTER_V);
        let r_base = s(NAV_KNOB_RADIUS_REF);
        let tooth_amp = s(NAV_TOOTH_AMP_REF);
        let r_outer = r_base + tooth_amp;

        // ── Scroll / press interaction on the gear circle ────────────────────
        let interact_r = r_outer * 1.1;
        let interact_rect = egui::Rect::from_center_size(center, Vec2::splat(interact_r * 2.0));
        let resp = ui.interact(
            interact_rect,
            ui.id().with("nav_rotary"),
            Sense::click_and_drag(), // drag keeps pointer captured while held (no drag-rotate, just ensures clean release)
        );

        // Scroll wheel → rotary ticks (accumulated to avoid sub-tick jitter).
        // Use raw_scroll_delta so ctrl+scroll still works (egui routes smooth_scroll_delta
        // to its zoom accumulator when ctrl is held, zeroing it for us).
        if resp.hovered() {
            let scroll_y = ui.input(|i| i.raw_scroll_delta.y);
            if scroll_y != 0.0 {
                self.nav_scroll_accum += scroll_y;
                let detent_rad = std::f32::consts::TAU / NAV_DETENT_COUNT;
                while self.nav_scroll_accum >= NAV_SCROLL_PX_PER_TICK {
                    self.nav_scroll_accum -= NAV_SCROLL_PX_PER_TICK;
                    self.nav_angle += detent_rad;
                    self.rotary = self.rotary.wrapping_sub(1);
                    self.inject_rotary();
                }
                while self.nav_scroll_accum <= -NAV_SCROLL_PX_PER_TICK {
                    self.nav_scroll_accum += NAV_SCROLL_PX_PER_TICK;
                    self.nav_angle -= detent_rad;
                    self.rotary = self.rotary.wrapping_add(1);
                    self.inject_rotary();
                }
            }
        }

        // Click → BTN_ROTARY_PRESS with ctrl-latch support.
        let is_down = resp.is_pointer_button_down_on();
        let ctrl = ui.input(|i| i.modifiers.ctrl);
        self.handle_btn_interaction(is_down, ctrl, miso_frame::BTN_ROTARY_PRESS);

        // Radii for the bezel ring and buttons.
        let bezel_inner_r = r_outer + s(NAV_BEZEL_INNER_GAP_REF);
        let bezel_outer_r = bezel_inner_r + s(NAV_BEZEL_WIDTH_REF);
        let btn_outer_r = s(NAV_BTN_OUTER_R_REF);

        // ── 1. Arc buttons: TOP pair + BOTTOM pair, nothing on the sides ─────
        // Each button spans NAV_BTN_OUTER_ANGLE_RAD from 12 o'clock (top pair)
        // or 6 o'clock (bottom pair). The left/right sides of the bezel are bare.
        // Angles in screen math: 0=right, π/2=down, -π/2=up.
        let btn_border = DoubleBorderSpec::from_strokes_with_gap(
            StrokeSpec {
                width: layout.sc(NAV_BTN_BORDER_INNER_W_REF),
                color: COL_DARK,
            },
            StrokeSpec {
                width: layout.sc(NAV_BTN_BORDER_OUTER_W_REF),
                color: COL_SILVER,
            },
            layout.sc(NAV_BTN_BORDER_GAP_REF),
        );
        let btn_dec_arc = ArcDecorSpec {
            radius: s(NAV_BTN_DEC_ARC_R_REF),
            stroke: Stroke::new(layout.sc(NAV_BTN_DEC_ARC_STROKE_REF), COL_SILVER),
        };
        let btn_notch_arc = ArcNotchSpec {
            radius: s(NAV_BTN_NOTCH_ARC_R_REF),
            half_angle: NAV_BTN_NOTCH_HALF_ANGLE_DEG * std::f32::consts::PI / 180.0,
            stroke: Stroke::new(layout.sc(NAV_BTN_NOTCH_STROKE_REF), COL_SILVER),
        };
        let font_sz = layout.sc(NAV_BTN_FONT_SIZE_REF);
        let btn_fill = COL_BTN;
        let btn_press = COL_DARK;

        let a = NAV_BTN_OUTER_ANGLE_RAD;
        let half_pi = std::f32::consts::FRAC_PI_2;

        let nudge = |n: (f32, f32)| egui::Vec2::new(layout.sc(n.0), layout.sc(n.1));

        let line_h = Some(layout.sc(NAV_BTN_LINE_HEIGHT_REF));

        // BACK - top-left: from (-π/2 - a) to (-π/2)
        self.arc_quad_btn(
            ui,
            center,
            bezel_outer_r,
            btn_outer_r,
            -half_pi - a,
            -half_pi,
            btn_fill,
            btn_press,
            btn_border,
            "BACK",
            font_sz,
            COL_BTN_TEXT,
            nudge(NAV_BTN_LABEL_NUDGE_BACK),
            None,
            FontFamily::Proportional,
            Some(btn_dec_arc),
            Some(btn_notch_arc),
            "nav_back",
            miso_frame::BTN_BACK,
        );
        // TAG TRACK - top-right: from (-π/2) to (-π/2 + a)
        self.arc_quad_btn(
            ui,
            center,
            bezel_outer_r,
            btn_outer_r,
            -half_pi,
            -half_pi + a,
            btn_fill,
            btn_press,
            btn_border,
            "TAG TRACK\n/ REMOVE",
            font_sz,
            COL_BTN_TEXT,
            nudge(NAV_BTN_LABEL_NUDGE_TAG),
            line_h,
            FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS_CONDENSED.into()),
            Some(btn_dec_arc),
            None,
            "nav_tag_track",
            miso_frame::BTN_TAG_TRACK,
        );
        // TRACK FILTER - bottom-left: from (π/2) to (π/2 + a)
        self.arc_quad_btn(
            ui,
            center,
            bezel_outer_r,
            btn_outer_r,
            half_pi,
            half_pi + a,
            btn_fill,
            btn_press,
            btn_border,
            "▪TRACK\nFILTER\n▬ EDIT",
            font_sz,
            COL_BTN_TEXT,
            nudge(NAV_BTN_LABEL_NUDGE_FILTER),
            line_h,
            FontFamily::Proportional,
            Some(btn_dec_arc),
            None,
            "nav_track_filter",
            miso_frame::BTN_TRACK_FILTER,
        );
        // SHORT CUT - bottom-right: from (π/2 - a) to (π/2)
        self.arc_quad_btn(
            ui,
            center,
            bezel_outer_r,
            btn_outer_r,
            half_pi - a,
            half_pi,
            btn_fill,
            btn_press,
            btn_border,
            "SHORT\n  CUT",
            font_sz,
            COL_BTN_TEXT,
            nudge(NAV_BTN_LABEL_NUDGE_SHORT),
            line_h,
            FontFamily::Proportional,
            Some(btn_dec_arc),
            None,
            "nav_shortcut",
            miso_frame::BTN_SHORTCUT,
        );

        // ── 2. Bezel ring between gear and buttons ────────────────────────────
        // circle_stroke renders the stroke outward from the given radius, so pass
        // bezel_inner_r directly so the fill starts exactly at the gear outer edge.
        let bezel_thickness = bezel_outer_r - bezel_inner_r;
        p.circle_stroke(
            center,
            bezel_inner_r,
            Stroke::new(bezel_thickness, COL_BLACK),
        );
        let bezel_outline = Stroke::new(layout.sc(NAV_BEZEL_STROKE_REF), COL_SILVER);
        p.circle_stroke(center, bezel_outer_r, bezel_outline);
        p.circle_stroke(center, bezel_inner_r, bezel_outline);

        // ── 2.5 Encoder LED glow ring - LED driven (live) ────────────────────
        {
            let encoder_on = self.led_state.mosi().led_bit(mosi_frame::LED_ENCODER);
            if encoder_on {
                // Same pipeline as on-air RGB: feed raw PWM bytes through led_color()
                // for gamma-expand + normalize-to-LED_PEAK on the dominant channel.
                let glow_col = mosi_frame::led_color(0x0a, 0x0f, 0x0f)
                    .unwrap_or(Color32::from_rgb(200, 220, 220));
                let main_w = layout.sc(NAV_ENCODER_MAIN_W_REF);
                let max_expand = layout.sc(NAV_ENCODER_GLOW_SPREAD_REF);
                // Outward halo layers: fatter stroke + lower alpha, outermost first.
                // Mirrors the on-air bar's expand-and-fade structure (draw_top.rs:312).
                for layer in 0..NAV_ENCODER_GLOW_LAYERS {
                    let t = 1.0 - layer as f32 / (NAV_ENCODER_GLOW_LAYERS - 1).max(1) as f32;
                    let expand = t * max_expand;
                    let glow_alpha = t * 0.28;
                    if glow_alpha < 0.005 {
                        continue;
                    }
                    p.circle_stroke(
                        center,
                        bezel_inner_r,
                        Stroke::new(main_w + expand * 2.0, glow_col.gamma_multiply(glow_alpha)),
                    );
                }
                // Main fill LAST at full alpha - same as on-air's solid trapezoid.
                p.circle_stroke(center, bezel_inner_r, Stroke::new(main_w, glow_col));
            }
        }

        // ── 3. Gear drawn last (on top of bezel and buttons) ─────────────────
        let nav_pressed = resp.is_pointer_button_down_on()
            || self.latched_btns.contains(&miso_frame::BTN_ROTARY_PRESS);
        let nav_fill = if nav_pressed { COL_DARK } else { COL_BTN };

        draw_rotary_control(
            p,
            center,
            RotaryControlSpec {
                gear: RotaryGearSpec {
                    base_radius: r_base,
                    tooth_count: NAV_TOOTH_COUNT,
                    tooth_amplitude: tooth_amp,
                    tooth_top_fraction: NAV_TOOTH_TOP_FRAC,
                    rotation_rad: self.nav_angle,
                    valley_segments: NAV_VALLEY_SEGMENTS,
                    fill: nav_fill,
                    stroke: StrokeSpec {
                        width: s(NAV_STROKE_REF),
                        color: COL_BTN_TEXT,
                    },
                },
                indicator: RotaryIndicatorSpec {
                    length: 0.0,
                    width: 0.0,
                    angle_cw_from_top_rad: 0.0,
                    stroke: StrokeSpec {
                        width: 0.0,
                        color: COL_BTN_TEXT,
                    },
                },
                ticks: RotaryTickSpec {
                    count: 0,
                    arc_start_cw_rad: 0.0,
                    arc_span_rad: 0.0,
                    ring_offset: 0.0,
                    dot_radius: 0.0,
                    center_dot_radius: 0.0,
                    endcap_length: 0.0,
                    endcap_stroke: StrokeSpec {
                        width: 0.0,
                        color: COL_BTN_TEXT,
                    },
                },
                color: COL_BTN_TEXT,
                draw_indicator: false,
                draw_ticks: false,
            },
        );

        // ── 4. Inner decorative circle on the rotary knob ────────────────────
        let inner_r = s(NAV_INNER_CIRCLE_R_REF);
        p.circle_filled(center, inner_r, nav_fill);
        paint_double_circle_ring(
            p,
            center,
            inner_r,
            DoubleBorderSpec::from_strokes_with_gap(
                StrokeSpec {
                    width: layout.sc(NAV_INNER_CIRCLE_INNER_W_REF),
                    color: COL_SILVER,
                },
                StrokeSpec {
                    width: layout.sc(NAV_INNER_CIRCLE_OUTER_W_REF),
                    color: COL_SILVER,
                },
                layout.sc(NAV_INNER_CIRCLE_GAP_REF),
            ),
        );
    }
}
