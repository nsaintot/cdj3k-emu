//! The CDJ-1500X slate: its reference canvas, section zones and section
//! renderers.
//!
//! The canvas is the cabinet's published footprint, 252.1 x 374.7 mm, at the
//! CDJ-3000X slate's 9.68 units per millimetre, so a part measures the same on
//! both. Every position is traced off AlphaTheta's plan drawing of the deck
//! (`cdj1500x_features_line3.png`, 1430 x 2125 px, the cabinet edge to edge),
//! through [`layout::px`].

mod draw_chassis;
mod draw_front;
mod draw_jog;
mod draw_knobs;
mod draw_tempo;
mod draw_top;
mod draw_transport;
mod key;
mod layout;

use super::card::{Card, CardGlyph};
use crate::app::ui::slate::Slate;
#[allow(unused_imports)]
use crate::app::ui::*;
use crate::app::CdjApp;
use cdj3k_emu_panel::Btn;
use egui::Color32;

pub(in crate::app) const SLATE: Slate = Slate {
    ref_canvas: (REF_W, REF_H),
    accent: Color32::from_rgb(255, 140, 40),
    card: Card {
        subtitle: "10.1-inch touch display",
        era: "2026",
        glyph: CardGlyph::Flat,
    },
    draw_panel,
};

/// The reference canvas, in the unscaled layout units of every zone in
/// [`layout`]. The window keeps the `REF_W : REF_H` aspect ratio.
pub(in crate::app) const REF_W: f32 = 2440.0;

pub(in crate::app) const REF_H: f32 = layout::px(layout::DRAWING_H);

/// Draws the whole panel: the chassis, the LCD and each section. While the
/// deck is tilted, it then turns all of it and draws the front face below.
fn draw_panel(app: &mut CdjApp, ui: &mut egui::Ui) {
    // The layout fits `max_rect`, as on the CDJ-3000X slate.
    let layout = UiScale::fit(ui.max_rect().shrink(2.0), REF_W, REF_H);
    let p = ui.painter().clone();
    let ppp = ui.ctx().pixels_per_point();
    let mut parts = Parts {
        layer: ui.layer_id(),
        t: 0.0,
        raised: Vec::new(),
    };
    let start = parts.mark(ui);
    // How far the deck is tilted, eased in and out; some parts shade with it.
    let t = ui.ctx().animate_bool_with_time(
        egui::Id::new("cdj1500x_tilt"),
        app.tilted,
        draw_front::TILT_SECS,
    );
    parts.t = t * t * (3.0 - 2.0 * t);
    app.tilt_progress = parts.t;

    app.frame_shape_count += app.caches.chassis_bg.paint(&p, &layout, ppp, |list| {
        draw_chassis::collect_chassis(list, &layout);
    });
    draw_sections(app, ui, &p, &layout, &mut parts);

    debug_align_squares(&p, &layout, REF_W, REF_H);

    let turned = turn(app, ui, &p, &layout, start, &parts);
    if draw_front::draw_toggle(app, &p, &layout, turned.as_ref(), parts.t) {
        app.tilted = !app.tilted;
    }
}

/// What each raised part painted on the slate's layer, and how it stands.
struct Parts {
    layer: egui::LayerId,
    /// The eased tilt. While it is 0 and the deck is flat, no part is recorded.
    t: f32,
    raised: Vec<tilt::Raised>,
}

impl Parts {
    /// Returns the number of shapes on the layer. A part's shapes are the ones
    /// added between two marks.
    fn mark(&self, ui: &egui::Ui) -> usize {
        ui.ctx()
            .graphics(|g| g.get(self.layer).map_or(0, |l| l.next_idx().0))
    }

    /// Records the shapes painted since `from` as one raised part, standing
    /// as `r` describes.
    fn add(&mut self, ui: &egui::Ui, from: usize, r: impl FnOnce() -> tilt::Raised) {
        let shapes = from..self.mark(ui);
        if self.t > 0.0 {
            self.raised.push(tilt::Raised { shapes, ..r() });
        }
    }
}

/// Draws the LCD, then each section, and records the raised parts.
fn draw_sections(
    app: &mut CdjApp,
    ui: &mut egui::Ui,
    p: &egui::Painter,
    layout: &UiScale,
    parts: &mut Parts,
) {
    {
        puffin::profile_scope!("draw_lcd");
        let extended = cdj3k_emu_platform::menu_state::lock().screen_extended;
        draw_lcd::draw_main_lcd(app, ui, p, layout, layout::lcd_bezel(extended));
    }
    {
        puffin::profile_scope!("draw_top");
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        app.frame_shape_count += app.caches.top_statics.paint(p, layout, ppp, |list| {
            draw_top::collect_top_statics(list, &ctx, layout);
            draw_knobs::collect_knob_statics(list, &ctx, layout);
            draw_tempo::collect_tempo_statics(list, &ctx, layout);
            draw_chassis::collect_badge(list, &ctx, layout);
        });
        let from = parts.mark(ui);
        draw_top::draw_top_section(app, ui, layout);
        parts.add(ui, from, || {
            key::raised(draw_top::footprints(), key::HEIGHT)
        });
    }
    {
        puffin::profile_scope!("draw_knobs");
        let from = parts.mark(ui);
        draw_knobs::draw_knobs(app, ui, p, layout);
        parts.add(ui, from, || draw_knobs::raised(app));
    }
    {
        puffin::profile_scope!("draw_transport");
        let from = parts.mark(ui);
        draw_transport::draw_shift(app, ui, layout);
        parts.add(ui, from, || {
            key::raised(vec![draw_transport::footprints().0], key::HEIGHT)
        });
        let from = parts.mark(ui);
        draw_transport::draw_cue_play(app, ui, layout);
        parts.add(ui, from, || {
            key::raised(draw_transport::footprints().1, key::ROUND_HEIGHT)
        });
    }
    {
        puffin::profile_scope!("draw_jog");
        let from = parts.mark(ui);
        draw_jog::draw_jog(app, ui, p, layout);
        parts.add(ui, from, draw_jog::raised);
    }
    {
        puffin::profile_scope!("draw_tempo");
        let from = parts.mark(ui);
        draw_tempo::draw_tempo(app, ui, p, layout, parts.t);
        parts.add(ui, from, || draw_tempo::raised_flange(app.tempo));
        let from = parts.mark(ui);
        draw_tempo::draw_tempo_ridge(app, p, layout);
        parts.add(ui, from, || draw_tempo::raised_ridge(app.tempo));
    }
}

/// While the deck is tilted, replaces what the slate painted from `start`
/// onward with its projection, draws the front face below and maps the
/// pointer back, then returns the tilt. When the deck is flat, EJECT is
/// released.
fn turn(
    app: &mut CdjApp,
    ui: &mut egui::Ui,
    p: &egui::Painter,
    layout: &UiScale,
    start: usize,
    parts: &Parts,
) -> Option<tilt::Tilt> {
    app.tilt_input = None;
    if parts.t <= 0.0 {
        if let Some(bit) = app.button(Btn::UsbStop) {
            // EJECT is on the front face, so it is released while the face is hidden.
            app.handle_btn_interaction(false, false, bit);
        }
        return None;
    }
    let ctx = ui.ctx().clone();
    let tilt = tilt::Tilt::new(
        layout,
        REF_W,
        REF_H,
        parts.t,
        draw_front::TILT_ANGLE,
        draw_front::CAM_DIST,
        Some(draw_front::crease()),
        &draw_front::extent(),
    );
    tilt::project_layer(
        &ctx,
        parts.layer,
        egui::layers::ShapeIdx(start),
        layout,
        &tilt,
        &parts.raised,
        egui::Stroke::new(layout.sc(2.0), COL_SILVER),
        &mut app.tilt_cache,
    );
    draw_front::draw_front(app, ui, p, layout, &tilt);
    // Hit-test the tops of the raised parts first, the last drawn first.
    let tops = parts
        .raised
        .iter()
        .rev()
        .flat_map(|r| r.screen_tops(&tilt))
        .collect();
    app.tilt_input = Some(crate::app::tilt_input::TiltInput {
        tilt,
        panel: ui.max_rect(),
        canvas: (REF_W, REF_H),
        tops,
    });
    Some(tilt)
}
