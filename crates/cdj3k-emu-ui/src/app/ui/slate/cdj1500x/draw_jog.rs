//! The CDJ-1500X jog: a flat platter, a knurled grip sloping down around it,
//! and the hub in the middle with its ring of LEDs.
//!
//! The platter's top is plain; the grip's knurl (raised pyramids lit from the
//! top left) and a white bar on it show it turning. The hub's ring lights the
//! pair of bars at the position the deck sends ([`cdj1500x::jog_ring`]). Drag,
//! scroll and the slingshot are the shared [`CdjApp::jog_interact`].

use std::f32::consts::TAU;

use cdj3k_emu_panel::cdj1500x::{self, JogRing};
use cdj3k_emu_panel::{Lamp, LampState};
use egui::{Color32, Mesh, Pos2, Shape, Stroke, Vec2};

use crate::app::ui::tilt::{Footprint, Lift, Raised};
use crate::app::ui::{UiScale, COL_JOG_BODY, COL_LCD_BG, COL_SILVER, COL_WHITE};
use crate::app::CdjApp;

use super::layout::px;

const CENTER: (f32, f32) = (714.83, 1604.53);

/// Radii from the centre out, drawing pixels.
const HUB_R: f32 = 177.5;
const PLATTER_R: f32 = 364.4;
const KNURL_IN_R: f32 = 379.0;
/// Where the diamonds stop; a plain chamfer runs from here to the knurl's
/// outer edge.
const KNURL_END_R: f32 = 430.0;
const KNURL_OUT_R: f32 = 436.0;
/// The assembly's outer edge.
const RIM_R: f32 = 443.0;

/// Diamonds around the knurl band, and how many deep it is: square at the
/// band's mean radius.
const KNURL_DIAMONDS: usize = 150;
const KNURL_ROWS: usize = 3;
/// Share of its cell a diamond covers; the rest shows the dark groove.
const KNURL_FILL: f32 = 1.0;
/// Share of a diamond its flat top covers: the pyramids are cut off.
const KNURL_TOP: f32 = 0.55;
/// The knurl's faces in soft light: the tops at one shade, the sides a little
/// lighter or darker as they face the top left, all darker towards the outer
/// edge where the grip falls away.
const KNURL_LIGHT: Vec2 = Vec2::new(-0.45, -0.89);
const KNURL_TOP_SHADE: f32 = 0.55;
const KNURL_SIDE_SHADE: f32 = 0.22;
const KNURL_FALLOFF: f32 = 0.15;
const KNURL_CHAMFER_SHADE: f32 = 0.25;
const COL_KNURL_LIT: Color32 = Color32::from_rgb(80, 80, 88);

/// The hub's LED ring: segments around it, between these radii.
const RING_SEGMENTS: usize = 72;
const RING_IN_R: f32 = 110.0;
const RING_OUT_R: f32 = 142.0;
/// Share of each segment's pitch it fills.
const RING_FILL: f32 = 0.4;
/// How many segments light at a position: the deck's 36 positions over the
/// 72 bars.
const RING_LIT: usize = RING_SEGMENTS / cdj1500x::JOG_RING_POSITIONS as usize;
/// The segment at the top, where the deck's position 0 lights: segments run
/// clockwise from 3 o'clock.
const RING_TOP: usize = RING_SEGMENTS * 3 / 4;
const COL_RING_OFF: Color32 = Color32::from_rgb(48, 48, 54);

/// The gap between the knurl and the panel, darker than the panel, and the
/// knurl's grooves.
const COL_RIM_GAP: Color32 = Color32::from_rgb(6, 6, 8);

/// The position bar across the grip.
const BAR_HALF_W: f32 = 5.0;

const HUB_STROKE: f32 = 6.0;
const LINE_STROKE: f32 = 2.0;

pub(super) fn draw_jog(app: &mut CdjApp, ui: &mut egui::Ui, p: &egui::Painter, layout: &UiScale) {
    let center = layout.sp(px(CENTER.0), px(CENTER.1));
    let s = |r: f32| layout.sc(px(r));
    app.jog_interact(ui, layout, center, s(PLATTER_R), s(RIM_R));

    let line = Stroke::new(layout.sc(LINE_STROKE), COL_SILVER);
    // The gap, the chamfer, the grooves' floor, then the platter over them
    // out to where the knurl begins.
    p.add(Shape::circle_filled(center, s(RIM_R), COL_RIM_GAP));
    p.add(Shape::circle_filled(
        center,
        s(KNURL_OUT_R),
        knurl_shade(KNURL_CHAMFER_SHADE),
    ));
    p.add(Shape::circle_filled(center, s(KNURL_END_R), COL_RIM_GAP));
    p.add(Shape::circle_filled(center, s(KNURL_IN_R), COL_JOG_BODY));

    let angle = app.jog_display_angle();
    let (ox, oy, scale) = layout.cache_key();
    let q = |v: f32| (v * 1000.0).round() as i32;
    let key = (q(ox), q(oy), q(scale), angle.to_bits());
    let mut shapes = app
        .knurl_cache
        .get_or_build(key, |list| list.add(Shape::mesh(knurl(center, angle, s))))
        .to_vec();
    shapes.reserve(RING_SEGMENTS + 7);
    // The position bar, across the knurl.
    let dir = Vec2::angled(angle);
    let side = dir.rot90() * s(BAR_HALF_W);
    shapes.push(Shape::convex_polygon(
        vec![
            center + dir * s(KNURL_IN_R) - side,
            center + dir * s(KNURL_OUT_R) - side,
            center + dir * s(KNURL_OUT_R) + side,
            center + dir * s(KNURL_IN_R) + side,
        ],
        COL_WHITE,
        Stroke::NONE,
    ));

    // The circle that divides the touch zone from the grip, a plain band in
    // from the knurl.
    shapes.push(Shape::circle_stroke(center, s(PLATTER_R), line));
    shapes.push(Shape::circle_filled(center, s(HUB_R), COL_LCD_BG));
    shapes.push(Shape::circle_stroke(
        center,
        s(HUB_R),
        Stroke::new(layout.sc(HUB_STROKE), COL_SILVER),
    ));

    // The LED ring, as the deck drives it.
    let ring = match app.mosi().lamp(Lamp::JogMeter) {
        Some(LampState::Level(v)) => cdj1500x::jog_ring(v),
        _ => JogRing::Off,
    };
    let seg = TAU / RING_SEGMENTS as f32;
    let half = seg * RING_FILL * 0.5;
    for i in 0..RING_SEGMENTS {
        let a = seg * (i as f32 + 0.5);
        let lit = match ring {
            JogRing::Off => false,
            JogRing::All => true,
            JogRing::At(p) => {
                let from = RING_TOP + p as usize * RING_LIT;
                (i + RING_SEGMENTS - from % RING_SEGMENTS) % RING_SEGMENTS < RING_LIT
            }
        };
        let (a0, a1) = (Vec2::angled(a - half), Vec2::angled(a + half));
        shapes.push(Shape::convex_polygon(
            vec![
                center + a0 * s(RING_IN_R),
                center + a0 * s(RING_OUT_R),
                center + a1 * s(RING_OUT_R),
                center + a1 * s(RING_IN_R),
            ],
            if lit { COL_WHITE } else { COL_RING_OFF },
            Stroke::NONE,
        ));
    }
    app.frame_shape_count += shapes.len() as u64;
    p.extend(shapes);
}

/// The knurl turned to `angle`: rows of flat-topped pyramids, each row half a
/// diamond out from the last and half a pitch round, the edge rows halved by
/// the band's edges.
fn knurl(center: Pos2, angle: f32, s: impl Fn(f32) -> f32) -> Mesh {
    let pitch = TAU / KNURL_DIAMONDS as f32;
    let depth = (KNURL_END_R - KNURL_IN_R) / KNURL_ROWS as f32;
    let at = |a: f32, r: f32| center + Vec2::angled(a) * s(r);
    let mut mesh = Mesh::default();
    for row in 0..=2 * KNURL_ROWS {
        let r = KNURL_IN_R + depth * 0.5 * row as f32;
        let fall = 1.0 - KNURL_FALLOFF * (r - KNURL_IN_R) / (KNURL_END_R - KNURL_IN_R);
        let (dr, da) = (depth * 0.5 * KNURL_FILL, pitch * 0.5 * KNURL_FILL);
        let r_in = (r - dr).max(KNURL_IN_R);
        let r_out = (r + dr).min(KNURL_END_R);
        for i in 0..KNURL_DIAMONDS {
            let a = angle + pitch * (i as f32 + 0.5 * (row % 2) as f32);
            let apex = at(a, r);
            let (out, inn) = (at(a, r_out), at(a, r_in));
            let (ccw, cw) = (at(a + da, r), at(a - da, r));
            let radial = Vec2::angled(a);
            let round = Vec2::new(-radial.y, radial.x);
            let top = |v: Pos2| apex + (v - apex) * KNURL_TOP;
            let tops = [out, ccw, inn, cw].map(top);
            let mut quad = |q: [Pos2; 4], shade: f32| {
                let col = knurl_shade(shade * fall);
                let k = mesh.vertices.len() as u32;
                for v in q {
                    mesh.colored_vertex(v, col);
                }
                mesh.add_triangle(k, k + 1, k + 2);
                mesh.add_triangle(k, k + 2, k + 3);
            };
            for (v0, v1, n) in [
                (out, ccw, radial + round),
                (ccw, inn, round - radial),
                (inn, cw, -radial - round),
                (cw, out, radial - round),
            ] {
                // An edge row's half beyond the band folds flat onto its apex.
                if v0 == apex || v1 == apex {
                    continue;
                }
                let lit = KNURL_TOP_SHADE + KNURL_SIDE_SHADE * n.normalized().dot(KNURL_LIGHT);
                quad([v0, v1, top(v1), top(v0)], lit);
            }
            quad(tops, KNURL_TOP_SHADE);
        }
    }
    mesh
}

/// The knurl's colour at `shade`, from the groove's dark to fully lit.
fn knurl_shade(shade: f32) -> Color32 {
    let t = shade.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color32::from_rgb(
        mix(COL_RIM_GAP.r(), COL_KNURL_LIT.r()),
        mix(COL_RIM_GAP.g(), COL_KNURL_LIT.g()),
        mix(COL_RIM_GAP.b(), COL_KNURL_LIT.b()),
    )
}

/// How far the platter's face stands above the panel, and the grip's foot:
/// the grip slopes from the one to the other, so seen side on the jog is a
/// trapezoid.
pub(super) const HEIGHT: f32 = 120.0;
const FOOT: f32 = 6.0;

/// The jog, standing off the panel.
pub(super) fn raised() -> Raised {
    let c = Pos2::new(px(CENTER.0), px(CENTER.1));
    Raised {
        shapes: 0..0,
        lift: Lift::Cone {
            center: c,
            r_top: px(KNURL_IN_R),
            r_foot: px(RIM_R),
            height: HEIGHT,
            foot: FOOT,
        },
        // The foot is the gap's side, unoutlined: the panel meets the gap
        // with no edge.
        walls: vec![Footprint {
            outlined: false,
            ..Footprint::upright(super::key::circle(c, px(RIM_R)), FOOT)
        }],
        wall: COL_RIM_GAP,
        ribs: Vec::new(),
        rib: Stroke::NONE,
        facets: Vec::new(),
        tops: vec![(super::key::circle(c, px(KNURL_IN_R)), HEIGHT)],
    }
}
