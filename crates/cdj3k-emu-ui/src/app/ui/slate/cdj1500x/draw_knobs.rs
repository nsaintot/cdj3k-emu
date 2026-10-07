//! The two push encoders, BEAT LOOP on the left and BROWSE on the right, and
//! the printed RELOOP legend under BEAT LOOP.
//!
//! Both knobs are one part: a silver ring, toothed round its edge, around a
//! dark cap. BROWSE turns the browse rotary and presses ROTARY_PRESS; BEAT
//! LOOP turns its own encoder counter and presses RELOOP.

use std::f32::consts::TAU;

use cdj3k_emu_panel::Btn;
use egui::{Color32, FontFamily, Pos2, Rect, Sense, Shape, Stroke, Vec2};

use crate::app::ui::draw_cache::ShapeList;
use crate::app::ui::tilt::{Facet, Footprint, Lift, Raised};
use crate::app::ui::{
    UiScale, COL_BLACK, COL_BTN, COL_BTN_TEXT, COL_DARK, COL_SILVER, NAV_DETENT_COUNT,
    NAV_SCROLL_PX_PER_TICK,
};
use crate::app::CdjApp;

use super::layout::px;

const BEAT_LOOP_CENTER: (f32, f32) = (124.84, 1211.48);
const BROWSE_CENTER: (f32, f32) = (1304.05, 1211.35);

/// Radii from the centre out, drawing pixels: the cap's edge, the root of
/// the silver ring's teeth, and their tips.
const CAP_R: f32 = 37.6;
const TEETH_ROOT_R: f32 = 49.0;
const TEETH_TIP_R: f32 = 54.0;
/// Teeth round the ring, and the share of each pitch a tooth's tip covers.
const TEETH: usize = 60;
const TOOTH_TIP_FRAC: f32 = 0.45;

const COL_RING: Color32 = COL_SILVER;

const BEAT_LOOP_CAPTION: (f32, f32, f32, f32) = (64.0, 1115.0, 187.0, 1130.0);
const BROWSE_CAPTION: (f32, f32, f32, f32) = (1258.0, 1115.0, 1352.0, 1130.0);
const RELOOP_PILL: (f32, f32, f32, f32) = (66.0, 1291.0, 185.0, 1320.0);
const RELOOP_TEXT: (f32, f32, f32, f32) = (89.0, 1299.0, 163.0, 1313.0);

const CAP_STROKE: f32 = 2.0;
const PILL_STROKE: f32 = 2.0;

fn screen_box(layout: &UiScale, (l, t, r, b): (f32, f32, f32, f32)) -> Rect {
    Rect::from_min_max(layout.sp(px(l), px(t)), layout.sp(px(r), px(b)))
}

/// Captions and the RELOOP legend (cached; rebuilt on resize).
pub(super) fn collect_knob_statics(list: &mut ShapeList, ctx: &egui::Context, layout: &UiScale) {
    let bold = FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS_BOLD.into());
    let regular = FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS.into());
    for (text, ink) in [("BEAT LOOP", BEAT_LOOP_CAPTION), ("BROWSE", BROWSE_CAPTION)] {
        list.text_in_ink_box(
            ctx,
            screen_box(layout, ink),
            text,
            bold.clone(),
            COL_BTN_TEXT,
        );
    }
    let pill = screen_box(layout, RELOOP_PILL);
    list.rect_stroke(
        pill,
        pill.height() * 0.5,
        Stroke::new(layout.sc(PILL_STROKE), COL_SILVER),
    );
    list.text_in_ink_box(
        ctx,
        screen_box(layout, RELOOP_TEXT),
        "RELOOP",
        regular,
        COL_BTN_TEXT,
    );
}

pub(super) fn draw_knobs(app: &mut CdjApp, ui: &mut egui::Ui, p: &egui::Painter, layout: &UiScale) {
    let beat_loop = centre(layout, BEAT_LOOP_CENTER);
    if let Some(ticks) = turned(
        app,
        ui,
        layout,
        beat_loop,
        "beat_loop_knob",
        Btn::Reloop,
        |app| &mut app.beat_loop_scroll_accum,
    ) {
        // Absolute counter like BROWSE; the deck differentiates it.
        for _ in 0..ticks.unsigned_abs() {
            app.beat_loop = if ticks > 0 {
                app.beat_loop.wrapping_sub(1)
            } else {
                app.beat_loop.wrapping_add(1)
            };
            app.inject_rotary();
        }
        app.beat_loop_angle += ticks as f32 * TAU / NAV_DETENT_COUNT;
    }

    let browse = centre(layout, BROWSE_CENTER);
    if let Some(ticks) = turned(
        app,
        ui,
        layout,
        browse,
        "browse_knob",
        Btn::RotaryPress,
        |app| &mut app.nav_scroll_accum,
    ) {
        // The rotary counts down for a scroll down, as on the other decks.
        for _ in 0..ticks.unsigned_abs() {
            app.rotary = if ticks > 0 {
                app.rotary.wrapping_sub(1)
            } else {
                app.rotary.wrapping_add(1)
            };
            app.inject_rotary();
        }
        app.nav_angle += ticks as f32 * TAU / NAV_DETENT_COUNT;
    }

    let pressed = |btn| {
        app.button(btn)
            .is_some_and(|b| app.held_btn == Some(b) || app.latched_btns.contains(&b))
    };
    let angles = [app.beat_loop_angle, app.nav_angle];
    let held = [pressed(Btn::Reloop), pressed(Btn::RotaryPress)];
    let (ox, oy, scale) = layout.cache_key();
    let q = |v: f32| (v * 1000.0).round() as i32;
    let key = (q(ox), q(oy), q(scale), angles.map(f32::to_bits), held);
    let shapes = app.knob_cache.get_or_build(key, |list| {
        collect_knob(list, layout, beat_loop, angles[0], held[0]);
        collect_knob(list, layout, browse, angles[1], held[1]);
    });
    let n = shapes.len() as u64;
    p.extend(shapes.iter().cloned());
    app.frame_shape_count += n;
}

fn centre(layout: &UiScale, (x, y): (f32, f32)) -> Pos2 {
    layout.sp(px(x), px(y))
}

/// Hit-test a knob: a press drives `btn`, a scroll over it turns it. Returns
/// the detents turned this frame, positive for a scroll down.
fn turned(
    app: &mut CdjApp,
    ui: &mut egui::Ui,
    layout: &UiScale,
    center: Pos2,
    id: &str,
    btn: Btn,
    accum: impl Fn(&mut CdjApp) -> &mut f32,
) -> Option<i32> {
    let r = layout.sc(px(TEETH_TIP_R));
    let rect = Rect::from_center_size(center, Vec2::splat(2.0 * r));
    let resp = ui.interact(rect, ui.id().with(id), Sense::click_and_drag());

    let is_down = resp.is_pointer_button_down_on();
    let ctrl = ui.input(|i| i.modifiers.ctrl);
    if let Some(bit) = app.button(btn) {
        app.handle_btn_interaction(is_down, ctrl, bit);
    }

    if !resp.hovered() {
        return None;
    }
    // Raw, so ctrl+scroll still reaches the knob.
    let scroll_y = crate::app::scroll::raw_delta(ui).y;
    if scroll_y == 0.0 {
        return None;
    }
    let acc = accum(app);
    *acc += scroll_y;
    let ticks = (*acc / NAV_SCROLL_PX_PER_TICK).trunc();
    *acc -= ticks * NAV_SCROLL_PX_PER_TICK;
    (ticks != 0.0).then_some(ticks as i32)
}

/// One knob at `center`, its ring turned by `angle`.
fn collect_knob(list: &mut ShapeList, layout: &UiScale, center: Pos2, angle: f32, pressed: bool) {
    let s = |r: f32| layout.sc(px(r));
    let pitch = TAU / TEETH as f32;
    let (root, tip) = (s(TEETH_ROOT_R), s(TEETH_TIP_R));
    for i in 0..TEETH {
        let a = angle + pitch * i as f32;
        let (h_root, h_tip) = (0.5 * pitch, 0.5 * pitch * TOOTH_TIP_FRAC);
        list.add(Shape::convex_polygon(
            vec![
                center + Vec2::angled(a - h_root) * root,
                center + Vec2::angled(a - h_tip) * tip,
                center + Vec2::angled(a + h_tip) * tip,
                center + Vec2::angled(a + h_root) * root,
            ],
            COL_RING,
            Stroke::NONE,
        ));
    }
    list.circle_filled(center, root, COL_RING);
    list.circle_filled(center, s(CAP_R), if pressed { COL_BTN } else { COL_DARK });
    list.circle_stroke(
        center,
        s(CAP_R),
        Stroke::new(layout.sc(CAP_STROKE), COL_BLACK),
    );
}

/// How far a knob stands above the panel.
pub(super) const HEIGHT: f32 = 105.0;

/// The knobs' sides: a silver cylinder, its teeth running up it. Each tooth
/// shows a land at the tips' radius and a flank down either side to the
/// root, where it meets the next; each patch is lit by how it faces
/// [`LIGHT`].
const COL_WALL: Color32 = Color32::from_rgb(92, 92, 100);
/// Where the light comes from, in the plan: the front, from the left.
const LIGHT: Vec2 = Vec2::new(-0.4, 1.0);
/// A patch's shade, facing away from [`LIGHT`] to facing it.
const AMBIENT: f32 = 0.45;
const DIFFUSE: f32 = 0.85;
/// The flanks sit in the dent between two teeth, and take less light.
const FLANK_SHADE: f32 = 0.75;

/// Both knobs, standing off the panel, their teeth turned with them.
pub(super) fn raised(app: &CdjApp) -> Raised {
    let light = LIGHT.normalized();
    let shade = |normal: Vec2, k: f32| {
        let lit = (AMBIENT + DIFFUSE * normal.dot(light).max(0.0)) * k;
        let f = |c: u8| (c as f32 * lit).round().clamp(0.0, 255.0) as u8;
        Color32::from_rgb(f(COL_WALL.r()), f(COL_WALL.g()), f(COL_WALL.b()))
    };

    let mut walls = Vec::new();
    let mut facets = Vec::new();
    let mut tops = Vec::new();
    for ((x, y), angle) in [
        (BEAT_LOOP_CENTER, app.beat_loop_angle),
        (BROWSE_CENTER, app.nav_angle),
    ] {
        let c = Pos2::new(px(x), px(y));
        let (tip, root) = (px(TEETH_TIP_R), px(TEETH_ROOT_R));
        walls.push(Footprint::upright(super::key::circle(c, tip), HEIGHT));

        // A patch standing up from the panel along the plan edge `p`-`q`.
        let mut patch = |p: Pos2, q: Pos2, k: f32| {
            let mut normal = (q - p).rot90().normalized();
            if normal.dot(p + (q - p) * 0.5 - c) < 0.0 {
                normal = -normal;
            }
            facets.push(Facet {
                quad: [
                    [p.x, p.y, 0.0],
                    [q.x, q.y, 0.0],
                    [q.x, q.y, HEIGHT],
                    [p.x, p.y, HEIGHT],
                ],
                normal,
                fill: shade(normal, k),
            });
        };
        let pitch = TAU / TEETH as f32;
        let (h_root, h_tip) = (0.5 * pitch, 0.5 * pitch * TOOTH_TIP_FRAC);
        let at = |a: f32, r: f32| c + Vec2::angled(a) * r;
        for i in 0..TEETH {
            let a = angle + pitch * i as f32;
            patch(at(a - h_root, root), at(a - h_tip, tip), FLANK_SHADE);
            patch(at(a - h_tip, tip), at(a + h_tip, tip), 1.0);
            patch(at(a + h_tip, tip), at(a + h_root, root), FLANK_SHADE);
        }
        tops.push((super::key::circle(c, tip), HEIGHT));
    }
    Raised {
        shapes: 0..0,
        lift: Lift::Flat(HEIGHT),
        walls,
        wall: COL_WALL,
        ribs: Vec::new(),
        rib: Stroke::NONE,
        facets,
        tops,
    }
}
