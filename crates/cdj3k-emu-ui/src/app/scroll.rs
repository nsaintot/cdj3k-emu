//! The wheel and trackpad as the frame's own events report them.

use egui::{Event, MouseWheelUnit, Vec2};

/// This frame's wheel movement, unsmoothed and whatever the modifiers: the
/// deck's jog and rotaries turn by what the hand did, and ctrl+scroll is
/// theirs too. Lines and pages are counted in points the way egui counts them.
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
