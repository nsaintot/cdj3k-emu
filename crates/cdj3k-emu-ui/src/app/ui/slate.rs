//! Device slates, one per emulated player.
//!
//! A slate is the full panel renderer for one model: its reference canvas, the
//! section zones (such as `cdj3k::layout`), the chassis silhouette and the section
//! renderers (screen/top band, left transport column, right modes column).
//! Every slate draws with the shared toolkit at the `ui` root (colours,
//! [`UiScale`], the button and rotary primitives, the shape
//! caches and the jog-wheel assembly in `ui::draw_jog`), so the control chrome
//! is identical across models and only the panel composition differs.
//!
//! [`for_model`] is the only place here that names a player. A slate is a
//! [`Slate`] constant, so adding a player takes a new module and one match arm
//! in [`for_model`].

pub(in crate::app) mod card;
pub(in crate::app) mod cdj1500x;
pub(in crate::app) mod cdj3k;
pub(in crate::app) mod cdj3kx;

use cdj3k_emu_panel::Model;

use egui::{Color32, Pos2, Rect, Vec2};

use super::draw_cache::ShapeList;
use super::{debug_align_squares, draw_jog, JogChrome, UiScale};
use crate::app::CdjApp;

use card::{Bay, Card, Glyph};

/// How one player is drawn, full size and as a picker miniature.
pub(in crate::app) struct Slate {
    /// Reference canvas `(w, h)` its layout constants are expressed in; the
    /// window is aspect-locked to it.
    pub ref_canvas: (f32, f32),
    /// The deck's colour wherever the setup window names it.
    pub accent: Color32,
    pub card: Card,
    /// Draws the chassis, the LCD overlay, then every section.
    pub draw_panel: fn(&mut CdjApp, &mut egui::Ui),
}

pub(in crate::app) fn for_model(model: Model) -> &'static Slate {
    match model {
        Model::Cdj3k => &cdj3k::SLATE,
        Model::Cdj3kx => &cdj3kx::SLATE,
        Model::Cdj1500x => &cdj1500x::SLATE,
    }
}

/// One section of a flat slate, drawn over its chassis.
type Section = fn(&mut CdjApp, &mut egui::Ui, &egui::Painter, &UiScale);

/// How a slate without a front face is composed: its canvas, its static
/// chassis and LCD overlay, then its sections and its jog zone.
pub(in crate::app) struct FlatPanel {
    pub canvas: (f32, f32),
    pub chassis: fn(&mut ShapeList, &UiScale),
    pub lcd_overlay: fn(&mut ShapeList, &UiScale),
    pub top: Section,
    pub left: Section,
    pub jog: (fn() -> Rect, JogChrome),
    pub right: Section,
}

/// Draws the whole panel: the chassis, the LCD overlay, then every section.
pub(in crate::app) fn draw_flat_panel(app: &mut CdjApp, ui: &mut egui::Ui, panel: &FlatPanel) {
    let (ref_w, ref_h) = panel.canvas;
    // `max_rect`, not `clip_rect`: the clip rect is a painting bound and
    // still spans the window when a panel sits above, which would draw the
    // chassis underneath the in-window menu.
    let layout = UiScale::fit(ui.max_rect().shrink(2.0), ref_w, ref_h);
    let p = ui.painter().clone();
    let ppp = ui.ctx().pixels_per_point();

    // The chassis is static and is rebuilt only on a resize.
    app.frame_shape_count += app
        .caches
        .chassis_bg
        .paint(&p, &layout, ppp, |list| (panel.chassis)(list, &layout));
    // The LCD overlay sits on top of all sections; it is static and is rebuilt
    // only on a resize.
    app.frame_shape_count += app
        .caches
        .chassis_lcd_overlay
        .paint(&p, &layout, ppp, |list| (panel.lcd_overlay)(list, &layout));

    {
        puffin::profile_scope!("draw_top");
        (panel.top)(app, ui, &p, &layout);
    }
    {
        puffin::profile_scope!("draw_left");
        (panel.left)(app, ui, &p, &layout);
    }
    {
        puffin::profile_scope!("draw_jog");
        let (jog_rect, chrome) = panel.jog;
        app.draw_jog_wheel_section(ui, &p, &layout, jog_rect(), chrome);
    }
    {
        puffin::profile_scope!("draw_right");
        (panel.right)(app, ui, &p, &layout);
    }

    debug_align_squares(&p, &layout, ref_w, ref_h);
}

/// The zones a riser card's glyph is built from, in the slate's reference
/// units. The panel draws from the same zones, so the glyph always matches it.
pub(in crate::app) struct RiserZones {
    pub canvas: (f32, f32),
    /// The block the screen stands in.
    pub riser: Rect,
    /// The screen's glass.
    pub screen: Rect,
    /// Gap between the screen and its frame, already a fraction of the
    /// outline.
    pub bezel: Vec2,
    pub recessed: bool,
    /// The hot-cue strip, and the share of each key's pitch left as gap.
    pub pads: Rect,
    pub pad_gap: f32,
    pub transport_col: Rect,
    pub modes_col: Rect,
    pub bay: Bay,
    /// The jog zone; the jog is the shared assembly centred in it.
    pub jog: Rect,
    /// The browse pod's centre, half the width of its button block, and its
    /// bezel ring's radius.
    pub pod: Pos2,
    pub pod_half_w: f32,
    pub pod_ring_r: f32,
}

/// `r` as fractions of a `w` x `h` canvas.
const fn fraction_of(r: Rect, w: f32, h: f32) -> Rect {
    Rect::from_min_max(
        Pos2::new(r.min.x / w, r.min.y / h),
        Pos2::new(r.max.x / w, r.max.y / h),
    )
}

/// The riser glyph of a slate whose zones are `z`.
pub(in crate::app) const fn riser_glyph(z: RiserZones) -> Glyph {
    let (w, h) = z.canvas;
    Glyph {
        canvas: z.canvas,
        body_top: z.transport_col.min.y / h,
        riser: (z.riser.min.x / w, z.riser.max.x / w),
        screen: fraction_of(z.screen, w, h),
        bezel: z.bezel,
        transport_col: fraction_of(z.transport_col, w, h),
        modes_col: fraction_of(z.modes_col, w, h),
        bay: z.bay,
        recessed: z.recessed,
        pads_x: (z.pads.min.x / w, z.pads.max.x / w),
        pads_y: (z.pads.min.y / h, z.pads.max.y / h),
        pad_gap: z.pad_gap,
        jog: (
            (z.jog.min.x + z.jog.max.x) * 0.5 / w,
            (z.jog.min.y + z.jog.max.y) * 0.5 / h,
            draw_jog::JOG_OUTER_2_STROKE_RADIUS / w,
        ),
        jog_hub: draw_jog::JOG_INNER_LCD_C0_RADIUS / draw_jog::JOG_OUTER_2_STROKE_RADIUS,
        pod: Pos2::new(z.pod.x / w, z.pod.y / h),
        pod_half_w: z.pod_half_w,
        pod_ring_r: z.pod_ring_r,
    }
}
