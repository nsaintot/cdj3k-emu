//! The CDJ-1500X's front face, shown when the deck is tilted, and the toggle
//! that tilts it.
//!
//! The face has two steps. The top shell ends in a band that carries the
//! phone pad. Under it, the lower body stands [`SETBACK`] further back and
//! narrows to its feet; it carries EJECT, the USB legends and the tray
//! opening. The tray glows with the casing's cavity light, and its two ports
//! sit on its left wall, out of view from the front.
//!
//! The setback is the overhang that hides the USB 1 legend when the deck is
//! tilted by 42 degrees. Depths into the tray are estimates.

use std::f32::consts::{FRAC_PI_2, PI};

use cdj3k_emu_panel::{Btn, Lamp};
use egui::{Color32, FontFamily, Pos2, Rect, Shape, Stroke, Vec2};

use crate::app::ui::draw_cache::ShapeList;
use crate::app::ui::tilt::{self, Face, Tilt, P3};
use crate::app::ui::{UiScale, COL_BLACK, COL_BTN, COL_BTN_TEXT, COL_DARK, COL_SILVER, COL_WHITE};
use crate::app::CdjApp;

use super::layout::CABINET_R;
use super::{REF_H, REF_W};

mod tray;

use tray::{TRAY_CORNER_R, TRAY_H, TRAY_L, TRAY_R};

/// The tilt angle, the camera distance and the duration of the turn.
pub(super) const TILT_ANGLE: f32 = 42.0 * std::f32::consts::PI / 180.0;
pub(super) const CAM_DIST: f32 = 2.5 * REF_H;
pub(super) const TILT_SECS: f32 = 0.6;
/// The screen housing rises from its front line toward the back: the
/// cabinet is 116.5 mm tall at the back and about 67 mm at the front. At
/// this angle the tilted deck shows a screen-to-panel depth ratio of 0.764.
pub(super) const HOUSING_ANGLE: f32 = 8.5 * std::f32::consts::PI / 180.0;

// ── The band, on the face under the top's front edge ──────────────────────────
const BAND_H: f32 = 311.0;
/// The phone pad: glossy, with corner brackets, a phone and a lamp bar.
const PAD: (f32, f32, f32, f32) = (2050.0, 70.0, 2228.0, 230.0);

// ── The lower body ────────────────────────────────────────────────────────────
const SETBACK: f32 = 35.0;
const LOWER_H: f32 = 336.0;
/// How far the lower body's sides sit inside the cabinet's sides, at the top
/// and at the foot.
const LOWER_INSET_TOP: f32 = 82.0;
const LOWER_INSET_BOT: f32 = 225.0;
/// The lower body's sides drop straight this far before the chamfer turns
/// them in.
const LOWER_STRAIGHT: f32 = 74.0;
const LOWER_FOOT_R: f32 = 90.0;
/// EJECT on the lower body's face, measured from the face's top edge.
const EJECT: (f32, f32, f32, f32) = (341.0, 57.0, 489.0, 139.0);
/// The USB legends' left edge and centres, and their size; USB 1 sits over
/// USB 2, as the ports do on the tray's wall.
const USB_LEGEND_X: f32 = 538.0;
const USB_1_Y: f32 = 31.0;
const USB_2_Y: f32 = 101.0;
const USB_LEGEND_SIZE: f32 = 34.0;

// ── Finishes ──────────────────────────────────────────────────────────────────
const COL_BAND: Color32 = Color32::from_rgb(24, 24, 29);
const COL_LOWER: Color32 = Color32::from_rgb(40, 40, 45);
const EDGE: f32 = 3.0;

// ── The toggle ────────────────────────────────────────────────────────────────
/// Centre and radius of the tilt toggle, on the panel under the jog; it
/// rides the deck as it tilts.
const TOGGLE_CENTER: (f32, f32) = (REF_W * 0.5, REF_H - 62.0);
const TOGGLE_R: f32 = 40.0;

/// Where the panel's surface bends, and how far it turns there.
pub(super) fn crease() -> (f32, f32) {
    (super::layout::HOUSING_FRONT, HOUSING_ANGLE)
}

fn band() -> Face {
    Face {
        origin: [0.0, REF_H, 0.0],
        u_axis: [1.0, 0.0, 0.0],
        v_axis: [0.0, 0.0, -1.0],
    }
}

fn lower() -> Face {
    Face {
        origin: [0.0, REF_H - SETBACK, -BAND_H],
        u_axis: [1.0, 0.0, 0.0],
        v_axis: [0.0, 0.0, -1.0],
    }
}

/// The deck's outermost points, which the tilt fits to the canvas.
pub(super) fn extent() -> [P3; 6] {
    let foot = -(BAND_H + LOWER_H);
    let back = crease().0 * HOUSING_ANGLE.tan();
    [
        [0.0, 0.0, back],
        [REF_W, 0.0, back],
        [0.0, REF_H, 0.0],
        [REF_W, REF_H, 0.0],
        [LOWER_INSET_BOT, REF_H - SETBACK, foot],
        [REF_W - LOWER_INSET_BOT, REF_H - SETBACK, foot],
    ]
}

/// Projects the deck's faces onto the screen for the current frame.
struct Projector<'a> {
    ctx: &'a egui::Context,
    layout: &'a UiScale,
    tilt: &'a Tilt,
}

impl Projector<'_> {
    fn project(&self, cache: &mut tilt::TiltCache, face: Face, list: ShapeList) -> Vec<Shape> {
        tilt::project_face(
            self.ctx,
            self.layout,
            self.tilt,
            face,
            list.into_shapes(),
            cache,
        )
    }
}

/// Draws the front face under `tilt` and handles EJECT.
pub(super) fn draw_front(
    app: &mut CdjApp,
    ui: &mut egui::Ui,
    p: &egui::Painter,
    layout: &UiScale,
    tilt: &Tilt,
) {
    let ctx = ui.ctx().clone();
    let pr = Projector {
        ctx: &ctx,
        layout,
        tilt,
    };
    let mouth_clip = tray::mouth_clip(tilt);
    let glow = tray::TrayGlow::new(app, mouth_clip);
    let mut shapes = tray::collect_tray(app, &pr, &glow);
    let interior = shapes.len();
    shapes.extend(lower_face(app, &pr));
    shapes.extend(band_face(app, &pr));
    app.frame_shape_count += shapes.len() as u64;
    // No part of the face shows above the top surface's front edge. When the
    // deck is nearly flat, the camera looks down past that edge, and the face,
    // which hangs below it, projects behind the top surface.
    let edge_y = tilt.project([REF_W * 0.5, REF_H, 0.0]).y;
    let below_edge = p.clip_rect().intersect(Rect::everything_below(edge_y));
    let rest = shapes.split_off(interior);
    p.with_clip_rect(mouth_clip.intersect(below_edge))
        .extend(shapes);
    p.with_clip_rect(below_edge).extend(rest);

    eject_interact(app, ui, tilt, edge_y);
}

/// Draws the lower body's face around the tray opening.
fn lower_face(app: &mut CdjApp, pr: &Projector) -> Vec<Shape> {
    let mut l = ShapeList::default();
    let outline = lower_outline();
    let edge = Stroke::new(EDGE, COL_SILVER);
    for piece in [
        clip(&outline, true, TRAY_L, true),
        clip(&outline, true, TRAY_R, false),
        clip(
            &clip(&clip(&outline, true, TRAY_L, false), true, TRAY_R, true),
            false,
            TRAY_H,
            false,
        ),
    ] {
        if piece.len() >= 3 {
            l.add(Shape::convex_polygon(piece, COL_LOWER, Stroke::NONE));
        }
    }
    let mouth = Rect::from_min_max(Pos2::new(TRAY_L, 0.0), Pos2::new(TRAY_R, TRAY_H));
    let r = TRAY_CORNER_R;
    collect_mouth_corners(&mut l, mouth);
    l.add(Shape::closed_line(outline, edge));
    l.rect_stroke(mouth, r, edge);
    let sans = FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS.into());
    for (text, y) in [("USB 1", USB_1_Y), ("USB 2", USB_2_Y)] {
        l.text(
            pr.ctx,
            Pos2::new(USB_LEGEND_X, y),
            egui::Align2::LEFT_CENTER,
            text,
            egui::FontId::new(USB_LEGEND_SIZE, sans.clone()),
            COL_BTN_TEXT,
        );
    }
    let held = app
        .button(Btn::UsbStop)
        .is_some_and(|b| app.held_btn == Some(b) || app.latched_btns.contains(&b));
    collect_eject(&mut l, pr.ctx, held, app.mosi().drive(Lamp::Eject));
    pr.project(&mut app.tilt_cache, lower(), l)
}

/// Fills the face in to the mouth's rounded corners.
fn collect_mouth_corners(l: &mut ShapeList, mouth: Rect) {
    let r = TRAY_CORNER_R;
    for (corner, c, a0) in [
        (mouth.left_top(), mouth.left_top() + Vec2::splat(r), PI),
        (
            mouth.right_top(),
            mouth.right_top() + Vec2::new(-r, r),
            1.5 * PI,
        ),
        (
            mouth.right_bottom(),
            mouth.right_bottom() - Vec2::splat(r),
            0.0,
        ),
        (
            mouth.left_bottom(),
            mouth.left_bottom() + Vec2::new(r, -r),
            FRAC_PI_2,
        ),
    ] {
        let mut fan = egui::Mesh::default();
        fan.colored_vertex(corner, COL_LOWER);
        for i in 0..=8 {
            let a = a0 + FRAC_PI_2 * i as f32 / 8.0;
            fan.colored_vertex(c + Vec2::angled(a) * r, COL_LOWER);
            if i > 0 {
                fan.add_triangle(0, i, i + 1);
            }
        }
        l.add(Shape::mesh(fan));
    }
}

/// Draws the band above the lower body. It is flat between the cabinet's
/// corners and wraps around them.
fn band_face(app: &mut CdjApp, pr: &Projector) -> Vec<Shape> {
    let r = CABINET_R;
    let mut l = ShapeList::default();
    let face = Rect::from_min_max(Pos2::new(r, 0.0), Pos2::new(REF_W - r, BAND_H));
    l.rect_filled(face, 0.0, COL_BAND);
    l.line_segment(
        [face.left_bottom(), face.right_bottom()],
        Stroke::new(EDGE, COL_SILVER),
    );
    collect_pad(&mut l);
    let mut shapes = pr.project(&mut app.tilt_cache, band(), l);
    shapes.extend(band_corners(pr.layout, pr.tilt));
    shapes
}

/// Hit-tests EJECT on the raw pointer, because the panel's pointer is mapped
/// onto the top surface. `edge_y` is the screen y of the top surface's front
/// edge; no part of the face shows above it.
fn eject_interact(app: &mut CdjApp, ui: &egui::Ui, tilt: &Tilt, edge_y: f32) {
    let quad: Vec<Pos2> = [
        (EJECT.0, EJECT.1),
        (EJECT.2, EJECT.1),
        (EJECT.2, EJECT.3),
        (EJECT.0, EJECT.3),
    ]
    .iter()
    .map(|&(u, v)| tilt.project(lower().at(u, v)))
    .collect();
    let raw = app.raw_pointer;
    let over = raw
        .pos
        .is_some_and(|pos| pos.y >= edge_y && inside(&quad, pos));
    let ctrl = ui.input(|i| i.modifiers.ctrl);
    if let Some(bit) = app.button(Btn::UsbStop) {
        let held = app.held_btn == Some(bit);
        app.handle_btn_interaction(raw.down && (over || held), ctrl, bit);
    }
}

/// Returns the lower body's outline on its face. The sides drop straight,
/// then a chamfer turns them in toward the foot, and the foot's corners are
/// rounded.
fn lower_outline() -> Vec<Pos2> {
    let segs = 8;
    let (h, r, v) = (LOWER_H, LOWER_FOOT_R, LOWER_STRAIGHT);
    let (top, foot) = (REF_W - LOWER_INSET_TOP, REF_W - LOWER_INSET_BOT);
    // The right chamfer runs from (top, v) to (foot, h); the round is
    // tangent to it and to the foot.
    let len = ((h - v) * (h - v) + (top - foot) * (top - foot)).sqrt();
    let a_side = (top - foot).atan2(h - v);
    let cx = top + (-r * len - (top - foot) * (h - r - v)) / (h - v);
    let mut pts = vec![
        Pos2::new(LOWER_INSET_TOP, 0.0),
        Pos2::new(top, 0.0),
        Pos2::new(top, v),
    ];
    let mut arc = |cx: f32, a0: f32, a1: f32| {
        for i in 0..=segs {
            let a = a0 + (a1 - a0) * i as f32 / segs as f32;
            pts.push(Pos2::new(cx + r * a.cos(), h - r + r * a.sin()));
        }
    };
    arc(cx, a_side, FRAC_PI_2);
    arc(REF_W - cx, FRAC_PI_2, PI - a_side);
    pts.push(Pos2::new(LOWER_INSET_TOP, v));
    pts
}

/// Draws the band where it wraps the cabinet's rounded corners: a quarter
/// cylinder on each side, with its bottom edge and outer silhouette stroked.
fn band_corners(layout: &UiScale, tilt: &Tilt) -> Vec<Shape> {
    let segs = 10;
    let r = CABINET_R;
    let edge = Stroke::new(layout.sc(EDGE), COL_SILVER);
    let mut out = Vec::new();
    for (cx, a0, a1, outer) in [(r, FRAC_PI_2, PI, PI), (REF_W - r, FRAC_PI_2, 0.0, 0.0)] {
        let at = |a: f32, z: f32| tilt.project([cx + r * a.cos(), REF_H - r + r * a.sin(), z]);
        let mut mesh = egui::Mesh::default();
        let mut bottom = Vec::with_capacity(segs + 1);
        for i in 0..=segs {
            let a = a0 + (a1 - a0) * i as f32 / segs as f32;
            mesh.colored_vertex(at(a, 0.0), COL_BAND);
            mesh.colored_vertex(at(a, -BAND_H), COL_BAND);
            bottom.push(at(a, -BAND_H));
            if i > 0 {
                let k = 2 * i as u32;
                mesh.add_triangle(k - 2, k - 1, k);
                mesh.add_triangle(k - 1, k + 1, k);
            }
        }
        out.push(Shape::mesh(mesh));
        out.push(Shape::line(bottom, edge));
        out.push(Shape::line_segment(
            [at(outer, 0.0), at(outer, -BAND_H)],
            edge,
        ));
    }
    out
}

/// Clips the convex `poly` to one side of a line. The line is `x = at` when
/// `across` is set and `y = at` otherwise; the kept side is below `at` when
/// `below` is set.
fn clip(poly: &[Pos2], across: bool, at: f32, below: bool) -> Vec<Pos2> {
    let side = |p: Pos2| {
        let d = if across { p.x } else { p.y } - at;
        if below {
            -d
        } else {
            d
        }
    };
    let mut out = Vec::with_capacity(poly.len() + 2);
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        let (sp, sq) = (side(p), side(q));
        if sp >= 0.0 {
            out.push(p);
        }
        if (sp >= 0.0) != (sq >= 0.0) {
            out.push(p.lerp(q, sp / (sp - sq)));
        }
    }
    out
}

fn rect((l, t, r, b): (f32, f32, f32, f32)) -> Rect {
    Rect::from_min_max(Pos2::new(l, t), Pos2::new(r, b))
}

fn inside(poly: &[Pos2], p: Pos2) -> bool {
    let n = poly.len();
    (0..n).all(|i| {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        (b - a).x * (p - a).y - (b - a).y * (p - a).x >= 0.0
    })
}

/// Draws EJECT, a small key in a recess with its name on the cap. The name is
/// lit at `drive`, from 0 (dark) to 255 (full).
fn collect_eject(l: &mut ShapeList, ctx: &egui::Context, held: bool, drive: u8) {
    let key = rect(EJECT);
    l.rect_filled(key, 8.0, COL_BLACK);
    let cap = key.shrink(7.0);
    l.rect_filled(cap, 5.0, if held { COL_BTN } else { COL_DARK });
    l.rect_stroke(cap, 5.0, Stroke::new(2.0, COL_SILVER));
    let ink = Rect::from_center_size(cap.center(), Vec2::new(cap.width() * 0.72, 22.0));
    l.text_in_ink_box(
        ctx,
        ink,
        "EJECT",
        FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS_BOLD.into()),
        if drive == 0 {
            COL_BTN_TEXT
        } else {
            super::key::led(COL_WHITE, drive)
        },
    );
}

/// Draws the phone pad. Its lamp bar is always lit white.
fn collect_pad(l: &mut ShapeList) {
    let pad = rect(PAD);
    l.rect_filled(pad, 12.0, COL_BLACK);
    l.rect_stroke(pad, 12.0, Stroke::new(2.0, COL_SILVER));
    let bracket = Stroke::new(4.0, COL_BTN_TEXT);
    let inner = pad.shrink(16.0);
    let arm = 22.0;
    for (corner, dx, dy) in [
        (inner.left_top(), 1.0, 1.0),
        (inner.right_top(), -1.0, 1.0),
        (inner.right_bottom(), -1.0, -1.0),
        (inner.left_bottom(), 1.0, -1.0),
    ] {
        l.line_segment([corner, corner + Vec2::new(dx * arm, 0.0)], bracket);
        l.line_segment([corner, corner + Vec2::new(0.0, dy * arm)], bracket);
    }
    let phone = Rect::from_center_size(pad.center() - Vec2::new(0.0, 8.0), Vec2::new(44.0, 72.0));
    l.rect_stroke(phone, 8.0, Stroke::new(4.0, COL_BTN_TEXT));
    l.circle_stroke(phone.center(), 10.0, Stroke::new(3.0, COL_BTN_TEXT));
    let lamp = Rect::from_center_size(
        Pos2::new(pad.center().x, inner.bottom() - 4.0),
        Vec2::new(26.0, 6.0),
    );
    l.rect_filled(lamp, 3.0, COL_WHITE);
}

/// Draws the tilt toggle, a disc with a double chevron that points down to
/// tilt the deck and up to bring it back. It is hit-tested on the raw
/// pointer, which the tilt does not remap. Returns whether it was clicked.
pub(super) fn draw_toggle(
    app: &mut CdjApp,
    p: &egui::Painter,
    layout: &UiScale,
    tilt: Option<&Tilt>,
    t: f32,
) -> bool {
    let (x, y) = TOGGLE_CENTER;
    let (c, r) = match tilt {
        Some(tilt) => {
            let c = tilt.project([x, y, 0.0]);
            let edge = tilt.project([x + TOGGLE_R, y, 0.0]);
            (c, c.distance(edge))
        }
        None => (layout.sp(x, y), layout.sc(TOGGLE_R)),
    };
    let k = r / layout.sc(TOGGLE_R);
    let raw = app.raw_pointer;
    let hovered = raw.pos.is_some_and(|pos| pos.distance(c) <= r);
    p.circle_filled(c, r, if hovered { COL_BTN } else { COL_DARK });
    p.circle_stroke(c, r, Stroke::new(layout.sc(2.0) * k, COL_SILVER));
    // Two stacked chevrons, pointing down in plan and turning over to point
    // up as the deck tilts.
    let dir = 1.0 - 2.0 * t;
    let col = if hovered { COL_WHITE } else { COL_BTN_TEXT };
    let s = |v: f32| layout.sc(v) * k;
    let stroke = Stroke::new(s(5.0), col);
    for row in [-1.0, 1.0] {
        let tip = c + Vec2::new(0.0, s(9.0 * row + 6.0 * dir));
        let rise = -s(12.0) * dir;
        let half = s(14.0);
        p.line_segment([tip + Vec2::new(-half, rise), tip], stroke);
        p.line_segment([tip, tip + Vec2::new(half, rise)], stroke);
    }
    hovered && raw.pressed
}
