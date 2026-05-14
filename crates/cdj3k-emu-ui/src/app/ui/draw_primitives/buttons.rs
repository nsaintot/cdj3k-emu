//! Button-shape collectors used by the per-frame button cache: rectangular,
//! circular, arc-quadrant, hot-cue, and inset-fill variants.

use std::f32::consts::PI;

use egui::epaint::PathShape;
use egui::{Align2, Color32, FontId, Pos2, Rect, Shape, Stroke, Vec2};

use crate::app::ui::draw_cache::ShapeList;

use super::super::{COL_BTN_HOT, COL_BTN_TEXT, COL_SILVER};
use super::*;

pub(in crate::app) fn collect_button_basic(
    out: &mut ShapeList,
    ctx: &egui::Context,
    rect: Rect,
    label: &str,
    font_size: f32,
    font_color: Option<Color32>,
    bg_color: Option<Color32>,
    touchdown_color: Option<Color32>,
    border: DoubleBorderSpec,
    label_touchdown_color: Option<Color32>,
    label_nudge: Option<Vec2>,
    font_family: egui::FontFamily,
    is_pressed: bool,
) {
    let fill = if is_pressed {
        touchdown_color.unwrap_or(COL_BTN_HOT)
    } else {
        bg_color.unwrap_or(COL_BTN)
    };
    let text_color = if is_pressed {
        label_touchdown_color.unwrap_or(font_color.unwrap_or(COL_BTN_TEXT))
    } else {
        font_color.unwrap_or(COL_BTN_TEXT)
    };
    let rounding = (rect.height() * 0.15).max(1.0);
    out.rect_filled(rect, rounding, fill);
    collect_double_rect_border(out, rect, Some(rounding), border);
    let label_pos = rect.center() + label_nudge.unwrap_or(Vec2::ZERO);
    let font = FontId::new(font_size, font_family);
    if label.chars().count() == 1 {
        out.text_centered_ink(ctx, label_pos, label, font, text_color);
    } else {
        out.text(
            ctx,
            label_pos,
            Align2::CENTER_CENTER,
            label,
            font,
            text_color,
        );
    }
}

/// Collect an inset-fill rectangular button into `out`.  The double
/// border still wraps the full `rect`, but the colored fill is drawn in
/// an inner rect that envelops the label glyph (with `pad_h` / `pad_v`
/// pixel padding on each axis).  Used by buttons whose colored fill is
/// supposed to read as a label highlight, not a full button background.
pub(in crate::app) fn collect_button_inset_fill(
    out: &mut ShapeList,
    ctx: &egui::Context,
    rect: Rect,
    label: &str,
    font_size: f32,
    font_color: Option<Color32>,
    bg_color: Option<Color32>,
    touchdown_color: Option<Color32>,
    border: DoubleBorderSpec,
    label_touchdown_color: Option<Color32>,
    label_nudge: Option<Vec2>,
    font_family: egui::FontFamily,
    is_pressed: bool,
    pad_h: f32,
    pad_v: f32,
) {
    let fill = if is_pressed {
        touchdown_color.unwrap_or(COL_BTN_HOT)
    } else {
        bg_color.unwrap_or(COL_BTN)
    };
    let text_color = if is_pressed {
        label_touchdown_color.unwrap_or(font_color.unwrap_or(COL_BTN_TEXT))
    } else {
        font_color.unwrap_or(COL_BTN_TEXT)
    };

    // Measure the label so the inset rect tracks the actual glyph box.
    let font = FontId::new(font_size, font_family.clone());
    let galley = ctx.fonts(|f| f.layout_no_wrap(label.to_owned(), font.clone(), text_color));
    let label_pos = rect.center() + label_nudge.unwrap_or(Vec2::ZERO);
    let glyph_size = galley.size();
    let inset_size = Vec2::new(
        (glyph_size.x + 2.0 * pad_h).min(rect.width()),
        (glyph_size.y + 2.0 * pad_v).min(rect.height()),
    );
    let inset_rect = Rect::from_center_size(label_pos, inset_size);
    let inset_rounding = (inset_rect.height() * 0.15).max(1.0);
    out.rect_filled(inset_rect, inset_rounding, fill);

    // Outer rect keeps the original size + double border.
    let outer_rounding = (rect.height() * 0.15).max(1.0);
    collect_double_rect_border(out, rect, Some(outer_rounding), border);

    if label.chars().count() == 1 {
        out.text_centered_ink(ctx, label_pos, label, font, text_color);
    } else {
        out.text(
            ctx,
            label_pos,
            Align2::CENTER_CENTER,
            label,
            font,
            text_color,
        );
    }
}

/// Collect a hot-cue style button into `out`. No interaction - caller provides `is_pressed`.
pub(in crate::app) fn collect_button_hotcue(
    out: &mut ShapeList,
    ctx: &egui::Context,
    rect: Rect,
    label: &str,
    font_size: f32,
    font_color: Option<Color32>,
    bg_color: Option<Color32>,
    touchdown_color: Option<Color32>,
    border: DoubleBorderSpec,
    label_touchdown_color: Option<Color32>,
    label_nudge: Option<Vec2>,
    font_family: egui::FontFamily,
    is_pressed: bool,
) {
    let fill = if is_pressed {
        touchdown_color.unwrap_or(COL_BTN_HOT)
    } else {
        bg_color.unwrap_or(COL_BTN)
    };
    let accent = if is_pressed {
        label_touchdown_color.unwrap_or(font_color.unwrap_or(COL_BTN_TEXT))
    } else {
        font_color.unwrap_or(COL_BTN_TEXT)
    };
    let rounding = (rect.height() * 0.12).max(1.0);
    out.rect_filled(rect, rounding, fill);
    collect_double_rect_border(out, rect, Some(rounding), border);
    out.text(
        ctx,
        Pos2::new(rect.left() + rect.width() * 0.05, rect.top())
            + label_nudge.unwrap_or(Vec2::ZERO),
        Align2::LEFT_TOP,
        label,
        FontId::new(font_size, font_family),
        accent,
    );
    let line_y = rect.bottom() - rect.height() * 0.15;
    let line_x0 = rect.left() + rect.width() * 0.06;
    let line_x1 = rect.right() - rect.width() * 0.06;
    let thickness = (rect.height() * 0.07).max(1.5);
    out.line_segment(
        [Pos2::new(line_x0, line_y), Pos2::new(line_x1, line_y)],
        Stroke::new(thickness, accent),
    );
}

/// Collect shapes for a button dispatching on `button_type`. No interaction.
pub(in crate::app) fn collect_button(
    out: &mut ShapeList,
    ctx: &egui::Context,
    button_type: ButtonType,
    rect: Rect,
    label: &str,
    font_size: f32,
    font_color: Option<Color32>,
    bg_color: Option<Color32>,
    touchdown_color: Option<Color32>,
    border: DoubleBorderSpec,
    label_touchdown_color: Option<Color32>,
    label_nudge: Option<Vec2>,
    font_family: egui::FontFamily,
    is_pressed: bool,
) {
    match button_type {
        ButtonType::Basic => collect_button_basic(
            out,
            ctx,
            rect,
            label,
            font_size,
            font_color,
            bg_color,
            touchdown_color,
            border,
            label_touchdown_color,
            label_nudge,
            font_family,
            is_pressed,
        ),
        ButtonType::HotCue => collect_button_hotcue(
            out,
            ctx,
            rect,
            label,
            font_size,
            font_color,
            bg_color,
            touchdown_color,
            border,
            label_touchdown_color,
            label_nudge,
            font_family,
            is_pressed,
        ),
        ButtonType::InsetFill { pad_h, pad_v } => collect_button_inset_fill(
            out,
            ctx,
            rect,
            label,
            font_size,
            font_color,
            bg_color,
            touchdown_color,
            border,
            label_touchdown_color,
            label_nudge,
            font_family,
            is_pressed,
            pad_h,
            pad_v,
        ),
    }
}

/// Collect a circle button into `out`. No interaction - caller provides `is_pressed`.
pub(in crate::app) fn collect_circle_button(
    out: &mut ShapeList,
    ctx: &egui::Context,
    center: Pos2,
    radius: f32,
    btn_color: Option<Color32>,
    touchdown_color: Option<Color32>,
    btn_label: &str,
    btn_label_size: f32,
    btn_label_color: Option<Color32>,
    btn_label_touchdown_color: Option<Color32>,
    btn_label_nudge: Option<Vec2>,
    border: DoubleBorderSpec,
    is_pressed: bool,
) {
    let base_fill = if is_pressed {
        touchdown_color.unwrap_or(COL_BTN_HOT)
    } else {
        btn_color.unwrap_or(COL_SILVER)
    };
    let base_label = btn_label_color.unwrap_or(COL_BTN_TEXT);
    let label_color = if is_pressed {
        btn_label_touchdown_color.unwrap_or(base_label)
    } else {
        base_label
    };
    out.circle_filled(center, radius, base_fill);
    paint_double_circle_ring_collect(out, center, radius, border);
    let label_pos = center + btn_label_nudge.unwrap_or(Vec2::ZERO);
    let font = FontId::proportional(btn_label_size);
    if btn_label.chars().count() == 1 {
        out.text_centered_ink(ctx, label_pos, btn_label, font, label_color);
    } else {
        out.text(
            ctx,
            label_pos,
            Align2::CENTER_CENTER,
            btn_label,
            font,
            label_color,
        );
    }
}

/// Collect an arc-quad button into `out`. No interaction - caller provides `is_pressed`.
pub(in crate::app) fn collect_arc_quad_button(
    out: &mut ShapeList,
    ctx: &egui::Context,
    center: Pos2,
    inner_r: f32,
    outer_r: f32,
    a_start: f32,
    a_end: f32,
    fill: Color32,
    press_fill: Color32,
    border: DoubleBorderSpec,
    label: &str,
    font_size: f32,
    font_color: Color32,
    label_nudge: Vec2,
    line_height: Option<f32>,
    font_family: egui::FontFamily,
    dec_arc: Option<ArcDecorSpec>,
    notch_arc: Option<ArcNotchSpec>,
    is_pressed: bool,
) {
    let cx = center.x;
    let cy = center.y;
    let upper = ((a_start + a_end) * 0.5).sin() < 0.0;
    let angle_at = |inner_angle: f32, r: f32| -> f32 {
        let x = inner_r * inner_angle.cos();
        let y_sq = r * r - x * x;
        if y_sq <= 0.0 {
            return if x < 0.0 { PI } else { 0.0 };
        }
        let y = if upper { -y_sq.sqrt() } else { y_sq.sqrt() };
        y.atan2(x)
    };
    let a_outer_at_start = angle_at(a_start, outer_r);
    let a_outer_at_end = angle_at(a_end, outer_r);
    let arc = |r: f32, s: f32, e: f32| -> Vec<Pos2> {
        (0..=ARC_SEGMENTS)
            .map(|i| {
                let t = i as f32 / ARC_SEGMENTS as f32;
                let a = s + (e - s) * t;
                Pos2::new(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect()
    };
    let mut pts = arc(inner_r, a_start, a_end);
    pts.extend(arc(outer_r, a_outer_at_end, a_outer_at_start));
    let actual_fill = if is_pressed { press_fill } else { fill };
    let outer_stroke_w = border.inner.width + border.gap * 2.0 + border.outer.width;
    out.add(Shape::Path(PathShape {
        points: pts.clone(),
        closed: true,
        fill: Color32::TRANSPARENT,
        stroke: PathStroke::new(outer_stroke_w, border.outer.color),
    }));
    out.add(Shape::Path(PathShape {
        points: pts,
        closed: true,
        fill: actual_fill,
        stroke: PathStroke::new(border.inner.width, border.inner.color),
    }));
    let a_mid = (a_start + a_end) * 0.5;
    let x_mid = inner_r * a_mid.cos();
    let y_inner_mid = inner_r * a_mid.sin();
    let y_outer_sq = (outer_r * outer_r - x_mid * x_mid).max(0.0);
    let y_outer_mid = if upper {
        -y_outer_sq.sqrt()
    } else {
        y_outer_sq.sqrt()
    };
    let label_pos = Pos2::new(cx + x_mid, cy + (y_inner_mid + y_outer_mid) * 0.5) + label_nudge;
    let font_id = FontId::new(font_size, font_family);
    if let Some(lh) = line_height {
        use egui::text::LayoutJob;
        use egui::{Align, TextFormat};
        let mut job = LayoutJob::default();
        job.halign = Align::Center;
        job.append(
            label,
            0.0,
            TextFormat {
                font_id,
                color: font_color,
                line_height: Some(lh),
                ..Default::default()
            },
        );
        let galley = ctx.fonts(|f| f.layout_job(job));
        out.add(Shape::galley(
            label_pos - galley.size() * 0.5,
            galley,
            font_color,
        ));
    } else {
        out.text(
            ctx,
            label_pos,
            Align2::CENTER_CENTER,
            label,
            font_id,
            font_color,
        );
    }
    if let Some(dec) = dec_arc {
        let dec_a_start = angle_at(a_start, dec.radius);
        let dec_a_end = angle_at(a_end, dec.radius);
        let arc_pts: Vec<Pos2> = (0..=ARC_SEGMENTS)
            .map(|i| {
                let t = i as f32 / ARC_SEGMENTS as f32;
                let a = dec_a_start + (dec_a_end - dec_a_start) * t;
                Pos2::new(cx + dec.radius * a.cos(), cy + dec.radius * a.sin())
            })
            .collect();
        out.add(Shape::line(arc_pts, dec.stroke));
    }
    if let Some(notch) = notch_arc {
        let notch_center = angle_at(a_mid, notch.radius);
        let notch_bound_start = angle_at(a_start, notch.radius);
        let notch_bound_end = angle_at(a_end, notch.radius);
        let lo = notch_bound_start.min(notch_bound_end);
        let hi = notch_bound_start.max(notch_bound_end);
        let n_start = (notch_center - notch.half_angle).max(lo);
        let n_end = (notch_center + notch.half_angle).min(hi);
        let notch_pts: Vec<Pos2> = (0..=ARC_SEGMENTS)
            .map(|i| {
                let t = i as f32 / ARC_SEGMENTS as f32;
                let a = n_start + (n_end - n_start) * t;
                Pos2::new(cx + notch.radius * a.cos(), cy + notch.radius * a.sin())
            })
            .collect();
        out.add(Shape::line(notch_pts, notch.stroke));
    }
}
