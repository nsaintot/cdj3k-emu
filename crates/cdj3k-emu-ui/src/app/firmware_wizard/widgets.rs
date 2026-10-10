//! The install step's own pieces: form labels, the outcome banner and the
//! stage meter.

use egui::{
    Align, Color32, CornerRadius, FontId, Frame, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};

use super::provision::{ProvisionStep, STAGES};
use crate::app::{picker, theme};

/// The width of the label column, which lines up the fields of every form row.
const LABEL_W: f32 = 116.0;
/// The stage meter: the height of a segment, and the gap to the next.
const METER_SEG_H: f32 = 3.0;
const METER_GAP: f32 = 6.0;
/// Font size of the name of the current stage, shown over the meter.
const METER_TITLE_FONT: f32 = 15.0;
/// Size of the square, in the state's colour, before a banner's text.
const MARK: f32 = 7.0;

/// A field label: capitals at the window's micro size, tracked by the painter
/// because egui has no letter-spacing.
pub(super) fn micro_label(ui: &mut egui::Ui, k: f32, col: Color32, text: &str) {
    let text = text.to_uppercase();
    let font = FontId::proportional(theme::MICRO_FONT * k);
    let w = picker::text_width(ui.painter(), &text, &font, theme::MICRO_TRACK * k);
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(w, theme::MICRO_FONT * k * 1.6), Sense::hover());
    picker::tracked(
        ui.painter(),
        Pos2::new(rect.left(), rect.center().y),
        &text,
        &font,
        col,
        theme::MICRO_TRACK * k,
    );
}

/// Draws the same label in the fixed label column, level with the middle of the
/// row's field.
pub(super) fn label_column(ui: &mut egui::Ui, k: f32, col: Color32, text: &str) {
    let text = text.to_uppercase();
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(LABEL_W * k, theme::FIELD_H * k), Sense::hover());
    picker::tracked(
        ui.painter(),
        Pos2::new(rect.left(), rect.center().y),
        &text,
        &FontId::proportional(theme::MICRO_FONT * k),
        col,
        theme::MICRO_TRACK * k,
    );
}

/// A plate across the step saying how the run ended: a square of the state's
/// colour, then the line itself.
pub(super) fn banner(
    ui: &mut egui::Ui,
    k: f32,
    pal: &theme::Palette,
    mark: Color32,
    text: &str,
    col: Color32,
) {
    Frame::default()
        .fill(pal.plate)
        .stroke(Stroke::new(theme::HAIRLINE * k, pal.line))
        .corner_radius(CornerRadius::from(theme::ROUND * k))
        .inner_margin(egui::epaint::MarginF32::symmetric(14.0 * k, 11.0 * k))
        .show(ui, |ui| {
            // Use the full width, so the banner reads as a bar across the step.
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(MARK * k), Sense::hover());
                ui.painter().rect_filled(rect, theme::HAIRLINE * k, mark);
                ui.label(RichText::new(text).size(theme::BODY_FONT * k).color(col));
            });
        });
}

/// The stage meter: the stage under way, how far along the run is, and the
/// four stages named under their own segments.
pub(super) fn meter(ui: &mut egui::Ui, k: f32, pal: &theme::Palette, step: &ProvisionStep) {
    let Some(at) = step.stage() else {
        return;
    };
    Frame::default()
        .fill(pal.plate)
        .stroke(Stroke::new(theme::HAIRLINE * k, pal.line))
        .corner_radius(CornerRadius::from(theme::ROUND * k))
        .inner_margin(egui::epaint::MarginF32::symmetric(16.0 * k, 14.0 * k))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 9.0 * k;
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(step.label())
                        .size(METER_TITLE_FONT * k)
                        .color(pal.ink),
                );
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("STEP {} / {}", at + 1, STAGES.len()))
                            .size(theme::HINT_FONT * k)
                            .monospace()
                            .color(pal.dim),
                    );
                });
            });
            meter_segments(ui, at, k, pal);
        });
}

/// One segment per stage, lit up to the one under way, each named under it.
fn meter_segments(ui: &mut egui::Ui, at: usize, k: f32, pal: &theme::Palette) {
    let n = STAGES.len() as f32;
    let gap = METER_GAP * k;
    let (bar, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), METER_SEG_H * k),
        Sense::hover(),
    );
    let seg = (bar.width() - gap * (n - 1.0)) / n;
    for i in 0..STAGES.len() {
        let x = bar.left() + i as f32 * (seg + gap);
        ui.painter().rect_filled(
            Rect::from_min_size(Pos2::new(x, bar.top()), Vec2::new(seg, bar.height())),
            0.0,
            if i <= at { pal.ink } else { pal.off_line },
        );
    }

    let (names, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), theme::HINT_FONT * k * 1.5),
        Sense::hover(),
    );
    for (i, name) in STAGES.iter().enumerate() {
        let (col, font) = match i {
            _ if i < at => (pal.dim, FontId::proportional(theme::HINT_FONT * k)),
            _ if i == at => (pal.ink, FontId::proportional(theme::HINT_FONT * k)),
            _ => (pal.faint, FontId::proportional(theme::HINT_FONT * k)),
        };
        ui.painter().text(
            Pos2::new(names.left() + i as f32 * (seg + gap), names.center().y),
            egui::Align2::LEFT_CENTER,
            name,
            font,
            col,
        );
    }
}
