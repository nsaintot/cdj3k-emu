//! Device slates - one per emulated player.
//!
//! A slate is the full panel renderer for one model: its reference canvas, the
//! section zones ([`cdj3k::layout`]), the chassis silhouette and the section
//! renderers (screen/top band, left transport column, right modes column).
//! Every slate draws with the SHARED toolkit at the `ui` root (colours,
//! [`UiScale`](super::UiScale), the button/rotary primitives, the shape caches
//! and the jog-wheel assembly in `ui::draw_jog`), so control chrome is identical
//! across models - only the panel composition differs.
//!
//! [`for_model`] is the one place here that names a player: a slate is a
//! [`Slate`] constant, so adding one is a module and an arm there, not a new
//! branch in every caller.

pub(in crate::app) mod cdj1500x;
pub(in crate::app) mod cdj3k;
pub(in crate::app) mod cdj3kx;

use cdj3k_emu_panel::Model;

use egui::Color32;

use crate::app::picker::Card;
use crate::app::CdjApp;

/// How one player is drawn, full size and as a picker miniature.
pub(in crate::app) struct Slate {
    /// Reference canvas `(w, h)` its layout constants are expressed in; the
    /// window is aspect-locked to it.
    pub ref_canvas: (f32, f32),
    /// The deck's colour wherever the setup window names it.
    pub accent: Color32,
    pub card: Card,
    /// Chassis, LCD overlay, then every section.
    pub draw_panel: fn(&mut CdjApp, &mut egui::Ui),
}

pub(in crate::app) fn for_model(model: Model) -> &'static Slate {
    match model {
        Model::Cdj3k => &cdj3k::SLATE,
        Model::Cdj3kx => &cdj3kx::SLATE,
        Model::Cdj1500x => &cdj1500x::SLATE,
    }
}
