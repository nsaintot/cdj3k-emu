//! Text faces and the small marks the strip and its rows share.

use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use super::style::TEXT;

// ── Text ──────────────────────────────────────────────────────────────────────

pub(super) fn tracked_fmt(size: f32, track: f32, color: Color32) -> egui::TextFormat {
    egui::TextFormat {
        font_id: FontId::proportional(size),
        extra_letter_spacing: size * track,
        color,
        ..Default::default()
    }
}

pub(super) fn mono_fmt(size: f32, color: Color32) -> egui::TextFormat {
    egui::TextFormat {
        font_id: FontId::monospace(size),
        color,
        ..Default::default()
    }
}

pub(super) fn paint_tracked(
    ui: &Ui,
    pos: Pos2,
    align: egui::Align2,
    text: &str,
    size: f32,
    track: f32,
    color: Color32,
) {
    let mut job = egui::text::LayoutJob::default();
    job.append(text, 0.0, tracked_fmt(size, track, color));
    job.wrap.max_width = f32::INFINITY;
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let at = align.align_size_within_rect(galley.size(), Rect::from_center_size(pos, Vec2::ZERO));
    ui.painter().galley(at.min, galley, color);
}

pub(super) fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font, TEXT).size().x)
}

// ── Marks ─────────────────────────────────────────────────────────────────────

pub(super) fn vertical_rule(ui: &mut Ui, h: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, h), Sense::hover());
    ui.painter()
        .vline(rect.center().x, rect.y_range(), Stroke::new(1.0_f32, color));
}

/// The design's chevron: an open V, centred on `c`, 9 x 6.
pub(super) fn chevron(ui: &Ui, c: Pos2, color: Color32) {
    let s = Stroke::new(1.3_f32, color);
    ui.painter().add(egui::Shape::line(
        vec![
            c + Vec2::new(-3.5, -1.7),
            c + Vec2::new(0.0, 1.7),
            c + Vec2::new(3.5, -1.7),
        ],
        s,
    ));
}

/// The design's tick, drawn from its left-centre.
pub(super) fn check_mark(p: &egui::Painter, left: Pos2, color: Color32) {
    let s = Stroke::new(1.4_f32, color);
    p.add(egui::Shape::line(
        vec![
            left + Vec2::new(1.0, 0.1),
            left + Vec2::new(4.2, 3.1),
            left + Vec2::new(11.0, -3.3),
        ],
        s,
    ));
}
