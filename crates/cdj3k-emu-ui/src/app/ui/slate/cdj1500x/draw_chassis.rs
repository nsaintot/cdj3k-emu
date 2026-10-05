//! CDJ-1500X cabinet outline: the silhouette, the rear lip, the screen slab
//! and the housing's front edge. The bottom-most layer, behind every control.

use std::f32::consts::PI;

use egui::{Color32, Pos2, Rect, Shape, Stroke};

use crate::app::ui::{draw_cache::ShapeList, COL_BTN_TEXT, COL_SILVER};

use super::layout::{
    px, CABINET_R, HOUSING_FRONT, LIP_BOT, LIP_INSET, SLAB, SLAB_BEVEL, SLAB_BEVEL_STROKE, SLAB_R,
};
use super::{UiScale, REF_H, REF_W};

const CHASSIS_STROKE: f32 = 10.0;
const LIP_STROKE: f32 = 3.0;
const SLAB_STROKE: f32 = 4.0;
const COL_CHASSIS: Color32 = COL_SILVER;

const CORNER_SEGS: usize = 12;

/// Collect the cabinet outline into `out` (cached; rebuilt on resize).
pub(super) fn collect_chassis(out: &mut ShapeList, layout: &UiScale) {
    let half = CHASSIS_STROKE * 0.5;
    let cabinet = Rect::from_min_max(Pos2::new(half, half), Pos2::new(REF_W - half, REF_H - half));
    out.add(Shape::closed_line(
        rounded_rect(layout, cabinet, CABINET_R),
        Stroke::new(layout.sc(CHASSIS_STROKE), COL_CHASSIS),
    ));

    // Rear lip: its sides, the line along its foot, and the housing's
    // corners rounding off under it to the outline.
    let lip = Stroke::new(layout.sc(LIP_STROKE), COL_CHASSIS);
    for x in [LIP_INSET, REF_W - LIP_INSET] {
        out.add(Shape::line_segment(
            [layout.sp(x, cabinet.top()), layout.sp(x, LIP_BOT)],
            lip,
        ));
    }
    let mut foot = Vec::with_capacity(2 * CORNER_SEGS + 2);
    push_arc(
        &mut foot,
        layout,
        Pos2::new(cabinet.left() + CABINET_R, LIP_BOT + CABINET_R),
        CABINET_R,
        PI,
        1.5 * PI,
    );
    push_arc(
        &mut foot,
        layout,
        Pos2::new(cabinet.right() - CABINET_R, LIP_BOT + CABINET_R),
        CABINET_R,
        1.5 * PI,
        2.0 * PI,
    );
    out.add(Shape::line(foot, lip));

    // The screen slab and its bevel.
    out.add(Shape::closed_line(
        rounded_rect(layout, SLAB, SLAB_R),
        Stroke::new(layout.sc(SLAB_STROKE), COL_CHASSIS),
    ));
    out.add(Shape::closed_line(
        rounded_rect(
            layout,
            SLAB.shrink(SLAB_BEVEL),
            (SLAB_R - SLAB_BEVEL).max(0.0),
        ),
        Stroke::new(layout.sc(SLAB_BEVEL_STROKE), COL_CHASSIS),
    ));

    // The housing's front edge, out to the outline either side of the slab.
    for (a, b) in [
        (cabinet.left(), SLAB.left()),
        (SLAB.right(), cabinet.right()),
    ] {
        out.add(Shape::line_segment(
            [layout.sp(a, HOUSING_FRONT), layout.sp(b, HOUSING_FRONT)],
            lip,
        ));
    }
}

/// A rounded rectangle's outline, clockwise from the top-left corner's end.
fn rounded_rect(layout: &UiScale, r: Rect, radius: f32) -> Vec<Pos2> {
    let mut pts = Vec::with_capacity(4 * (CORNER_SEGS + 1));
    let c = |x: f32, y: f32| Pos2::new(x, y);
    push_arc(
        &mut pts,
        layout,
        c(r.right() - radius, r.top() + radius),
        radius,
        1.5 * PI,
        2.0 * PI,
    );
    push_arc(
        &mut pts,
        layout,
        c(r.right() - radius, r.bottom() - radius),
        radius,
        0.0,
        0.5 * PI,
    );
    push_arc(
        &mut pts,
        layout,
        c(r.left() + radius, r.bottom() - radius),
        radius,
        0.5 * PI,
        PI,
    );
    push_arc(
        &mut pts,
        layout,
        c(r.left() + radius, r.top() + radius),
        radius,
        PI,
        1.5 * PI,
    );
    pts
}

/// Push an arc about `center` from `start` to `end` radians, both ends
/// included, in screen space.
fn push_arc(
    pts: &mut Vec<Pos2>,
    layout: &UiScale,
    center: Pos2,
    radius: f32,
    start: f32,
    end: f32,
) {
    for i in 0..=CORNER_SEGS {
        let a = start + (end - start) * i as f32 / CORNER_SEGS as f32;
        pts.push(layout.sp(center.x + radius * a.cos(), center.y + radius * a.sin()));
    }
}

/// The model name printed under the tempo fader.
const BADGE: (f32, f32, f32, f32) = (1231.0, 2071.0, 1378.0, 2091.0);

/// The printed model name (cached with the other captions).
pub(super) fn collect_badge(out: &mut ShapeList, ctx: &egui::Context, layout: &UiScale) {
    let (l, t, r, b) = BADGE;
    out.text_in_ink_box(
        ctx,
        Rect::from_min_max(layout.sp(px(l), px(t)), layout.sp(px(r), px(b))),
        "CDJ-1500X",
        egui::FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS_BOLD.into()),
        COL_BTN_TEXT,
    );
}
