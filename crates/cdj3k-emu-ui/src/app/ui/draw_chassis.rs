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
// Mirror the LCD geometry from draw_top.rs - must stay in sync if those change.
const LCD_PANEL_WIDTH_FRAC: f32 = 0.85;
const LCD_PANEL_ASPECT: f32 = 1285.0 / 760.0;
const LCD_PANEL_V_TOP: f32 = 0.145;
/// How far the outer frame extends below the LCD bezel (reference units).
const LCD_BORDER_DEPTH: f32 = 42.0;
/// Height of the small rectangle below the bottom diagonals (the |__| part).
const LCD_RECT_DEPTH: f32 = 30.0;

// ── Cosmetic bottom line
const COSBOT_LINE_1_Y: f32 = CHASSIS_BOT - 50.0;
const COSBOT_LINE_2_Y: f32 = CHASSIS_BOT - 90.0;
const COSBOT_LINE_1_W: f32 = 620.0;
const COSBOT_JOINT_ANGLE: f32 = 18.0;

// ── Stroke ────────────────────────────────────────────────────────────────────
const CHASSIS_STROKE: f32 = 10.0;
const COL_CHASSIS: Color32 = COL_SILVER;
const COSTOP_STROKE: f32 = 3.0;
const COSBOT_STROKE: f32 = 2.0;

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
}

/// LCD panel overlay - must be called **after** all section fills so it is not buried under them.
///
/// Draws (top to bottom):
///   __________     ← navbar rect
///   |--------|
///   |________|     ← bezel top line
///   |\______/|     ← top trapezoid: outer corners → diagonal
///   ||      ||     ← outer frame sides + bezel side walls
///   ||______||     ← bezel bottom line
///   |/      \|     ← two bottom diagonals, no center span
///   |________|     ← bottom |__| rectangle
/// Collect the LCD panel overlay into `out` (no-painter variant for caching).
pub(super) fn collect_chassis_lcd_overlay(out: &mut ShapeList, layout: &UiScale) {
    let costop_stroke = Stroke::new(layout.sc(COSTOP_STROKE), COL_CHASSIS);
    let menubar_left = layout::TOP_CENTRAL_REF_LEFT;
    let menubar_right = layout::TOP_CENTRAL_REF_RIGHT;
    let menubar_top = layout::TOP_CENTRAL_REF_TOP;
    let menubar_width = layout::TOP_CENTRAL_REF_SIZE_W;
    let menubar_height = layout::TOP_CENTRAL_REF_SIZE_H;
    let lcd_width_ref = menubar_width * LCD_PANEL_WIDTH_FRAC;
    let lcd_height_ref = lcd_width_ref / LCD_PANEL_ASPECT;
    let lcd_center_x = (menubar_left + menubar_right) * 0.5;
    let lcd_ref_left = lcd_center_x - lcd_width_ref * 0.5;
    let lcd_ref_right = lcd_center_x + lcd_width_ref * 0.5;
    let lcd_ref_top = menubar_top + menubar_height * LCD_PANEL_V_TOP;
    let lcd_ref_bot = lcd_ref_top + lcd_height_ref;
    let trap_top = lcd_ref_top + LCD_BORDER_DEPTH;
    let outer_bot = lcd_ref_bot + LCD_BORDER_DEPTH;
    let rect_bot = outer_bot + LCD_RECT_DEPTH;
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
    out.line_segment(
        [
            layout.sp(menubar_left, lcd_ref_top),
            layout.sp(menubar_left, outer_bot),
        ],
        costop_stroke,
    );
    out.line_segment(
        [
            layout.sp(menubar_right, lcd_ref_top),
            layout.sp(menubar_right, outer_bot),
        ],
        costop_stroke,
    );
    out.line_segment(
        [
            layout.sp(menubar_left, lcd_ref_top),
            layout.sp(lcd_ref_left, trap_top),
        ],
        costop_stroke,
    );
    out.line_segment(
        [
            layout.sp(menubar_right, lcd_ref_top),
            layout.sp(lcd_ref_right, trap_top),
        ],
        costop_stroke,
    );
    out.line_segment(
        [
            layout.sp(lcd_ref_left, trap_top),
            layout.sp(lcd_ref_right, trap_top),
        ],
        costop_stroke,
    );
    out.line_segment(
        [
            layout.sp(lcd_ref_left, trap_top),
            layout.sp(lcd_ref_left, lcd_ref_bot),
        ],
        costop_stroke,
    );
    out.line_segment(
        [
            layout.sp(lcd_ref_right, trap_top),
            layout.sp(lcd_ref_right, lcd_ref_bot),
        ],
        costop_stroke,
    );
    out.line_segment(
        [
            layout.sp(lcd_ref_left, lcd_ref_bot),
            layout.sp(lcd_ref_right, lcd_ref_bot),
        ],
        costop_stroke,
    );
    out.line_segment(
        [
            layout.sp(lcd_ref_left, lcd_ref_bot),
            layout.sp(menubar_left, outer_bot),
        ],
        costop_stroke,
    );
    out.line_segment(
        [
            layout.sp(lcd_ref_right, lcd_ref_bot),
            layout.sp(menubar_right, outer_bot),
        ],
        costop_stroke,
    );
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
