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
const TOP_PANEL_TOP: f32 = layout::TOP_CENTRAL_REF_TOP - 80.0;

// ── Cosmetic lcd panel lines ──────────────────────────────────────────────────
//
// The glass itself comes from the same `layout` constants that draw_top.rs
// uses (see `lcd_glass`).
/// The recess around the screen, in reference units: the mouth's top and
/// bottom, and the foot of the strip below it. The walls run from the mouth to
/// the glass and read as a trapezoid, because the deck is drawn from slightly
/// above.
const LCD_MOUTH_TOP: f32 = 283.0;
const LCD_MOUTH_BOT: f32 = 1467.0;
const LCD_STRIP_BOT: f32 = 1486.0;
/// Gap between the glass and its bezel, which is drawn as a second line
/// outside the glass.
const LCD_BEZEL_GAP: f32 = 7.0;

// ── Cosmetic bottom line
const COSBOT_LINE_1_Y: f32 = CHASSIS_BOT - 50.0;
const COSBOT_LINE_2_Y: f32 = CHASSIS_BOT - 90.0;
const COSBOT_LINE_1_W: f32 = 620.0;
const COSBOT_JOINT_ANGLE: f32 = 18.0;

// ── Inner decorative line ─────────────────────────────────────────────────────
//
// Two parallel runs inset from the chassis' top-left inner corner, down past
// the screen, along its foot, around the nav pod and back to the top-right
// inner corner. The runs are straight except for the detour around the pod,
// which follows the pod's outline pushed out: an arc concentric with the
// buttons' outer edge, and a straight side that clears the bezel ring.
/// Clear distance between the two runs.
/// The pair continues the chassis line's width down from its inner corner:
/// the outer edges of the runs line up with the edges of that line, so the
/// stroke widths are taken out of the gap.
const DECOR_GAP: f32 = CHASSIS_STROKE - (DECOR_OUTER_STROKE + DECOR_INNER_STROKE) * 0.5;
const DECOR_X_LEFT: f32 =
    layout::TOP_CENTRAL_REF_LEFT - (CHASSIS_STROKE - DECOR_OUTER_STROKE) * 0.5;
const DECOR_X_POD_LEFT: f32 =
    layout::TOP_CENTRAL_REF_RIGHT + (CHASSIS_STROKE - DECOR_OUTER_STROKE) * 0.5;
/// Both ends run into the chassis' top edge.
const DECOR_Y_TOP: f32 = CHASSIS_TOP;
const DECOR_Y_BOT: f32 = 1481.0;
const DECOR_POD_CENTER_X: f32 =
    layout::MODES_REF_LEFT + layout::MODES_REF_SIZE_W * layout::NAV_POD_U;
const DECOR_POD_CENTER_Y: f32 =
    layout::MODES_REF_TOP + layout::MODES_REF_SIZE_H * layout::NAV_POD_V;
/// The inner run follows the pod (the buttons' outer edge, then the bezel
/// ring), and the outer run lies [`DECOR_GAP`] beyond it.
const DECOR_POD_INNER_R: f32 = 378.0;
const DECOR_POD_INNER_DX: f32 = 164.0;
const DECOR_POD_R: f32 = DECOR_POD_INNER_R + DECOR_GAP;
const DECOR_POD_DX: f32 = DECOR_POD_INNER_DX + DECOR_GAP;
/// The arc stops this many degrees short of the pod's crown, so the run
/// leaving it keeps the tangent's angle out to the corner, which opens that
/// corner past a right angle.
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

/// Collects the chassis into `out` for caching: its outline, the cosmetic line
/// along its foot and the decoration line.
pub(super) fn collect_chassis(out: &mut ShapeList, layout: &UiScale) {
    collect_chassis_outline(out, layout);
    collect_cosmetic_bottom(out, layout);
    collect_decor_line(out, layout);
}

/// Collects the cabinet outline, which is rounded at its corners and rises
/// around the top panel.
fn collect_chassis_outline(out: &mut ShapeList, layout: &UiScale) {
    let stroke = Stroke::new(layout.sc(CHASSIS_STROKE), COL_CHASSIS);
    let chassis_left = layout.sp(CHASSIS_LEFT, CHASSIS_TOP).x;
    let chassis_right = layout.sp(CHASSIS_RIGHT, CHASSIS_TOP).x;
    let chassis_top = layout.sp(CHASSIS_LEFT, CHASSIS_TOP).y;
    let chassis_bottom = layout.sp(CHASSIS_LEFT, CHASSIS_BOT).y;
    let chassis_rounding = layout.sc(CHASSIS_ROUNDING);
    let panel_left = layout.sp(TOP_PANEL_LEFT, 0.0).x;
    let panel_right = layout.sp(TOP_PANEL_RIGHT, 0.0).x;
    let panel_top = layout.sp(0.0, TOP_PANEL_TOP).y;
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

/// Collects the cosmetic line along the foot. It is flat at each side and
/// steps up on a diagonal to the run between them.
fn collect_cosmetic_bottom(out: &mut ShapeList, layout: &UiScale) {
    let cosbot_stroke = Stroke::new(layout.sc(COSBOT_STROKE), COL_CHASSIS);
    let chassis_left = layout.sp(CHASSIS_LEFT, CHASSIS_TOP).x;
    let chassis_right = layout.sp(CHASSIS_RIGHT, CHASSIS_TOP).x;
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

/// Collects the inner decorative line: the run on the path, then a parallel
/// run [`DECOR_GAP`] to the left of the direction of travel.
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

/// Top of the recess the screen sits in, and the foot of the strip below it,
/// in reference units. `View > Extend Screen` grows the panel to fill exactly
/// this band, so the extended size and the decoration it stops against share
/// one definition.
pub(super) fn lcd_decor_band() -> (f32, f32) {
    (LCD_MOUTH_TOP, LCD_STRIP_BOT)
}

/// The screen's glass in reference units, `(left, right, top, bot)`. It is the
/// same rectangle `draw_top` fills, so the recess walls meet the screen.
fn lcd_glass() -> (f32, f32, f32, f32) {
    let w = layout::TOP_CENTRAL_REF_SIZE_W * layout::LCD_GLASS_WIDTH_FRAC;
    let cx = (layout::TOP_CENTRAL_REF_LEFT + layout::TOP_CENTRAL_REF_RIGHT) * 0.5;
    let top =
        layout::TOP_CENTRAL_REF_TOP + layout::TOP_CENTRAL_REF_SIZE_H * layout::LCD_GLASS_V_TOP;
    (
        cx - w * 0.5,
        cx + w * 0.5,
        top,
        top + w / layout::LCD_GLASS_ASPECT,
    )
}

/// Collects the LCD panel overlay. Call it after all section fills so that
/// they do not cover it.
///
/// Draws (top to bottom):
///   __________     ← navbar rect
///   |--------|
///   |________|     ← bezel top line
///   |\______/|     ← top trapezoid, with diagonals from the outer corners
///   ||      ||     ← outer frame sides and bezel side walls
///   ||______||     ← bezel bottom line
///   |/      \|     ← two bottom diagonals, no center span
///   |________|     ← bottom |__| rectangle
pub(super) fn collect_chassis_lcd_overlay(out: &mut ShapeList, layout: &UiScale) {
    let costop_stroke = Stroke::new(layout.sc(COSTOP_STROKE), COL_CHASSIS);
    let menubar_left = layout::TOP_CENTRAL_REF_LEFT;
    let menubar_right = layout::TOP_CENTRAL_REF_RIGHT;
    let menubar_top = layout::TOP_CENTRAL_REF_TOP;
    // The recess: its mouth spans the zone, its floor is the glass, and the
    // walls between them form the trapezoid.
    let (glass_left, glass_right, trap_top, glass_bot) = lcd_glass();
    let lcd_ref_left = glass_left;
    let lcd_ref_right = glass_right;
    let lcd_ref_top = LCD_MOUTH_TOP;
    let lcd_ref_bot = glass_bot;
    let outer_bot = LCD_MOUTH_BOT;
    let rect_bot = LCD_STRIP_BOT;
    out.add(Shape::line(
        vec![
            layout.sp(menubar_left, menubar_top),
            layout.sp(menubar_right, menubar_top),
            layout.sp(menubar_right, lcd_ref_top),
            layout.sp(menubar_left, lcd_ref_top),
            layout.sp(menubar_left, menubar_top),
        ],
        costop_stroke,
    ));
    for [(x0, y0), (x1, y1)] in [
        [(menubar_left, lcd_ref_top), (menubar_left, outer_bot)],
        [(menubar_right, lcd_ref_top), (menubar_right, outer_bot)],
        [(menubar_left, lcd_ref_top), (lcd_ref_left, trap_top)],
        [(menubar_right, lcd_ref_top), (lcd_ref_right, trap_top)],
        [(lcd_ref_left, trap_top), (lcd_ref_right, trap_top)],
        [(lcd_ref_left, trap_top), (lcd_ref_left, lcd_ref_bot)],
        [(lcd_ref_right, trap_top), (lcd_ref_right, lcd_ref_bot)],
        [(lcd_ref_left, lcd_ref_bot), (lcd_ref_right, lcd_ref_bot)],
    ] {
        out.line_segment([layout.sp(x0, y0), layout.sp(x1, y1)], costop_stroke);
    }
    // The bezel is the glass outline pushed out by `LCD_BEZEL_GAP`; together
    // with the glass edge it draws the screen's double border.
    out.add(Shape::line(
        vec![
            layout.sp(lcd_ref_left - LCD_BEZEL_GAP, trap_top - LCD_BEZEL_GAP),
            layout.sp(lcd_ref_right + LCD_BEZEL_GAP, trap_top - LCD_BEZEL_GAP),
            layout.sp(lcd_ref_right + LCD_BEZEL_GAP, lcd_ref_bot + LCD_BEZEL_GAP),
            layout.sp(lcd_ref_left - LCD_BEZEL_GAP, lcd_ref_bot + LCD_BEZEL_GAP),
            layout.sp(lcd_ref_left - LCD_BEZEL_GAP, trap_top - LCD_BEZEL_GAP),
        ],
        costop_stroke,
    ));
    for [(x0, y0), (x1, y1)] in [
        [(lcd_ref_left, lcd_ref_bot), (menubar_left, outer_bot)],
        [(lcd_ref_right, lcd_ref_bot), (menubar_right, outer_bot)],
    ] {
        out.line_segment([layout.sp(x0, y0), layout.sp(x1, y1)], costop_stroke);
    }
    out.add(Shape::line(
        vec![
            layout.sp(menubar_left, outer_bot),
            layout.sp(menubar_left, rect_bot),
            layout.sp(menubar_right, rect_bot),
            layout.sp(menubar_right, outer_bot),
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
