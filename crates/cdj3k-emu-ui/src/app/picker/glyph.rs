//! The deck miniatures on the cards: the riser decks drawn from their
//! [`Glyph`] description, and the CDJ-1500X's flat cabinet. The fractions
//! below are measured off AlphaTheta's outline drawings.

use egui::{Color32, Pos2, Rect, Shape, Stroke, Vec2};

use crate::app::theme::Palette;
use crate::app::ui::slate::card::{Bay, Glyph};

// Deck glyph: cabinet, screen in its frame, hot-cue pads, USB bay, jog and
// browse pod. Nothing touches anything; the outline and the jog carry the
// weight and everything else is drawn a shade finer.
const GLYPH_STROKE: f32 = 1.3;
const GLYPH_DETAIL_STROKE: f32 = 0.9;
/// The cabinet's corner radius, in reference units: the chassis' own, which is
/// slight - at this size all but sharp.
const GLYPH_CHASSIS_ROUND_REF: f32 = 25.0;
/// Segments per rounded corner of the cabinet silhouette.
const GLYPH_CORNER_SEGS: usize = 5;
const GLYPH_SCREEN_ROUND: f32 = 1.5;
const GLYPH_FRAME_ROUND: f32 = 1.0;
/// Hot cues, A to H.
const GLYPH_PAD_COUNT: usize = 8;

// Where the shared controls sit inside their own column, measured off
// AlphaTheta's CDJ-3000X outline drawing. Both decks give their columns the
// same size, so one set of fractions places them on either.
/// CUE and PLAY: their centre across the transport column, their diameter
/// over its width, and the two heights they sit at down it. The drawing puts
/// them at 350 x 348 reference units - round, not square - and all but
/// touching; drawn a little under that, and further apart, they read as two
/// keys at 91 px rather than a figure of eight.
const KEY_CX: f32 = 0.5288;
const KEY_D: f32 = 0.60;
const KEY_CY: (f32, f32) = (0.815, 0.945);

/// The tempo fader, down the modes column.
const FADER: Rect = Rect::from_min_max(Pos2::new(0.2835, 0.6566), Pos2::new(0.7427, 0.9604));

// The media bay, also in transport-column fractions. The parts keep the
// proportions the drawings give them - the CDJ-3000's slot 212 x 194 against
// its 365 x 222 SD, the CDJ-3000X's two slots 228 x 223 - but the bay is centred in
// its column and spaced for a miniature 91 px wide, where the panel's own
// margins close up into a blot in the corner.
const BAY_TOP: f32 = 0.030;
/// CDJ-3000: the slot, the round stop off its bottom-right corner, the SD
/// under the pair. The panel leaves 20 reference units between slot and stop,
/// which is half a pixel here - the two strokes merge - so the stop is set
/// out clear of it.
const BAY_USB: Rect = Rect::from_min_max(Pos2::new(0.1000, BAY_TOP), Pos2::new(0.5000, 0.0775));
/// Centre across and down the column, then radius over its width. Its centre
/// sits on the slot's bottom edge.
const BAY_STOP: (f32, f32, f32) = (0.7200, 0.0775, 0.1000);
const BAY_SD: Rect = Rect::from_min_max(Pos2::new(0.1000, 0.1150), Pos2::new(0.8000, 0.1694));
const BAY_SLOTS: [Rect; 2] = [
    Rect::from_min_max(Pos2::new(0.2763, BAY_TOP), Pos2::new(0.7237, 0.0896)),
    Rect::from_min_max(Pos2::new(0.2763, 0.1646), Pos2::new(0.7237, 0.2192)),
];
/// Their stop bars, each just under its own slot.
const BAY_STOPS: [Rect; 2] = [
    Rect::from_min_max(Pos2::new(0.4147, 0.1016), Pos2::new(0.5853, 0.1098)),
    Rect::from_min_max(Pos2::new(0.4147, 0.2312), Pos2::new(0.5853, 0.2394)),
];

// Browse pod, in reference units. The block the four arc buttons fill is the
// same height on both decks; its width and the bezel ring around the rotary
// are not - see `Glyph::pod_ring_r`.
const GLYPH_POD_H_REF: f32 = 726.0;
const GLYPH_POD_ROUND_REF: f32 = 45.0;
/// Segments along each button pair's inner edge, which follows the ring.
const GLYPH_POD_ARC_SEGS: usize = 10;
const GLYPH_POD_HUB_R_REF: f32 = 100.0;
/// Reference-canvas height `GLYPH_H_FRAC` is authored against; a taller
/// cabinet draws proportionally taller.
const GLYPH_CANVAS_H: f32 = 4659.0;
/// Reference units trimmed off the cabinet's foot: none, since PLAY sits in
/// the skirt there.
const GLYPH_FOOT_TRIM_REF: f32 = 0.0;

// ── The CDJ-1500X's miniature ─────────────────────────────────────────────────
// Measured off AlphaTheta's own CDJ-1500X outline drawing. The cabinet is a
// plain rectangle - no riser block, the screen assembly is the whole of the
// top - with the tempo fader down the right and CUE and PLAY down the left.
/// Its canvas: the slate's reference units, so the cabinet is drawn at the
/// same units-per-pixel as the riser glyphs.
const FLAT_CANVAS: (f32, f32) = (
    crate::app::ui::slate::cdj1500x::REF_W,
    crate::app::ui::slate::cdj1500x::REF_H,
);
const FLAT_SCREEN_FRAME: (f32, f32, f32, f32) = (0.012, 0.012, 0.988, 0.455);
/// The glass inside the frame. Sized off the CDJ-3000X's, which the
/// vector drawing puts at 217.0 x 134.2 mm - the 10.1-inch panel both decks
/// carry - as the nested frames in this drawing are a pixel apart at its
/// published size.
const FLAT_SCREEN: (f32, f32, f32, f32) = (0.1166, 0.0749, 0.8834, 0.3921);
const FLAT_PADS: (f32, f32, f32, f32) = (0.172, 0.478, 0.833, 0.505);
/// Share of each key's pitch left as the gap, read off the same drawing.
const FLAT_PAD_GAP: f32 = 0.34;
/// Jog: centre as fractions of the outline, radius over its width.
const FLAT_JOG: (f32, f32, f32) = (0.495, 0.755, 0.305);
const FLAT_JOG_HUB: f32 = 0.120;
/// The browse rotary, which stands alone where the CDJ-3000 has a pod.
const FLAT_BROWSE: (f32, f32, f32) = (0.912, 0.570, 0.037);
const FLAT_LOOP: (f32, f32, f32) = (0.086, 0.570, 0.040);
/// Tempo fader, down the right.
const FLAT_TEMPO: (f32, f32, f32, f32) = (0.855, 0.635, 0.955, 0.955);
/// CUE and PLAY, down the left. Same radius, one above the other.
const FLAT_TRANSPORT_X: f32 = 0.100;
const FLAT_TRANSPORT_R: f32 = 0.053;
const FLAT_TRANSPORT_Y: (f32, f32) = (0.845, 0.945);

/// The CDJ-1500X's miniature, on the same foot and units-per-pixel as the
/// riser glyphs: the area holds the tallest cabinet, and this one takes the
/// share its canvas asks for.
pub(super) fn draw_flat_glyph(
    p: &egui::Painter,
    area: Rect,
    k: f32,
    col: Color32,
    detail_col: Color32,
) {
    let u = area.height() / (GLYPH_CANVAS_H - GLYPH_FOOT_TRIM_REF);
    let (w, h) = (FLAT_CANVAS.0 * u, FLAT_CANVAS.1 * u);
    let outline = Rect::from_min_size(
        Pos2::new(area.center().x - w * 0.5, area.bottom() - h),
        Vec2::new(w, h),
    );
    let at = |x: f32, y: f32| {
        Pos2::new(
            outline.left() + x * outline.width(),
            outline.top() + y * outline.height(),
        )
    };
    let place = |r: (f32, f32, f32, f32)| Rect::from_min_max(at(r.0, r.1), at(r.2, r.3));

    let stroke = Stroke::new(GLYPH_STROKE * k, col);
    let detail = Stroke::new(GLYPH_DETAIL_STROKE * k, detail_col);

    p.rect_stroke(
        outline,
        GLYPH_CHASSIS_ROUND_REF * k * 0.05,
        stroke,
        egui::StrokeKind::Middle,
    );
    p.rect_stroke(
        place(FLAT_SCREEN_FRAME),
        GLYPH_FRAME_ROUND * k,
        detail,
        egui::StrokeKind::Middle,
    );
    p.rect_filled(place(FLAT_SCREEN), GLYPH_SCREEN_ROUND * k, col);
    button_row(p, place(FLAT_PADS), GLYPH_PAD_COUNT, FLAT_PAD_GAP, col, k);
    p.rect_stroke(
        place(FLAT_TEMPO),
        GLYPH_FRAME_ROUND * k,
        detail,
        egui::StrokeKind::Middle,
    );

    let (jx, jy, jr) = FLAT_JOG;
    let jog_c = at(jx, jy);
    p.circle_stroke(jog_c, jr * outline.width(), stroke);
    p.circle_filled(jog_c, FLAT_JOG_HUB * outline.width(), col);
    for (x, y, r) in [FLAT_BROWSE, FLAT_LOOP] {
        p.circle_stroke(at(x, y), r * outline.width(), detail);
    }
    for y in [FLAT_TRANSPORT_Y.0, FLAT_TRANSPORT_Y.1] {
        p.circle_stroke(
            at(FLAT_TRANSPORT_X, y),
            FLAT_TRANSPORT_R * outline.width(),
            detail,
        );
    }
}

/// A riser glyph's outline in glyph pixels. Horizontal fractions are of the
/// outline's width; vertical ones are of the whole reference canvas, not of
/// the outline, so trimming the foot leaves everything above it where it was.
struct Outline {
    rect: Rect,
    /// The reference canvas's height in pixels.
    vh: f32,
    /// Glyph pixels per reference unit, shared by both models, so a part given
    /// in reference units comes out the same size on either card and the
    /// taller cabinet is the one that fills the area.
    u: f32,
}

impl Outline {
    fn new(area: Rect, g: &Glyph) -> Self {
        let u = area.height() / (GLYPH_CANVAS_H - GLYPH_FOOT_TRIM_REF);
        let vh = g.canvas.1 * u;
        let foot = area.bottom();
        let rect = Rect::from_min_max(
            Pos2::new(
                area.center().x - g.canvas.0 * u * 0.5,
                foot - vh + GLYPH_FOOT_TRIM_REF * u,
            ),
            Pos2::new(area.center().x + g.canvas.0 * u * 0.5, foot),
        );
        Self { rect, vh, u }
    }

    fn at(&self, x: f32, y: f32) -> Pos2 {
        Pos2::new(
            self.rect.left() + x * self.rect.width(),
            self.rect.top() + y * self.vh,
        )
    }

    fn place(&self, r: Rect) -> Rect {
        Rect::from_min_max(self.at(r.left(), r.top()), self.at(r.right(), r.bottom()))
    }
}

/// Draw the model's miniature into `area`, standing on its foot.
pub(super) fn draw_deck_glyph(
    p: &egui::Painter,
    area: Rect,
    g: &Glyph,
    pal: &Palette,
    fill: Color32,
    k: f32,
) {
    let o = Outline::new(area, g);
    // The cabinet and the jog carry the weight; everything else is a shade
    // finer. The deck's colour is on the card's cap, not in here.
    let col = pal.glyph;
    let stroke = Stroke::new(GLYPH_STROKE * k, col);
    let detail = Stroke::new(GLYPH_DETAIL_STROKE * k, pal.glyph_detail);

    draw_cabinet(p, &o, g, stroke);
    draw_screen(p, &o, g, col, detail, k);

    // Hot cues, filled so a row this small still reads as eight of them, in
    // the block the panel puts them in.
    button_row(
        p,
        Rect::from_min_max(o.at(g.pads_x.0, g.pads_y.0), o.at(g.pads_x.1, g.pads_y.1)),
        GLYPH_PAD_COUNT,
        g.pad_gap,
        col,
        k,
    );

    draw_columns(p, &o, g, detail, k);

    let (jx, jy, jr) = g.jog;
    let jog_c = o.at(jx, jy);
    let jog_r = jr * o.rect.width();
    p.circle_stroke(jog_c, jog_r, stroke);
    p.circle_filled(jog_c, jog_r * g.jog_hub, col);

    draw_pod(p, &o, g, detail, fill, col);
}

/// The cabinet: the body, with the screen's block standing above it. The two
/// corners at the block's foot turn inward and are left sharp, as the chassis
/// draws them.
fn draw_cabinet(p: &egui::Painter, o: &Outline, g: &Glyph, stroke: Stroke) {
    let outline = o.rect;
    let body_top = outline.top() + g.body_top * o.vh;
    let (riser_l, riser_r) = (
        outline.left() + g.riser.0 * outline.width(),
        outline.left() + g.riser.1 * outline.width(),
    );
    let r = GLYPH_CHASSIS_ROUND_REF * o.u;
    p.add(Shape::closed_line(
        rounded_polygon(&[
            (Pos2::new(outline.left(), body_top), r),
            (Pos2::new(riser_l, body_top), 0.0),
            (Pos2::new(riser_l, outline.top()), r),
            (Pos2::new(riser_r, outline.top()), r),
            (Pos2::new(riser_r, body_top), 0.0),
            (Pos2::new(outline.right(), body_top), r),
            (outline.right_bottom(), r),
            (outline.left_bottom(), r),
        ]),
        stroke,
    ));
}

/// The screen, and the frame it sits in.
fn draw_screen(p: &egui::Painter, o: &Outline, g: &Glyph, col: Color32, detail: Stroke, k: f32) {
    let screen = o.place(g.screen);
    let frame = screen.expand2(Vec2::new(g.bezel.x * o.rect.width(), g.bezel.y * o.vh));
    p.rect_stroke(
        frame,
        GLYPH_FRAME_ROUND * k,
        detail,
        egui::StrokeKind::Middle,
    );
    if g.recessed {
        for (wall, floor) in [
            (frame.left_top(), screen.left_top()),
            (frame.right_top(), screen.right_top()),
            (frame.right_bottom(), screen.right_bottom()),
            (frame.left_bottom(), screen.left_bottom()),
        ] {
            p.line_segment([wall, floor], detail);
        }
    }
    p.rect_filled(screen, GLYPH_SCREEN_ROUND * k, col);
}

/// A point given in fractions of `col`.
fn in_col(col: Rect, x: f32, y: f32) -> Pos2 {
    Pos2::new(col.left() + x * col.width(), col.top() + y * col.height())
}

/// A rect given in fractions of `col`.
fn cell(col: Rect, r: Rect) -> Rect {
    Rect::from_min_max(
        in_col(col, r.left(), r.top()),
        in_col(col, r.right(), r.bottom()),
    )
}

/// What the two columns carry: a pair of keys at the head of the left one,
/// CUE and PLAY at its foot, and the tempo fader down the right. These are
/// what tell a CDJ-3000 from the CDJ-1500X beside it; the two 3000s carry
/// them alike.
fn draw_columns(p: &egui::Painter, o: &Outline, g: &Glyph, detail: Stroke, k: f32) {
    let tcol = o.place(g.transport_col);
    let mcol = o.place(g.modes_col);
    let key_r = KEY_D * tcol.width() * 0.5;
    for cy in [KEY_CY.0, KEY_CY.1] {
        p.circle_stroke(in_col(tcol, KEY_CX, cy), key_r, detail);
    }
    draw_bay(p, tcol, &g.bay, detail, k);
    p.rect_stroke(
        cell(mcol, FADER),
        GLYPH_FRAME_ROUND * k,
        detail,
        egui::StrokeKind::Middle,
    );
}

/// The media bay, laid out in the transport column rather than traced into
/// the corner the panel puts it in.
fn draw_bay(p: &egui::Painter, tcol: Rect, bay: &Bay, detail: Stroke, k: f32) {
    match bay {
        Bay::UsbAndSd => {
            p.rect_stroke(
                cell(tcol, BAY_USB),
                GLYPH_FRAME_ROUND * k,
                detail,
                egui::StrokeKind::Middle,
            );
            p.circle_stroke(
                in_col(tcol, BAY_STOP.0, BAY_STOP.1),
                BAY_STOP.2 * tcol.width(),
                detail,
            );
            p.rect_stroke(
                cell(tcol, BAY_SD),
                GLYPH_FRAME_ROUND * k,
                detail,
                egui::StrokeKind::Middle,
            );
        }
        Bay::TwinUsb => {
            for (slot, stop) in BAY_SLOTS.iter().zip(BAY_STOPS.iter()) {
                p.rect_stroke(
                    cell(tcol, *slot),
                    GLYPH_FRAME_ROUND * k,
                    detail,
                    egui::StrokeKind::Middle,
                );
                // Filled: a bar this thin closes up as an outline.
                let bar = cell(tcol, *stop);
                p.rect_filled(bar, bar.height() * 0.5, detail.color);
            }
        }
    }
}

/// Browse pod: two arc buttons above the rotary and two below, their inner
/// edge the bezel ring itself, which is why the ring shows between them at
/// the sides - wide of the buttons on the CDJ-3000, flush on the CDJ-3000X.
fn draw_pod(
    p: &egui::Painter,
    o: &Outline,
    g: &Glyph,
    detail: Stroke,
    fill: Color32,
    col: Color32,
) {
    let u = o.u;
    let pod_c = o.at(g.pod.x, g.pod.y);
    let (hw, ring_r) = (g.pod_half_w * u, g.pod_ring_r * u);
    let half_h = GLYPH_POD_H_REF * 0.5 * u;
    let round = GLYPH_POD_ROUND_REF * u;
    if hw < ring_r {
        pod_arc_buttons(p, pod_c, Vec2::new(hw, half_h), ring_r, round, detail);
    } else {
        // The ring fits inside the block: one plain outline, and the seam runs
        // from each end down to the ring.
        let pod = Rect::from_center_size(pod_c, Vec2::new(2.0 * hw, 2.0 * half_h));
        p.rect_stroke(pod, round, detail, egui::StrokeKind::Middle);
        for (y0, y1) in [
            (pod.top(), pod_c.y - ring_r),
            (pod_c.y + ring_r, pod.bottom()),
        ] {
            p.line_segment([Pos2::new(pod_c.x, y0), Pos2::new(pod_c.x, y1)], detail);
        }
    }
    // Filled first: the button block's own edges run in behind the ring, and
    // the rotary stands on top of them.
    p.circle_filled(pod_c, ring_r, fill);
    p.circle_stroke(pod_c, ring_r, detail);
    p.circle_filled(pod_c, GLYPH_POD_HUB_R_REF * u, col);
}

/// The ring stands wider than the buttons, so each pair's inner edge is the
/// ring itself and no cut face crosses it. `half` is the button block's half
/// width and half height.
fn pod_arc_buttons(
    p: &egui::Painter,
    pod_c: Pos2,
    half: Vec2,
    ring_r: f32,
    round: f32,
    detail: Stroke,
) {
    let (hw, half_h) = (half.x, half.y);
    let dy = (ring_r * ring_r - hw * hw).sqrt();
    let a = dy.atan2(hw);
    let pi = std::f32::consts::PI;
    for (edge, join, arc) in [
        (pod_c.y - half_h, pod_c.y - dy, (-a, a - pi)),
        (pod_c.y + half_h, pod_c.y + dy, (a, pi - a)),
    ] {
        let mut pts = rounded_polygon(&[
            (Pos2::new(pod_c.x - hw, join), 0.0),
            (Pos2::new(pod_c.x - hw, edge), round),
            (Pos2::new(pod_c.x + hw, edge), round),
            (Pos2::new(pod_c.x + hw, join), 0.0),
        ]);
        for i in 0..=GLYPH_POD_ARC_SEGS {
            let t = i as f32 / GLYPH_POD_ARC_SEGS as f32;
            pts.push(pod_c + Vec2::angled(arc.0 + (arc.1 - arc.0) * t) * ring_r);
        }
        p.add(Shape::closed_line(pts, detail));
        p.line_segment([Pos2::new(pod_c.x, edge), Pos2::new(pod_c.x, join)], detail);
    }
}

/// The hot-cue strip: `count` keys filling `rect`, filled because an outline
/// this small would close up.
fn button_row(p: &egui::Painter, rect: Rect, count: usize, gap: f32, col: Color32, k: f32) {
    let pitch = rect.width() / count as f32;
    let w = pitch * (1.0 - gap);
    for i in 0..count {
        p.rect_filled(
            Rect::from_min_size(
                Pos2::new(rect.left() + i as f32 * pitch, rect.top()),
                Vec2::new(w, rect.height()),
            ),
            GLYPH_SCREEN_ROUND * k * 0.5,
            col,
        );
    }
}

/// Round each corner of `pts` by its own radius - a quadratic Bezier through
/// the corner, which at this size is indistinguishable from a fillet and needs
/// no centres. A radius of zero leaves the corner sharp.
fn rounded_polygon(pts: &[(Pos2, f32)]) -> Vec<Pos2> {
    let n = pts.len();
    let mut out = Vec::with_capacity(n * (GLYPH_CORNER_SEGS + 1));
    for i in 0..n {
        let (v, r) = pts[i];
        if r <= 0.0 {
            out.push(v);
            continue;
        }
        let (prev, next) = (pts[(i + n - 1) % n].0, pts[(i + 1) % n].0);
        let (to_prev, to_next) = (prev - v, next - v);
        let d = r.min(to_prev.length() * 0.5).min(to_next.length() * 0.5);
        let a = v + to_prev.normalized() * d;
        let b = v + to_next.normalized() * d;
        for seg in 0..=GLYPH_CORNER_SEGS {
            let t = seg as f32 / GLYPH_CORNER_SEGS as f32;
            let u = 1.0 - t;
            out.push(Pos2::new(
                u * u * a.x + 2.0 * u * t * v.x + t * t * b.x,
                u * u * a.y + 2.0 * u * t * v.y + t * t * b.y,
            ));
        }
    }
    out
}
