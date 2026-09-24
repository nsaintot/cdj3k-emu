//! CDJ-3000 form-factor outline.
//!
//! Draws the device silhouette as the bottom-most layer, behind all controls.
//! All geometry is in reference space and scaled via [`UiScale`].

use std::f32::consts::PI;

use egui::{Color32, Pos2, Shape, Stroke};

use crate::app::ui::{draw_cache::ShapeList, COL_SILVER};

use super::{layout, UiScale};

// ── Main chassis hull ─────────────────────────────────────────────────────────

const CHASSIS_LEFT: f32 = layout::TRANSPORT_REF_LEFT;
const CHASSIS_RIGHT: f32 = layout::MODES_REF_RIGHT;
const CHASSIS_TOP: f32 = layout::TRANSPORT_REF_TOP;
const CHASSIS_BOT: f32 = layout::TRANSPORT_REF_BOT;
const CHASSIS_ROUNDING: f32 = 25.0;

// ── Top panel (second rectangle, interlocked at top) ──────────────────────────
const TOP_PANEL_LEFT: f32 = layout::TOP_CENTRAL_REF_LEFT;
const TOP_PANEL_RIGHT: f32 = layout::TOP_CENTRAL_REF_RIGHT;
const TOP_PANEL_TOP: f32 = layout::TOP_CENTRAL_REF_TOP - 19.0;

// ── Cosmetic lcd panel lines ──────────────────────────────────────────────────
//
// Mirror the LCD geometry from draw_top.rs - must stay in sync if those change.
const LCD_PANEL_WIDTH_FRAC: f32 = 0.815;
const LCD_PANEL_ASPECT: f32 = 1280.0 / 800.0;
const LCD_PANEL_V_TOP: f32 = 0.176;
/// Clear distance from the display aperture out to the bezel frame.
const LCD_BEZEL_MARGIN: f32 = 130.0;

// ── Cosmetic bottom line
const COSBOT_LINE_1_Y: f32 = CHASSIS_BOT - 50.0;
const COSBOT_LINE_2_Y: f32 = CHASSIS_BOT - 90.0;
const COSBOT_LINE_1_W: f32 = 620.0;
const COSBOT_JOINT_ANGLE: f32 = 18.0;

// ── Inner decorative line ─────────────────────────────────────────────────────
//
// Two parallel runs inset from the chassis' top-left inner corner, down past
// the screen, along its foot, around the nav pod and back to the top-right
// inner corner. Rectilinear but for the detour, which is the pod's own outline
// pushed out: an arc concentric with the buttons' outer edge, and a straight
// side clearing the bezel's cut face.
/// Clear distance between the two runs. The drawing's own pen is far heavier
/// than ours, so matching its line centres left a gap twice the width of the
/// one it draws; this matches the gap instead.
/// The pair carries the chassis line's width on down from the inner corner it
/// springs from: the runs' own outer edges land on that line's edges, so their
/// stroke widths come out of the gap instead of standing proud of it.
const DECOR_GAP: f32 = CHASSIS_STROKE - (DECOR_OUTER_STROKE + DECOR_INNER_STROKE) * 0.5;
const DECOR_X_LEFT: f32 =
    layout::TOP_CENTRAL_REF_LEFT - (CHASSIS_STROKE - DECOR_OUTER_STROKE) * 0.5;
const DECOR_X_POD_LEFT: f32 =
    layout::TOP_CENTRAL_REF_RIGHT + (CHASSIS_STROKE - DECOR_OUTER_STROKE) * 0.5;
/// Both ends run into the chassis' top edge rather than stopping short of it.
const DECOR_Y_TOP: f32 = CHASSIS_TOP;
const DECOR_Y_BOT: f32 = 1793.0;
const DECOR_POD_CENTER_X: f32 =
    layout::MODES_REF_LEFT + layout::MODES_REF_SIZE_W * layout::NAV_POD_U;
const DECOR_POD_CENTER_Y: f32 =
    layout::MODES_REF_TOP + layout::MODES_REF_SIZE_H * layout::NAV_POD_V;
/// The inner run hugs the pod - the buttons' outer edge, then the bezel's cut
/// face - and the outer run stands [`DECOR_GAP`] beyond it. Both clear the
/// buttons' silhouette by 6: its arc reaches 372 and its cut face 159.9.
const DECOR_POD_INNER_R: f32 = 378.0;
const DECOR_POD_INNER_DX: f32 = 166.0;
const DECOR_POD_R: f32 = DECOR_POD_INNER_R + DECOR_GAP;
const DECOR_POD_DX: f32 = DECOR_POD_INNER_DX + DECOR_GAP;
/// The arc stops this far short of the pod's crown, so the run leaving it
/// carries the tangent's angle out to the corner instead of arriving flat -
/// which is what opens that corner past a right angle.
const DECOR_POD_TANGENT_DEG: f32 = 2.5;
const DECOR_SWEEP_SEGS: usize = 16;

// ── Stroke ────────────────────────────────────────────────────────────────────
const CHASSIS_STROKE: f32 = 10.0;
const COL_CHASSIS: Color32 = COL_SILVER;
const COSTOP_STROKE: f32 = 3.0;
const COSBOT_STROKE: f32 = 2.0;
const DECOR_OUTER_STROKE: f32 = 3.0;
const DECOR_INNER_STROKE: f32 = 2.0;

// ── Arc sample count per corner ───────────────────────────────────────────────
const CORNER_SEGS: usize = 12;
const CHASSIS_SEGS: usize = 5;
const CHASSIS_ROUNDED_CORNERS: usize = 4;

const TOP_PANEL_SEGS: usize = 3;
const TOP_PANEL_ROUNDED_CORNERS: usize = 2;
// ─────────────────────────────────────────────────────────────────────────────

/// Collect the device chassis silhouette into `out` (no-painter variant for caching).
pub(super) fn collect_chassis(out: &mut ShapeList, layout: &UiScale) {
    let stroke = Stroke::new(layout.sc(CHASSIS_STROKE), COL_CHASSIS);
    let cosbot_stroke = Stroke::new(layout.sc(COSBOT_STROKE), COL_CHASSIS);
    let chassis_left = layout.sp(CHASSIS_LEFT, CHASSIS_TOP).x;
    let chassis_right = layout.sp(CHASSIS_RIGHT, CHASSIS_TOP).x;
    let chassis_top = layout.sp(CHASSIS_LEFT, CHASSIS_TOP).y;
    let chassis_bottom = layout.sp(CHASSIS_LEFT, CHASSIS_BOT).y;
    let chassis_rounding = layout.sc(CHASSIS_ROUNDING);
    let panel_left = layout.sp(TOP_PANEL_LEFT, 0.0).x;
    let panel_right = layout.sp(TOP_PANEL_RIGHT, 0.0).x;
    let panel_top = layout.sp(0.0, TOP_PANEL_TOP).y;
    {
        let mut points: Vec<Pos2> = Vec::with_capacity(
            CORNER_SEGS * (CHASSIS_ROUNDED_CORNERS + CHASSIS_SEGS)
                + (CORNER_SEGS * TOP_PANEL_ROUNDED_CORNERS + TOP_PANEL_SEGS),
        );
        points.push(Pos2::new(panel_right, chassis_top));
        points.push(Pos2::new(chassis_right - chassis_rounding, chassis_top));
        push_arc(
            &mut points,
            chassis_right - chassis_rounding,
            chassis_top + chassis_rounding,
            chassis_rounding,
            3.0 * PI / 2.0,
            2.0 * PI,
        );
        points.push(Pos2::new(chassis_right, chassis_bottom - chassis_rounding));
        push_arc(
            &mut points,
            chassis_right - chassis_rounding,
            chassis_bottom - chassis_rounding,
            chassis_rounding,
            0.0,
            PI / 2.0,
        );
        points.push(Pos2::new(chassis_left + chassis_rounding, chassis_bottom));
        push_arc(
            &mut points,
            chassis_left + chassis_rounding,
            chassis_bottom - chassis_rounding,
            chassis_rounding,
            PI / 2.0,
            PI,
        );
        points.push(Pos2::new(chassis_left, chassis_top + chassis_rounding));
        push_arc(
            &mut points,
            chassis_left + chassis_rounding,
            chassis_top + chassis_rounding,
            chassis_rounding,
            PI,
            3.0 * PI / 2.0,
        );
        points.push(Pos2::new(panel_left, chassis_top));
        points.push(Pos2::new(panel_left, chassis_top));
        points.push(Pos2::new(panel_left, panel_top + chassis_rounding));
        push_arc(
            &mut points,
            panel_left + chassis_rounding,
            panel_top + chassis_rounding,
            chassis_rounding,
            PI,
            3.0 * PI / 2.0,
        );
        points.push(Pos2::new(panel_right - chassis_rounding, panel_top));
        push_arc(
            &mut points,
            panel_right - chassis_rounding,
            panel_top + chassis_rounding,
            chassis_rounding,
            3.0 * PI / 2.0,
            2.0 * PI,
        );
        points.push(Pos2::new(panel_right, chassis_top));
        out.add(Shape::line(points, stroke));
    }
    {
        let line1_y = layout.sp(0.0, COSBOT_LINE_1_Y).y;
        let line2_y = layout.sp(0.0, COSBOT_LINE_2_Y).y;
        let line1_width = layout.sc(COSBOT_LINE_1_W);
        let height_diff = line1_y - line2_y;
        let diagonal_run = height_diff / COSBOT_JOINT_ANGLE.to_radians().tan();
        let cosbot_points = vec![
            Pos2::new(chassis_left, line1_y),
            Pos2::new(chassis_left + line1_width, line1_y),
            Pos2::new(chassis_left + line1_width + diagonal_run, line2_y),
            Pos2::new(chassis_right - line1_width - diagonal_run, line2_y),
            Pos2::new(chassis_right - line1_width, line1_y),
            Pos2::new(chassis_right, line1_y),
        ];
        out.add(Shape::line(cosbot_points, cosbot_stroke));
    }
    collect_decor_line(out, layout);
}

/// Inner decorative line: the run on the path, then its parallel [`DECOR_GAP`]
/// to the left of travel.
fn collect_decor_line(out: &mut ShapeList, layout: &UiScale) {
    for (inset, width) in [(0.0, DECOR_OUTER_STROKE), (DECOR_GAP, DECOR_INNER_STROKE)] {
        out.add(Shape::line(
            decor_run(layout, inset),
            Stroke::new(layout.sc(width), COL_CHASSIS),
        ));
    }
}

/// One run of the decorative line, `inset` reference units to the left of the
/// path's travel; `0.0` is the run on the path itself.
fn decor_run(layout: &UiScale, inset: f32) -> Vec<Pos2> {
    let x_left = DECOR_X_LEFT + inset;
    let y_bot = DECOR_Y_BOT - inset;
    let x_pod_left = DECOR_X_POD_LEFT - inset;
    let r = DECOR_POD_R - inset;
    let tangent = DECOR_POD_TANGENT_DEG.to_radians();
    // Where the arc leaves the crown, and where it meets the straight side.
    let a_crown = PI / 2.0 - tangent;
    let a_side = ((DECOR_POD_DX - inset) / r).acos();
    // The tangent run from the arc's end out to its corner over the bottom
    // line; `sign` is +1 below the pod's centre line and -1 above it.
    let corner_y = |sign: f32| {
        let end_x = DECOR_POD_CENTER_X + r * a_crown.cos();
        let end_y = DECOR_POD_CENTER_Y + sign * r * a_crown.sin();
        end_y + sign * (end_x - x_pod_left) * tangent.tan()
    };

    let mut points = Vec::with_capacity(DECOR_SWEEP_SEGS * 2 + 8);
    points.push(layout.sp(x_left, DECOR_Y_TOP));
    points.push(layout.sp(x_left, y_bot));
    points.push(layout.sp(x_pod_left, y_bot));
    points.push(layout.sp(x_pod_left, corner_y(1.0)));
    push_sweep(&mut points, layout, r, a_crown, a_side);
    push_sweep(&mut points, layout, r, -a_side, -a_crown);
    points.push(layout.sp(x_pod_left, corner_y(-1.0)));
    points.push(layout.sp(x_pod_left, DECOR_Y_TOP));
    points
}

/// Push arc points about the pod's centre from `start_rad` to `end_rad`, in
/// reference space (inclusive of both ends).
fn push_sweep(points: &mut Vec<Pos2>, layout: &UiScale, r: f32, start_rad: f32, end_rad: f32) {
    for i in 0..=DECOR_SWEEP_SEGS {
        let t = i as f32 / DECOR_SWEEP_SEGS as f32;
        let angle = start_rad + (end_rad - start_rad) * t;
        points.push(layout.sp(
            DECOR_POD_CENTER_X + r * angle.cos(),
            DECOR_POD_CENTER_Y + r * angle.sin(),
        ));
    }
}

/// LCD bezel frame: one rectangle around the display aperture.
///
/// Called **after** all section fills so it is not buried under them.
/// The horizontal decoration lines that frame the LCD, as `(top, bot)` in
/// reference units. `View > Extend Screen` grows the panel to fill exactly this
/// band, so the screen's extended size and the decoration it stops against come
/// from one definition rather than two that must be kept equal by hand.
pub(super) fn lcd_decor_band() -> (f32, f32) {
    let menubar_width = layout::TOP_CENTRAL_REF_RIGHT - layout::TOP_CENTRAL_REF_LEFT;
    let lcd_height_ref = menubar_width * LCD_PANEL_WIDTH_FRAC / LCD_PANEL_ASPECT;
    let top = layout::TOP_CENTRAL_REF_TOP + layout::TOP_CENTRAL_REF_SIZE_H * LCD_PANEL_V_TOP
        - LCD_BEZEL_MARGIN;
    (top, top + lcd_height_ref + LCD_BEZEL_MARGIN * 2.0)
}

pub(super) fn collect_chassis_lcd_overlay(out: &mut ShapeList, layout: &UiScale) {
    let costop_stroke = Stroke::new(layout.sc(COSTOP_STROKE), COL_CHASSIS);
    let menubar_left = layout::TOP_CENTRAL_REF_LEFT;
    let menubar_right = layout::TOP_CENTRAL_REF_RIGHT;
    let lcd_width_ref = (menubar_right - menubar_left) * LCD_PANEL_WIDTH_FRAC;
    let center_x = (menubar_left + menubar_right) * 0.5;
    let left = center_x - lcd_width_ref * 0.5 - LCD_BEZEL_MARGIN;
    let right = center_x + lcd_width_ref * 0.5 + LCD_BEZEL_MARGIN;
    let (top, bot) = lcd_decor_band();
    out.add(Shape::line(
        vec![
            layout.sp(left, top),
            layout.sp(right, top),
            layout.sp(right, bot),
            layout.sp(left, bot),
            layout.sp(left, top),
        ],
        costop_stroke,
    ));
}

/// Push arc points from `start_rad` to `end_rad` (exclusive of start, inclusive of end).
fn push_arc(
    points: &mut Vec<Pos2>,
    center_x: f32,
    center_y: f32,
    radius: f32,
    start_rad: f32,
    end_rad: f32,
) {
    for i in 1..=CORNER_SEGS {
        let t = i as f32 / CORNER_SEGS as f32;
        let angle = start_rad + (end_rad - start_rad) * t;
        points.push(Pos2::new(
            center_x + radius * angle.cos(),
            center_y + radius * angle.sin(),
        ));
    }
}
