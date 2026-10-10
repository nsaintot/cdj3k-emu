//! The tray under the lower body: its back wall, floor and side walls seen
//! through its mouth, lit by the slot lamp, and the corners where they meet.

use egui::{Color32, Pos2, Rect, Shape, Stroke, Vec2};

use crate::app::ui::draw_cache::ShapeList;
use crate::app::ui::tilt::{Face, Tilt, P3};
use crate::app::ui::{collect_usb_trident, UiScale, COL_BLACK, COL_BTN_TEXT, COL_SILVER};

use super::{lower, rect, Projector, BAND_H, COL_LOWER, EDGE, SETBACK};
use crate::app::ui::slate::cdj1500x::REF_H;
use crate::app::CdjApp;
use cdj3k_emu_panel::mosi_frame::LedPart;
use cdj3k_emu_panel::Lamp;

// ── The tray ──────────────────────────────────────────────────────────────────
pub(super) const TRAY_L: f32 = 654.0;
pub(super) const TRAY_R: f32 = 1780.0;
pub(super) const TRAY_H: f32 = 300.0;
const TRAY_DEPTH: f32 = 420.0;
/// The ports on the tray's left wall: along the wall (back from the opening)
/// and down it.
const PORT_C: (f32, f32, f32, f32) = (110.0, 70.0, 230.0, 110.0);
const PORT_A: (f32, f32, f32, f32) = (110.0, 170.0, 260.0, 225.0);
/// The USB trident printed on the tray's floor: its centre (across from the
/// left wall, back from the mouth), and its size against the CDJ-3000's.
const FLOOR_MARK: (f32, f32) = (255.0, 45.0);
const FLOOR_MARK_SCALE: f32 = 1.36;
/// The mouth's corners.
pub(super) const TRAY_CORNER_R: f32 = 30.0;

/// Share of the lower body's finish the tray shows, darkening with depth: at
/// the mouth and at the back.
const SHADE_MOUTH: f32 = 1.0;
const SHADE_DEEP: f32 = 0.4;
/// The slot lamp's light in the tray: a strip along the floor's front edge,
/// its glow spreading back across the floor and faintly up the walls.
const GLOW_EDGE: f32 = 0.55;
const GLOW_FLOOR: f32 = 0.3;
const GLOW_FLOOR_REACH: f32 = 0.7;
const GLOW_WALL: f32 = 0.12;
const GLOW_EDGE_W: f32 = 5.0;
/// How tall the mouth stands on screen, in points, before the light inside
/// shows at full strength; edge-on it shows none.
const GLOW_OPEN_PTS: f32 = 6.0;
const COL_TRAY_EDGE: Color32 = Color32::from_rgb(74, 74, 82);

/// `c` with its channels scaled by `k`, opaque.
fn scaled(c: Color32, k: f32) -> Color32 {
    let f = |v: u8| (v as f32 * k).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

/// `c` scaled by `k` as light added to what is under it: premultiplied with
/// no alpha, so it blends by addition.
fn additive(c: Color32, k: f32) -> Color32 {
    let f = |v: u8| (v as f32 * k).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_premultiplied(f(c.r()), f(c.g()), f(c.b()), 0)
}

/// A rectangle shaded from `near` to `far`: down it when `down`, else
/// across it.
fn gradient(r: Rect, near: Color32, far: Color32, down: bool) -> egui::Mesh {
    let mut m = egui::Mesh::default();
    let (a, b) = if down {
        (
            [r.left_top(), r.right_top()],
            [r.right_bottom(), r.left_bottom()],
        )
    } else {
        (
            [r.left_top(), r.left_bottom()],
            [r.right_bottom(), r.right_top()],
        )
    };
    m.colored_vertex(a[0], near);
    m.colored_vertex(a[1], near);
    m.colored_vertex(b[0], far);
    m.colored_vertex(b[1], far);
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    m
}

/// The tray's light: the colour the deck drives the casing's cavity light.
fn tray_light(app: &CdjApp) -> Option<Color32> {
    let mosi = app.mosi();
    let (r, g, b) = mosi.rgb(Lamp::Slot1)?;
    mosi.led_color(LedPart::Slot, r, g, b)
}

/// The tray's light at the mouth: the lamp's colour, and how far the mouth
/// stands open on screen.
pub(super) struct TrayGlow {
    light: Option<Color32>,
    open: f32,
}

impl TrayGlow {
    /// `mouth_clip` is the mouth's bounds on screen.
    pub(super) fn new(app: &CdjApp, mouth_clip: Rect) -> Self {
        Self {
            light: tray_light(app),
            open: ((mouth_clip.height() - 1.0) / GLOW_OPEN_PTS).clamp(0.0, 1.0),
        }
    }

    /// The lamp's light at strength `k`, as light added to the surface.
    fn at(&self, k: f32) -> Color32 {
        self.light
            .map_or(Color32::TRANSPARENT, |c| additive(c, k * self.open))
    }
}

/// The lower body's finish at shade `k`.
fn body(k: f32) -> Color32 {
    scaled(COL_LOWER, k)
}

/// The tray's inside shows only through its mouth: the mouth's bounds on screen.
pub(super) fn mouth_clip(tilt: &Tilt) -> Rect {
    [
        (TRAY_L, 0.0),
        (TRAY_R, 0.0),
        (TRAY_R, TRAY_H),
        (TRAY_L, TRAY_H),
    ]
    .iter()
    .fold(Rect::NOTHING, |r, &(u, v)| {
        r.union(Rect::from_pos(tilt.project(lower().at(u, v))))
    })
}

/// The tray, through its opening: back wall, floor, side walls, then the
/// corners where they meet.
pub(super) fn collect_tray(app: &mut CdjApp, pr: &Projector, glow: &TrayGlow) -> Vec<Shape> {
    let mut shapes = Vec::new();
    shapes.extend(back_wall(app, pr, glow));
    shapes.extend(floor(app, pr, glow));
    for (x, ports) in [(TRAY_L, true), (TRAY_R, false)] {
        shapes.extend(side_wall(app, pr, glow, x, ports));
    }
    shapes.extend(corners(pr.layout, pr.tilt));
    shapes
}

/// The back wall: deep, so dark; lit faintly from its foot.
fn back_wall(app: &mut CdjApp, pr: &Projector, glow: &TrayGlow) -> Vec<Shape> {
    let tray_w = TRAY_R - TRAY_L;
    let back_y = REF_H - SETBACK - TRAY_DEPTH;
    let back_wall = Face {
        origin: [TRAY_L, back_y, -BAND_H],
        u_axis: [1.0, 0.0, 0.0],
        v_axis: [0.0, 0.0, -1.0],
    };
    let mut l = ShapeList::default();
    let r = Rect::from_min_size(Pos2::ZERO, Vec2::new(tray_w, TRAY_H));
    l.rect_filled(r, 0.0, body(SHADE_DEEP));
    l.add(Shape::mesh(gradient(
        r,
        glow.at(0.0),
        glow.at(GLOW_WALL),
        true,
    )));
    pr.project(&mut app.tilt_cache, back_wall, l)
}

/// The floor: darkening back from the mouth, the lamp's strip along its
/// front edge and its glow spreading back.
fn floor(app: &mut CdjApp, pr: &Projector, glow: &TrayGlow) -> Vec<Shape> {
    let tray_w = TRAY_R - TRAY_L;
    let mouth_y = REF_H - SETBACK;
    let floor_z = -(BAND_H + TRAY_H);
    let mut l = ShapeList::default();
    let r = Rect::from_min_size(Pos2::ZERO, Vec2::new(tray_w, TRAY_DEPTH));
    l.add(Shape::mesh(gradient(
        r,
        body(SHADE_MOUTH),
        body(SHADE_DEEP),
        true,
    )));
    let lit = Rect::from_min_size(Pos2::ZERO, Vec2::new(tray_w, TRAY_DEPTH * GLOW_FLOOR_REACH));
    l.add(Shape::mesh(gradient(
        lit,
        glow.at(GLOW_FLOOR),
        glow.at(0.0),
        true,
    )));
    // The floor's units run back from the mouth; the trident's top is
    // toward the back.
    let (cx, cy) = FLOOR_MARK;
    let k = FLOOR_MARK_SCALE;
    collect_usb_trident(
        &mut l,
        |x, y| Pos2::new(cx + (x - cx) * k, cy - (y - cy) * k),
        |v| v * k,
        FLOOR_MARK,
        COL_BTN_TEXT,
    );
    l.rect_filled(
        Rect::from_min_size(Pos2::ZERO, Vec2::new(tray_w, GLOW_EDGE_W)),
        0.0,
        glow.at(GLOW_EDGE),
    );
    pr.project(
        &mut app.tilt_cache,
        Face {
            origin: [TRAY_L, mouth_y, floor_z],
            u_axis: [1.0, 0.0, 0.0],
            v_axis: [0.0, -1.0, 0.0],
        },
        l,
    )
}

/// A side wall: darkening back from the mouth, lit faintly from the floor;
/// the left one carries the ports.
fn side_wall(app: &mut CdjApp, pr: &Projector, glow: &TrayGlow, x: f32, ports: bool) -> Vec<Shape> {
    let mouth_y = REF_H - SETBACK;
    let mut l = ShapeList::default();
    let r = Rect::from_min_size(Pos2::ZERO, Vec2::new(TRAY_DEPTH, TRAY_H));
    l.add(Shape::mesh(gradient(
        r,
        body(SHADE_MOUTH * 0.8),
        body(SHADE_DEEP),
        false,
    )));
    l.add(Shape::mesh(gradient(
        r,
        glow.at(0.0),
        glow.at(GLOW_WALL),
        true,
    )));
    if ports {
        let edge = Stroke::new(EDGE, COL_SILVER);
        let c = rect(PORT_C);
        l.rect_filled(c, c.height() * 0.5, COL_BLACK);
        l.rect_stroke(c, c.height() * 0.5, edge);
        let a = rect(PORT_A);
        l.rect_filled(a, 4.0, COL_BLACK);
        l.rect_stroke(a, 4.0, edge);
    }
    pr.project(
        &mut app.tilt_cache,
        Face {
            origin: [x, mouth_y, -BAND_H],
            u_axis: [0.0, -1.0, 0.0],
            v_axis: [0.0, 0.0, -1.0],
        },
        l,
    )
}

/// The tray's corners: where its walls meet the floor and its ceiling,
/// running back from the mouth, and the back wall's edge.
fn corners(layout: &UiScale, tilt: &Tilt) -> Vec<Shape> {
    let mouth_y = REF_H - SETBACK;
    let back_y = mouth_y - TRAY_DEPTH;
    let floor_z = -(BAND_H + TRAY_H);
    let mut shapes = Vec::new();
    let edge = Stroke::new(layout.sc(EDGE), COL_TRAY_EDGE);
    let line = |a: P3, b: P3| Shape::line_segment([tilt.project(a), tilt.project(b)], edge);
    for x in [TRAY_L, TRAY_R] {
        for z in [-BAND_H, floor_z] {
            shapes.push(line([x, mouth_y, z], [x, back_y, z]));
        }
        shapes.push(line([x, back_y, -BAND_H], [x, back_y, floor_z]));
    }
    for z in [-BAND_H, floor_z] {
        shapes.push(line([TRAY_L, back_y, z], [TRAY_R, back_y, z]));
    }
    shapes
}
