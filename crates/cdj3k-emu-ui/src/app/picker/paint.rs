//! Painting helpers the setup window's steps share: letter-spaced text,
//! hairlines, the bin glyph and colour blending.

use egui::{Align2, Color32, FontId, Pos2, Rect, Stroke};

use crate::app::theme;

// The bin on the delete button, in fractions of the square the button
// reserves for it: a lid, a handle and a body that tapers to its base. It is
// drawn because no bundled font has a bin glyph, and an emoji bin would be
// the only filled shape in the window.
const TRASH_LID_Y: f32 = 0.28;
const TRASH_LID_X: (f32, f32) = (0.04, 0.96);
const TRASH_HANDLE_X: (f32, f32) = (0.34, 0.66);
const TRASH_HANDLE_Y: f32 = 0.10;
const TRASH_BODY_TOP: (f32, f32) = (0.15, 0.85);
const TRASH_BODY_BASE: (f32, f32) = (0.26, 0.74);
const TRASH_BODY_Y: f32 = 0.94;

/// Draw `text` with `track` extra pixels between glyphs (egui's painter has no
/// letter-spacing), left-aligned on `pos` and centred on its y. Returns the
/// width it took.
pub(in crate::app) fn tracked(
    p: &egui::Painter,
    pos: Pos2,
    text: &str,
    font: &FontId,
    color: Color32,
    track: f32,
) -> f32 {
    let mut x = pos.x;
    for c in text.chars() {
        p.text(
            Pos2::new(x, pos.y),
            Align2::LEFT_CENTER,
            c,
            font.clone(),
            color,
        );
        x += glyph_width(p, c, font) + track;
    }
    (x - pos.x - track).max(0.0)
}

/// Draws [`tracked`] text centred on `pos`.
pub(super) fn tracked_center(
    p: &egui::Painter,
    pos: Pos2,
    text: &str,
    font: &FontId,
    color: Color32,
    track: f32,
) {
    let w = text_width(p, text, font, track);
    tracked(
        p,
        Pos2::new(pos.x - w * 0.5, pos.y),
        text,
        font,
        color,
        track,
    );
}

/// The width that [`tracked`] takes for `text`.
pub(in crate::app) fn text_width(p: &egui::Painter, text: &str, font: &FontId, track: f32) -> f32 {
    let n = text.chars().count();
    if n == 0 {
        return 0.0;
    }
    text.chars().map(|c| glyph_width(p, c, font)).sum::<f32>() + track * (n - 1) as f32
}

fn glyph_width(p: &egui::Painter, c: char, font: &FontId) -> f32 {
    p.ctx().fonts_mut(|f| {
        f.layout_no_wrap(c.to_string(), font.clone(), Color32::WHITE)
            .size()
            .x
    })
}

/// A rule [`theme::HAIRLINE`] thick, scaled by the window's size factor `k`.
pub(in crate::app) fn hairline(p: &egui::Painter, a: Pos2, b: Pos2, col: Color32, k: f32) {
    p.line_segment([a, b], Stroke::new(theme::HAIRLINE * k, col));
}

/// Draws the bin beside the Delete slot label with the window's hairline
/// stroke.
pub(in crate::app) fn trash_glyph(p: &egui::Painter, rect: Rect, col: Color32, k: f32) {
    let x = |t: f32| rect.left() + rect.width() * t;
    let y = |t: f32| rect.top() + rect.height() * t;
    let stroke = Stroke::new(theme::HAIRLINE * k, col);
    let line = |a: Pos2, b: Pos2| p.line_segment([a, b], stroke);

    line(
        Pos2::new(x(TRASH_LID_X.0), y(TRASH_LID_Y)),
        Pos2::new(x(TRASH_LID_X.1), y(TRASH_LID_Y)),
    );
    line(
        Pos2::new(x(TRASH_HANDLE_X.0), y(TRASH_LID_Y)),
        Pos2::new(x(TRASH_HANDLE_X.0), y(TRASH_HANDLE_Y)),
    );
    line(
        Pos2::new(x(TRASH_HANDLE_X.0), y(TRASH_HANDLE_Y)),
        Pos2::new(x(TRASH_HANDLE_X.1), y(TRASH_HANDLE_Y)),
    );
    line(
        Pos2::new(x(TRASH_HANDLE_X.1), y(TRASH_HANDLE_Y)),
        Pos2::new(x(TRASH_HANDLE_X.1), y(TRASH_LID_Y)),
    );
    line(
        Pos2::new(x(TRASH_BODY_TOP.0), y(TRASH_LID_Y)),
        Pos2::new(x(TRASH_BODY_BASE.0), y(TRASH_BODY_Y)),
    );
    line(
        Pos2::new(x(TRASH_BODY_TOP.1), y(TRASH_LID_Y)),
        Pos2::new(x(TRASH_BODY_BASE.1), y(TRASH_BODY_Y)),
    );
    line(
        Pos2::new(x(TRASH_BODY_BASE.0), y(TRASH_BODY_Y)),
        Pos2::new(x(TRASH_BODY_BASE.1), y(TRASH_BODY_Y)),
    );
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
pub(super) fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| lerp(x as f32, y as f32, t).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(
        m(a.r(), b.r()),
        m(a.g(), b.g()),
        m(a.b(), b.b()),
        m(a.a(), b.a()),
    )
}
