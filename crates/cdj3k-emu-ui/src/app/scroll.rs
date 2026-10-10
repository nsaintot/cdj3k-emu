//! Reads the wheel and trackpad movement from the frame's own events.

use egui::{Event, MouseWheelUnit, Vec2};

/// This frame's wheel movement, unsmoothed and regardless of modifiers, so
/// the deck's jog and rotaries turn by exactly what the hand did, also while
/// Ctrl is held. Lines and pages are converted to points the way egui
/// converts them.
pub(in crate::app) fn raw_delta(ui: &egui::Ui) -> Vec2 {
    let line = ui.ctx().options(|o| o.input_options.line_scroll_speed);
    ui.input(|i| {
        let page = i.viewport_rect().height();
        i.events
            .iter()
            .filter_map(|e| match e {
                Event::MouseWheel { unit, delta, .. } => Some(match unit {
                    MouseWheelUnit::Point => *delta,
                    MouseWheelUnit::Line => *delta * line,
                    MouseWheelUnit::Page => *delta * page,
                }),
                _ => None,
            })
            .fold(Vec2::ZERO, |sum, d| sum + d)
    })
}
