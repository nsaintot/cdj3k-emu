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

// The USB trident, reference units: trunk, arrow, and two forks, one ending in
// a dot and one in a square.
const USB_W: f32 = 70.0;
const USB_STROKE: f32 = 3.5;
const USB_TAIL_DOT_R: f32 = 5.0;
const USB_ARROW_LEN: f32 = 12.0;
const USB_ARROW_HALF: f32 = 6.0;
const USB_FORK_OFF_Y: f32 = 12.0;
/// Fork x positions, as offsets from the trident's centre: the top fork's
/// start, the end of its diagonal and its end; the same for the bottom fork.
const USB_TOP_FORK_X: [f32; 3] = [-22.0, -10.0, 0.0];
const USB_BOT_FORK_X: [f32; 3] = [-18.0, 0.0, 11.0];
const USB_SQUARE_S: f32 = 8.0;
const USB_CIRCLE_R: f32 = 5.0;

/// The USB trident centred on `(cx, cy)` in reference units, placed by `at`
/// and sized by `len` - a plan's mapping to the screen, or a face's own units
/// for a tilt to project.
pub(in crate::app) fn collect_usb_trident(
    list: &mut ShapeList,
    at: impl Fn(f32, f32) -> Pos2,
    len: impl Fn(f32) -> f32,
    (cx, cy): (f32, f32),
    color: Color32,
) {
    let stroke = Stroke::new(len(USB_STROKE), color);

    // Trunk: horizontal line spanning the full width.
    let half_w = USB_W * 0.5;
    list.line_segment(
        [at(cx - half_w, cy), at(cx + half_w - USB_ARROW_LEN, cy)],
        stroke,
    );

    // Tail dot (left end of the trunk).
    list.circle_filled(at(cx - half_w, cy), len(USB_TAIL_DOT_R), color);

    // Arrow head (right end): filled triangle pointing right.
    let arrow_base_x = cx + half_w - USB_ARROW_LEN;
    list.add(Shape::convex_polygon(
        vec![
            at(arrow_base_x, cy - USB_ARROW_HALF),
            at(cx + half_w, cy),
            at(arrow_base_x, cy + USB_ARROW_HALF),
        ],
        color,
        Stroke::NONE,
    ));

    // Upper fork: trunk -> diagonal up -> horizontal -> filled circle.
    let [start, diag_end, end] = USB_TOP_FORK_X.map(|x| cx + x);
    let top_y = cy - USB_FORK_OFF_Y;
    list.line_segment([at(start, cy), at(diag_end, top_y)], stroke);
    list.line_segment([at(diag_end, top_y), at(end, top_y)], stroke);
    list.circle_filled(at(end + USB_CIRCLE_R, top_y), len(USB_CIRCLE_R), color);

    // Lower fork: trunk -> diagonal down -> horizontal -> filled square.
    let [start, diag_end, end] = USB_BOT_FORK_X.map(|x| cx + x);
    let bot_y = cy + USB_FORK_OFF_Y;
    list.line_segment([at(start, cy), at(diag_end, bot_y)], stroke);
    list.line_segment([at(diag_end, bot_y), at(end, bot_y)], stroke);
    let sq = Rect::from_center_size(
        at(end + USB_SQUARE_S * 0.5, bot_y),
        Vec2::new(len(USB_SQUARE_S), len(USB_SQUARE_S)),
    );
    list.rect_filled(sq, 0.0, color);
}
