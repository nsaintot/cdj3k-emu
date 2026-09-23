//! Pictograms drawn as vectors. The UI font has no dependable coverage for
//! them and a missing glyph renders as a box.

use egui::{Color32, Pos2, Rect, Shape, Stroke, Vec2};

use crate::app::ui::{draw_cache::ShapeList, COL_BLACK, COL_DARK};

/// Host-link computer: a stroked screen over a filled trapezoid stand, `w`
/// wide overall at the stand's foot, `center` on the screen. `notch` cuts the
/// stand's centre.
pub(in crate::app) fn collect_computer_glyph(
    out: &mut ShapeList,
    center: Pos2,
    w: f32,
    color: Color32,
    notch: bool,
) {
    let f = |v: f32| v * w / 70.0;
    let screen = Rect::from_center_size(center, Vec2::new(f(37.0), f(20.0)));
    out.rect_filled(screen, 0.0, COL_BLACK);
    out.rect_stroke(screen, 0.0, Stroke::new(f(5.0), color));

    let stand_top = screen.bottom() + f(5.4);
    let stand_bot = stand_top + f(10.0);
    let cx = center.x;
    out.add(Shape::convex_polygon(
        vec![
            Pos2::new(cx - f(22.5), stand_top),
            Pos2::new(cx + f(22.5), stand_top),
            Pos2::new(cx + f(35.0), stand_bot),
            Pos2::new(cx - f(35.0), stand_bot),
        ],
        color,
        Stroke::NONE,
    ));

    if notch {
        out.rect_filled(
            Rect::from_min_max(
                Pos2::new(cx - f(5.0), stand_bot - f(5.0)),
                Pos2::new(cx + f(5.0), stand_bot),
            ),
            0.0,
            COL_DARK,
        );
    }
}
