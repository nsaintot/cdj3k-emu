//! Draws the NAV rotary: its large outer ring, its inner press button, and
//! the scroll detents that move the menu cursor on the LCD.

use cdj3k_emu_panel::{Btn, Lamp};
use egui::{Color32, FontFamily, Pos2, Sense, Stroke, Vec2};

use crate::app::CdjApp;

use super::super::{
    draw_rotary_control, paint_double_circle_ring, scroll_detents, ArcDecorSpec, ArcNotchSpec,
    DoubleBorderSpec, RotaryControlSpec, RotaryGearSpec, StrokeSpec, UiScale, COL_BLACK, COL_BTN,
    COL_BTN_TEXT, COL_DARK, COL_SILVER, NAV_DETENT_COUNT, NAV_SCROLL_PX_PER_TICK,
};
use super::*;
use crate::app::ui::draw_vinyl_speed::{
    VINYL_SPEED_KNOB_RADIUS_REF, VINYL_SPEED_STROKE_REF, VINYL_SPEED_TOOTH_TOP_FRAC,
    VINYL_SPEED_VALLEY_SEGMENTS,
};

// ── Navigation rotary (owned by this module) ─────────────────────────────────
/// Vertical position of the nav rotary center within MODES_COL_REF, from 0 at
/// the top to 1 at the bottom.
pub(super) const NAV_CENTER_V: f32 = layout::NAV_POD_V;
/// Colour of the single-colour encoder LED; the encoder bit only switches it
/// on and off.
const NAV_ENCODER_GLOW_COL: Color32 = Color32::from_rgb(182, 220, 220);

/// Horizontal position of the nav rotary center within MODES_COL_REF.
pub(super) const NAV_CENTER_U: f32 = layout::NAV_POD_U;
/// The size multiplier for the nav rotary's reference dimensions.
pub(super) const NAV_SIZE_SCALE: f32 = 1.00;
/// Gear radius in ref units, the same as the vinyl speed knob's.
pub(super) const NAV_KNOB_RADIUS_REF: f32 = VINYL_SPEED_KNOB_RADIUS_REF;
/// Gear tooth count.
pub(super) const NAV_TOOTH_COUNT: usize = 12;
/// Gear tooth amplitude (ref units).
pub(super) const NAV_TOOTH_AMP_REF: f32 = 15.0;
pub(super) const NAV_TOOTH_TOP_FRAC: f32 = VINYL_SPEED_TOOTH_TOP_FRAC;
pub(super) const NAV_VALLEY_SEGMENTS: usize = VINYL_SPEED_VALLEY_SEGMENTS;
pub(super) const NAV_STROKE_REF: f32 = VINYL_SPEED_STROKE_REF;
/// Gap (ref units) between the gear outer radius and the inner edge of the bezel ring.
pub(in crate::app) const NAV_BEZEL_INNER_GAP_REF: f32 = 12.0;
/// Width (ref units) of the filled bezel ring between gear and buttons.
pub(in crate::app) const NAV_BEZEL_WIDTH_REF: f32 = 82.0;
/// Stroke width for the two circle outlines bounding the bezel ring (ref units).
pub(super) const NAV_BEZEL_STROKE_REF: f32 = 6.0;
/// Outer radius of the arc buttons in ref units, the same for all four.
pub(super) const NAV_BTN_OUTER_R_REF: f32 = 363.0;
/// Half-angle (radians) each button spans from 12 o'clock (top) or 6 o'clock (bottom).
pub(super) const NAV_BTN_OUTER_ANGLE_RAD: f32 = 45.5 * std::f32::consts::PI / 180.0;
/// Inner stroke width for the double border on arc buttons (ref units).
pub(super) const NAV_BTN_BORDER_INNER_W_REF: f32 = 9.0;
/// Outer stroke width for the double border on arc buttons (ref units).
pub(super) const NAV_BTN_BORDER_OUTER_W_REF: f32 = 6.0;
/// Gap between inner and outer border strokes (ref units).
pub(super) const NAV_BTN_BORDER_GAP_REF: f32 = 1.5;
/// The radius of the decorative arc inside each button, measured from the nav
/// center, in ref units.
pub(super) const NAV_BTN_DEC_ARC_R_REF: f32 = 255.0;
/// Stroke width of the decorative arc (ref units).
pub(super) const NAV_BTN_DEC_ARC_STROKE_REF: f32 = 6.0;
/// Notch arc radius in ref units; the notch arc sits between the bezel and
/// the decorative arc.
pub(super) const NAV_BTN_NOTCH_ARC_R_REF: f32 = 265.0;
/// Half-angle of the notch arc (degrees).
pub(super) const NAV_BTN_NOTCH_HALF_ANGLE_DEG: f32 = 12.0;
/// Stroke width of the notch arc in ref units; it is thicker than the
/// decorative arc.
pub(super) const NAV_BTN_NOTCH_STROKE_REF: f32 = 10.0;
/// The radius of the decorative inner circle on the nav rotary knob, in ref
/// units.
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

// ── Encoder LED glow ring ─────────────────────────────────────────────────────
/// Number of outward halo layers (fattening the ring stroke per layer).
pub(super) const NAV_ENCODER_GLOW_LAYERS: usize = 8;
/// Maximum total widening of the ring stroke for the outermost glow layer (ref units).
pub(super) const NAV_ENCODER_GLOW_SPREAD_REF: f32 = 25.0;
/// Width of the main fill stroke at full alpha (ref units).
pub(super) const NAV_ENCODER_MAIN_W_REF: f32 = 8.0;

/// One arc button of the pod: its span in screen angles, where 0 points right,
/// π/2 down and -π/2 up; its legend and the legend's nudge in ref units;
/// whether the legend is condensed and line-spaced; and whether it carries the
/// notch arc.
struct NavButton {
    span: (f32, f32),
    label: &'static str,
    nudge: (f32, f32),
    multiline: bool,
    condensed: bool,
    notch: bool,
    id: &'static str,
    btn: Btn,
}

const HALF_PI: f32 = std::f32::consts::FRAC_PI_2;

const NAV_BUTTONS: [NavButton; 4] = [
    // BACK - top-left: from (-π/2 - a) to (-π/2)
    NavButton {
        span: (-HALF_PI - NAV_BTN_OUTER_ANGLE_RAD, -HALF_PI),
        label: "BACK",
        nudge: NAV_BTN_LABEL_NUDGE_BACK,
        multiline: false,
        condensed: false,
        notch: true,
        id: "nav_back",
        btn: Btn::Back,
    },
    // TAG TRACK - top-right: from (-π/2) to (-π/2 + a)
    NavButton {
        span: (-HALF_PI, -HALF_PI + NAV_BTN_OUTER_ANGLE_RAD),
        label: "TAG TRACK\n/ REMOVE",
        nudge: NAV_BTN_LABEL_NUDGE_TAG,
        multiline: true,
        condensed: true,
        notch: false,
        id: "nav_tag_track",
        btn: Btn::TagTrack,
    },
    // TRACK FILTER - bottom-left: from (π/2) to (π/2 + a)
    NavButton {
        span: (HALF_PI, HALF_PI + NAV_BTN_OUTER_ANGLE_RAD),
        label: "▪TRACK\nFILTER\n▬ EDIT",
        nudge: NAV_BTN_LABEL_NUDGE_FILTER,
        multiline: true,
        condensed: false,
        notch: false,
        id: "nav_track_filter",
        btn: Btn::TrackFilter,
    },
    // SHORT CUT - bottom-right: from (π/2 - a) to (π/2)
    NavButton {
        span: (HALF_PI - NAV_BTN_OUTER_ANGLE_RAD, HALF_PI),
        label: "SHORT\n  CUT",
        nudge: NAV_BTN_LABEL_NUDGE_SHORT,
        multiline: true,
        condensed: false,
        notch: false,
        id: "nav_shortcut",
        btn: Btn::Shortcut,
    },
];

pub(super) fn draw_nav_rotary(
    app: &mut CdjApp,
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

    let resp = nav_interact(app, ui, center, r_outer);

    // Radii for the bezel ring and buttons.
    let bezel_inner_r = r_outer + s(NAV_BEZEL_INNER_GAP_REF);
    let bezel_outer_r = bezel_inner_r + s(NAV_BEZEL_WIDTH_REF);
    let btn_outer_r = s(NAV_BTN_OUTER_R_REF);
    // The pod, with its bezel and arc buttons, is drawn over the extended
    // screen, so its bloom is kept there.
    let pod_edge = layout
        .sc(NAV_BTN_BORDER_INNER_W_REF + NAV_BTN_BORDER_GAP_REF * 2.0 + NAV_BTN_BORDER_OUTER_W_REF);
    app.bloom_keeps.push(crate::app::bloom::BloomKeep {
        centre: center,
        core_r: bezel_outer_r + pod_edge,
        outer_r: btn_outer_r + pod_edge,
        half_w: bezel_outer_r * NAV_BTN_OUTER_ANGLE_RAD.sin() + pod_edge * 0.5,
        tan_a: 0.0,
    });

    draw_nav_buttons(app, ui, layout, center, bezel_outer_r, btn_outer_r);
    paint_bezel(p, layout, center, bezel_inner_r, bezel_outer_r);
    paint_encoder_glow(app, p, layout, center, bezel_inner_r);
    paint_nav_gear(app, p, layout, &resp, center, r_base, tooth_amp);
}

/// Handles scrolling and pressing on the gear circle.
fn nav_interact(app: &mut CdjApp, ui: &mut egui::Ui, center: Pos2, r_outer: f32) -> egui::Response {
    let interact_r = r_outer * 1.1;
    let interact_rect = egui::Rect::from_center_size(center, Vec2::splat(interact_r * 2.0));
    let resp = ui.interact(
        interact_rect,
        ui.id().with("nav_rotary"),
        // Sensing drags keeps the pointer captured while the button is held,
        // so the release is clean; dragging does not rotate the knob.
        Sense::click_and_drag(),
    );

    // Turn scroll-wheel movement into rotary ticks, accumulated to avoid
    // sub-tick jitter. Read the raw wheel events so that Ctrl-scroll still
    // works: while Ctrl is held, egui sends its smoothed delta to its zoom
    // accumulator and leaves it at zero.
    if resp.hovered() {
        let scroll_y = crate::app::scroll::raw_delta(ui).y;
        if scroll_y != 0.0 {
            let detent_rad = std::f32::consts::TAU / NAV_DETENT_COUNT;
            let ticks = scroll_detents(&mut app.nav_scroll_accum, scroll_y, NAV_SCROLL_PX_PER_TICK);
            for _ in 0..ticks.unsigned_abs() {
                if ticks > 0 {
                    app.nav_angle += detent_rad;
                    app.rotary = app.rotary.wrapping_sub(1);
                } else {
                    app.nav_angle -= detent_rad;
                    app.rotary = app.rotary.wrapping_add(1);
                }
                app.inject_rotary();
            }
        }
    }

    // A click sends BTN_ROTARY_PRESS; holding Ctrl latches it.
    let is_down = resp.is_pointer_button_down_on();
    let ctrl = ui.input(|i| i.modifiers.ctrl);
    if let Some(bit) = app.button(Btn::RotaryPress) {
        app.handle_btn_interaction(is_down, ctrl, bit);
    }
    resp
}

/// Draws the arc buttons: a pair at the top and a pair at the bottom, with
/// nothing on the sides of the bezel. Each button spans
/// NAV_BTN_OUTER_ANGLE_RAD from 12 o'clock (top pair) or 6 o'clock (bottom
/// pair). Angles are screen angles: 0 is right, π/2 is down and -π/2 is up.
fn draw_nav_buttons(
    app: &mut CdjApp,
    ui: &mut egui::Ui,
    layout: &UiScale,
    center: Pos2,
    bezel_outer_r: f32,
    btn_outer_r: f32,
) {
    let s = |r: f32| layout.sc(r * NAV_SIZE_SCALE);
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

    let nudge = |n: (f32, f32)| egui::Vec2::new(layout.sc(n.0), layout.sc(n.1));

    let line_h = layout.sc(NAV_BTN_LINE_HEIGHT_REF);
    let condensed = FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS_CONDENSED.into());

    for b in &NAV_BUTTONS {
        app.arc_quad_btn(
            ui,
            center,
            bezel_outer_r,
            btn_outer_r,
            b.span.0,
            b.span.1,
            btn_fill,
            btn_press,
            btn_border,
            b.label,
            font_sz,
            COL_BTN_TEXT,
            nudge(b.nudge),
            b.multiline.then_some(line_h),
            if b.condensed {
                condensed.clone()
            } else {
                FontFamily::Proportional
            },
            Some(btn_dec_arc),
            b.notch.then_some(btn_notch_arc),
            b.id,
            b.btn,
        );
    }
}

/// Paints the bezel ring between the gear and the buttons.
fn paint_bezel(
    p: &egui::Painter,
    layout: &UiScale,
    center: Pos2,
    bezel_inner_r: f32,
    bezel_outer_r: f32,
) {
    // circle_stroke draws the stroke outward from the given radius, so passing
    // bezel_inner_r starts the fill at the gear's outer edge.
    let bezel_thickness = bezel_outer_r - bezel_inner_r;
    p.circle_stroke(
        center,
        bezel_inner_r,
        Stroke::new(bezel_thickness, COL_BLACK),
    );
    let bezel_outline = Stroke::new(layout.sc(NAV_BEZEL_STROKE_REF), COL_SILVER);
    p.circle_stroke(center, bezel_outer_r, bezel_outline);
    p.circle_stroke(center, bezel_inner_r, bezel_outline);
}

/// Paints the encoder's glow ring while the deck lights the encoder LED.
fn paint_encoder_glow(
    app: &CdjApp,
    p: &egui::Painter,
    layout: &UiScale,
    center: Pos2,
    bezel_inner_r: f32,
) {
    let encoder_on = app.mosi().lit(Lamp::Encoder);
    if encoder_on {
        let glow_col = NAV_ENCODER_GLOW_COL;
        let main_w = layout.sc(NAV_ENCODER_MAIN_W_REF);
        let max_expand = layout.sc(NAV_ENCODER_GLOW_SPREAD_REF);
        // Paint the outward halo layers, outermost first; each layer has a
        // wider stroke and a lower alpha, like the on-air bar's glow
        // (`draw_on_air_bar` in draw_top.rs).
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
        // Paint the main fill last, at full alpha, like the on-air bar's solid
        // trapezoid.
        p.circle_stroke(center, bezel_inner_r, Stroke::new(main_w, glow_col));
    }
}

/// Paints the gear last, over the bezel and the buttons, and the decorative
/// inner circle on it.
fn paint_nav_gear(
    app: &CdjApp,
    p: &egui::Painter,
    layout: &UiScale,
    resp: &egui::Response,
    center: Pos2,
    r_base: f32,
    tooth_amp: f32,
) {
    let s = |r: f32| layout.sc(r * NAV_SIZE_SCALE);
    let nav_pressed = resp.is_pointer_button_down_on()
        || app
            .button(Btn::RotaryPress)
            .is_some_and(|bit| app.latched_btns.contains(&bit));
    let nav_fill = if nav_pressed { COL_DARK } else { COL_BTN };

    draw_rotary_control(
        p,
        center,
        RotaryControlSpec::gear_only(
            RotaryGearSpec {
                base_radius: r_base,
                tooth_count: NAV_TOOTH_COUNT,
                tooth_amplitude: tooth_amp,
                tooth_top_fraction: NAV_TOOTH_TOP_FRAC,
                rotation_rad: app.nav_angle,
                valley_segments: NAV_VALLEY_SEGMENTS,
                fill: nav_fill,
                stroke: StrokeSpec {
                    width: s(NAV_STROKE_REF),
                    color: COL_BTN_TEXT,
                },
            },
            COL_BTN_TEXT,
        ),
    );

    // ── Inner decorative circle on the rotary knob ───────────────────────
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
