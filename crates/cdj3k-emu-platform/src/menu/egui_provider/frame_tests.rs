//! Whole frames of the strip run through an egui context, with no window.

use egui::{Color32, FontId, Pos2, Vec2};

use super::panel::panel_width;
use super::style::*;
use super::*;
use crate::menu::model::{MenuIcon, NetKind, Notice};

mod panel;
mod strip;

/// One frame of `ui`, its texture uploads dropped: nothing renders here.
fn run(
    ctx: &egui::Context,
    input: egui::RawInput,
    ui: impl FnMut(&mut egui::Ui),
) -> egui::FullOutput {
    let mut out = ctx.run_ui(input, ui);
    out.textures_delta.clear();
    out
}

/// A context with the app's named families bound to egui's own fonts: the
/// strip draws the wordmark in one of them where it is the title bar.
fn test_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    let mut fonts = egui::FontDefinitions::default();
    let base = fonts.families[&egui::FontFamily::Proportional].clone();
    for name in [
        crate::fonts::NIMBUS_SANS,
        crate::fonts::NIMBUS_SANS_BOLD,
        crate::fonts::NIMBUS_SANS_CONDENSED,
    ] {
        fonts
            .families
            .insert(egui::FontFamily::Name(name.into()), base.clone());
    }
    ctx.set_fonts(fonts);
    ctx
}

/// Everything the strip painted this frame, as (text, rect) pairs.
///
/// Rows are painted rather than built from widgets, so tests locate them
/// in the shape list.
fn texts(out: &egui::FullOutput) -> Vec<(String, Rect)> {
    out.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some((t.galley.job.text.clone(), t.visual_bounding_rect())),
            _ => None,
        })
        .collect()
}

/// The pointer at `pos` over a 900 x 600 window.
fn hover(pos: Pos2) -> egui::RawInput {
    let mut input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(
            Pos2::new(0.0, 0.0),
            Vec2::new(900.0, 600.0),
        )),
        ..Default::default()
    };
    input.events.push(egui::Event::PointerMoved(pos));
    input
}

/// A primary click at `pos` over a 900 x 600 window.
fn click_at(pos: Pos2) -> egui::RawInput {
    let mut input = hover(pos);
    for pressed in [true, false] {
        input.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        });
    }
    input
}

/// A model with the shapes the panel has to be able to draw.
fn model() -> MenuModel {
    MenuModel {
        roots: vec![MenuNode::submenu(
            "Storage",
            vec![
                MenuNode::item(MenuId::ManageEmulation, "Manage Emulation"),
                MenuNode::Separator,
                MenuNode::Section("Slots".into()),
                MenuNode::check(MenuId::ServiceMode, "Service Mode", true),
                MenuNode::Readout {
                    title: "Pipeline latency".into(),
                    value: "141 ms".into(),
                    detail: None,
                },
                MenuNode::Predefined(Predefined::Quit),
            ],
        )
        .with_status("CDJ-3000", MenuIcon::Emulation)
        .with_detail("3.20")],
        ..Default::default()
    }
}
