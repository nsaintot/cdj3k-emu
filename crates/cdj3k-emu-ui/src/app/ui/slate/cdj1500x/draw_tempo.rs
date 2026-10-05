//! The tempo fader: its slot, the track, the cap, and the dotted scale with
//! − / 0 / + down its left.
//!
//! The cap is the CDJ-3000s' - a flange, a ridge across it, the white index
//! line along the ridge - and drawn in their skin; the slot around it and the
//! single track are this deck's own. Tilted, the ridge rises off the flange
//! on coved sides.

use std::f32::consts::FRAC_PI_2;

use egui::{Color32, FontFamily, Pos2, Rect, Sense, Stroke};

use crate::app::ui::draw_cache::ShapeList;
use crate::app::ui::tilt::{Footprint, Lift, Raised, Rib};
use crate::app::ui::{UiScale, COL_BLACK, COL_BTN, COL_BTN_TEXT, COL_DARK, COL_SILVER, COL_WHITE};
use crate::app::CdjApp;

use super::layout::px;

/// The slot's outer edge.
const SLOT: (f32, f32, f32, f32) = (1242.0, 1334.0, 1366.5, 2047.5);
const SLOT_R: f32 = 14.0;

/// The track the cap rides on.
const TRACK: (f32, f32, f32, f32) = (1297.5, 1387.0, 1310.5, 1993.0);

/// The cap at rest: its flange, and the ridge across it, the CDJ-3000s'
/// share of the flange's depth.
const CAP_W: f32 = 102.5;
const CAP_H: f32 = 113.5;
const CAP_X: f32 = 1304.25;
const RIDGE_H_FRAC: f32 = 0.4;
/// The CDJ-3000s' cap skin, reference units: corner round, index line
/// weight, and its length as a share of the cap's width.
const CAP_ROUND: f32 = 2.0;
const INDEX_STROKE: f32 = 10.0;
const INDEX_W_FRAC: f32 = 0.9;

/// Where the cap's centre sits at either end of its travel; the middle is
/// the drawing's rest position, level with the scale's 0.
const TRAVEL_TOP: f32 = 1403.5;
const TRAVEL_BOT: f32 = 1978.0;
/// Within this fraction of the middle, the cap snaps to it.
const CENTER_SNAP_ZONE: f32 = 0.02;

/// The scale: its dots, spaced evenly either side of the 0, and its marks.
const SCALE_X: f32 = 1221.5;
const DOT_R: f32 = 1.75;
const DOTS_ABOVE: (f32, f32, usize) = (1429.0, 1667.0, 16);
const DOTS_BELOW: (f32, f32, usize) = (1713.0, 1967.0, 17);
const MINUS: (f32, f32, f32, f32) = (1216.0, 1406.0, 1229.0, 1409.5);
const ZERO: (f32, f32, f32, f32) = (1217.0, 1683.0, 1227.0, 1698.0);
const PLUS: (f32, f32, f32, f32) = (1216.0, 1975.0, 1229.0, 1981.0);

const SLOT_STROKE: f32 = 4.0;
const LINE_STROKE: f32 = 2.0;
const MARK_STROKE: f32 = 3.0;

fn sbox(layout: &UiScale, (l, t, r, b): (f32, f32, f32, f32)) -> Rect {
    Rect::from_min_max(layout.sp(px(l), px(t)), layout.sp(px(r), px(b)))
}

/// Slot, track and scale (cached; rebuilt on resize).
pub(super) fn collect_tempo_statics(list: &mut ShapeList, ctx: &egui::Context, layout: &UiScale) {
    let slot = sbox(layout, SLOT);
    let r = layout.sc(px(SLOT_R));
    list.rect_filled(slot, r, COL_BLACK);
    list.rect_stroke(slot, r, Stroke::new(layout.sc(SLOT_STROKE), COL_SILVER));

    let track = sbox(layout, TRACK);
    list.rect_filled(track, 0.0, COL_DARK);
    list.rect_stroke(track, 0.0, Stroke::new(layout.sc(LINE_STROKE), COL_SILVER));

    for (first, last, n) in [DOTS_ABOVE, DOTS_BELOW] {
        for i in 0..n {
            let y = first + (last - first) * i as f32 / (n - 1) as f32;
            list.circle_filled(
                layout.sp(px(SCALE_X), px(y)),
                layout.sc(px(DOT_R)),
                COL_BTN_TEXT,
            );
        }
    }
    // − and + are drawn bars, so they read at the drawing's weight.
    let mark = Stroke::new(layout.sc(MARK_STROKE), COL_BTN_TEXT);
    for b in [MINUS, PLUS] {
        let r = sbox(layout, b);
        list.line_segment([r.left_center(), r.right_center()], mark);
    }
    let plus = sbox(layout, PLUS);
    let half = plus.width() * 0.5;
    list.line_segment(
        [
            plus.center() - egui::vec2(0.0, half),
            plus.center() + egui::vec2(0.0, half),
        ],
        mark,
    );
    list.text_in_ink_box(
        ctx,
        sbox(layout, ZERO),
        "0",
        FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS.into()),
        COL_BTN_TEXT,
    );
}

/// The cap and its drag. The flange is the CDJ-3000s' silver flat, turning to
/// the cap's own black as the deck tilts by `t`.
pub(super) fn draw_tempo(
    app: &mut CdjApp,
    ui: &mut egui::Ui,
    p: &egui::Painter,
    layout: &UiScale,
    t: f32,
) {
    let top = layout.sp(0.0, px(TRAVEL_TOP)).y;
    let bot = layout.sp(0.0, px(TRAVEL_BOT)).y;

    let drag = ui.interact(
        sbox(layout, SLOT),
        ui.id().with("tempo_slider"),
        Sense::drag(),
    );
    if drag.dragged() {
        if let Some(ptr) = drag.interact_pointer_pos() {
            let t = ((ptr.y - top) / (bot - top)).clamp(0.0, 1.0);
            let t = if (t - 0.5).abs() < CENTER_SNAP_ZONE {
                0.5
            } else {
                t
            };
            let prev = app.tempo;
            app.tempo = t;
            if (app.tempo - prev).abs() > 1e-4 {
                app.inject_tempo();
            }
        }
    }

    let cap = cap_rect(app.tempo, layout);
    let round = layout.sc(CAP_ROUND);
    let flange = lerp(COL_SILVER, COL_CAP_TILTED, t);
    p.rect_filled(cap, round, flange);
    p.rect_stroke(cap, round, Stroke::new(layout.sc(LINE_STROKE), COL_SILVER));
}

/// The cap's ridge and its index line, drawn after the flange so a tilt can
/// stand them on it.
pub(super) fn draw_tempo_ridge(app: &CdjApp, p: &egui::Painter, layout: &UiScale) {
    let cap = cap_rect(app.tempo, layout);
    let ridge = Rect::from_center_size(
        cap.center(),
        egui::vec2(cap.width(), cap.height() * RIDGE_H_FRAC),
    );
    p.rect_filled(ridge, layout.sc(CAP_ROUND) * 0.5, COL_BTN);
    // The ridge top's front edge, where the cove begins.
    p.line_segment(
        [ridge.left_bottom(), ridge.right_bottom()],
        Stroke::new(layout.sc(LINE_STROKE), COL_SILVER),
    );
    let half = cap.width() * INDEX_W_FRAC * 0.5;
    p.line_segment(
        [
            Pos2::new(cap.center().x - half, cap.center().y),
            Pos2::new(cap.center().x + half, cap.center().y),
        ],
        Stroke::new(layout.sc(INDEX_STROKE), COL_WHITE),
    );
}

/// The cap's centre down the canvas where `tempo` puts it, drawing pixels.
fn cap_y(tempo: f32) -> f32 {
    TRAVEL_TOP + (TRAVEL_BOT - TRAVEL_TOP) * tempo
}

/// The cap's flange on screen.
fn cap_rect(tempo: f32, layout: &UiScale) -> Rect {
    Rect::from_center_size(
        layout.sp(px(CAP_X), px(cap_y(tempo))),
        egui::vec2(layout.sc(px(CAP_W)), layout.sc(px(CAP_H))),
    )
}

/// How far the cap's flange stands off the panel, and its ridge.
const FLANGE_HEIGHT: f32 = 10.0;
pub(super) const HEIGHT: f32 = 95.0;
/// The ridge's front: a quarter-round cove from its top's front edge out to
/// the knob's front extremity, as stacked walls, then an upright skirt
/// [`SKIRT_HEIGHT`] tall down to the panel. Its back drops upright to the
/// flange, which shows behind it.
const COVE_STEPS: usize = 12;
const SKIRT_HEIGHT: f32 = 14.0;
/// The sides take the faces' colours: the flange's silver, shaded, and the
/// ridge's grey - the cove dark where it turns away at either end and
/// lightest across its middle, the skirt dark.
const COL_FLANGE_WALL: Color32 = Color32::from_rgb(22, 22, 26);
/// The cove is darkest where it leaves the top, lightest through its lower
/// middle, and darkens again toward the skirt.
const COVE_SHADE_TOP: f32 = 0.35;
const COVE_SHADE_FOOT: f32 = 0.6;
const COVE_SHADE_MIDDLE: f32 = 1.6;
const SKIRT_SHADE: f32 = 0.55;
const COL_EDGE: Color32 = Color32::from_rgb(78, 78, 86);

/// The cap's black, as the tilted deck shows it.
const COL_CAP_TILTED: Color32 = Color32::from_rgb(30, 30, 35);

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

fn corners(r: Rect) -> Vec<Pos2> {
    vec![
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
    ]
}

/// The flange where `tempo` puts it, reference units.
fn flange(tempo: f32) -> Rect {
    Rect::from_center_size(
        Pos2::new(px(CAP_X), px(cap_y(tempo))),
        egui::vec2(px(CAP_W), px(CAP_H)),
    )
}

/// The cap's flange, standing off the panel: the ledge behind the ridge; in
/// front, the ridge's cove runs down over it to the panel.
pub(super) fn raised_flange(tempo: f32) -> Raised {
    let full = flange(tempo);
    let ridge_back = full.center().y - full.height() * RIDGE_H_FRAC * 0.5;
    let f = corners(Rect::from_min_max(
        full.min,
        Pos2::new(full.right(), ridge_back),
    ));
    Raised {
        shapes: 0..0,
        lift: Lift::Flat(FLANGE_HEIGHT),
        walls: vec![Footprint::upright(f.clone(), FLANGE_HEIGHT)],
        wall: COL_FLANGE_WALL,
        ribs: Vec::new(),
        rib: Stroke::NONE,
        facets: Vec::new(),
        tops: vec![(f, FLANGE_HEIGHT)],
    }
}

/// The ridge, rising off the flange: upright at the back, and at the front a
/// quarter-round cove sweeping from the top's front edge down to the knob's
/// front extremity on the panel.
pub(super) fn raised_ridge(tempo: f32) -> Raised {
    let flange = flange(tempo);
    let ridge_half = flange.height() * RIDGE_H_FRAC * 0.5;
    let (back, top_front, foot_front) = (
        flange.center().y - ridge_half,
        flange.center().y + ridge_half,
        flange.bottom(),
    );
    // `a` runs from the foot (a quarter turn) to the top (none).
    let level = |a: f32| {
        let front = foot_front - (foot_front - top_front) * a.cos();
        let z = HEIGHT - (HEIGHT - SKIRT_HEIGHT) * a.sin();
        let r = Rect::from_min_max(
            Pos2::new(flange.left(), back),
            Pos2::new(flange.right(), front),
        );
        (corners(r), z)
    };
    let shade = |k: f32| {
        let f = |c: u8| (c as f32 * k).round() as u8;
        Color32::from_rgb(f(COL_BTN.r()), f(COL_BTN.g()), f(COL_BTN.b()))
    };
    let cove = (0..COVE_STEPS).map(|i| {
        let t0 = i as f32 / COVE_STEPS as f32;
        let t1 = (i + 1) as f32 / COVE_STEPS as f32;
        let a = |t: f32| FRAC_PI_2 * (1.0 - t);
        let (base, base_z) = level(a(t0));
        let (top, top_z) = level(a(t1));
        // `u` runs down the cove from the top.
        let u = 1.0 - (t0 + t1) * 0.5;
        let edge = COVE_SHADE_TOP + (COVE_SHADE_FOOT - COVE_SHADE_TOP) * u;
        let middle = (std::f32::consts::PI * u).sin();
        Footprint {
            base,
            base_z,
            top,
            top_z,
            outlined: false,
            fill: Some(shade(edge + (COVE_SHADE_MIDDLE - edge) * middle)),
        }
    });
    let foot = Rect::from_min_max(
        Pos2::new(flange.left(), back),
        Pos2::new(flange.right(), foot_front),
    );
    let skirt = Footprint {
        base: corners(foot),
        base_z: 0.0,
        top: corners(foot),
        top_z: SKIRT_HEIGHT,
        outlined: false,
        fill: Some(shade(SKIRT_SHADE)),
    };
    let top_rect = Rect::from_min_max(
        Pos2::new(flange.left(), back),
        Pos2::new(flange.right(), top_front),
    );
    let walls = std::iter::once(skirt).chain(cove).collect();
    // The lines where the top turns into the cove and the cove into the
    // skirt.
    let across = |y: f32, z: f32| Rib {
        a: [flange.left(), y, z],
        b: [flange.right(), y, z],
        normal: egui::Vec2::Y,
    };
    Raised {
        shapes: 0..0,
        lift: Lift::Flat(HEIGHT),
        walls,
        wall: COL_BTN,
        ribs: vec![across(top_front, HEIGHT), across(foot_front, SKIRT_HEIGHT)],
        rib: Stroke::new(2.0, COL_EDGE),
        facets: Vec::new(),
        tops: vec![(corners(top_rect), HEIGHT)],
    }
}
