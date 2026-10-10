//! Drawing pieces the Mods screens share.

use super::*;

// ── Text in a row ─────────────────────────────────────────────────────────────
pub(super) const NAME_FONT: f32 = 13.5;
pub(super) const MONO_FONT: f32 = 10.5;
const LINE_GAP: f32 = 3.0;
pub(super) const STATUS_FONT: f32 = 9.0;
pub(super) const STATUS_TRACK: f32 = 1.26;
/// The font size of a note line and of a mod's author.
pub(super) const NOTE_FONT: f32 = 12.0;
const CHIP_H: f32 = 16.0;
const CHIP_PAD: f32 = 5.0;
const CHIP_FONT: f32 = 8.5;
const CHIP_TRACK: f32 = 1.2;

// ── Buttons ───────────────────────────────────────────────────────────────────
const TOOL_PAD: f32 = 14.0;
const TOOL_GLYPH: f32 = 11.0;
const TOOL_GLYPH_GAP: f32 = 7.0;
const ACTION_GLYPH: f32 = 14.0;
/// The width of Back and of the dialogs' buttons.
pub(super) const BACK_W: f32 = 100.0;
/// The room on both sides of the label of a button sized to its text.
pub(super) const COPY_PAD: f32 = 32.0;

// ── Dashes and links ──────────────────────────────────────────────────────────
const DASH: f32 = 4.0;
const DASH_GAP: f32 = 3.0;
const LINK_FONT: f32 = 13.0;
const LINK_GLYPH: f32 = 14.0;
pub(super) const LINKS_H: f32 = 20.0;

/// Draws the footer bar without the app version at its left end.
pub(super) fn plain_footer(ui: &egui::Ui, bar: Rect, k: f32, pal: &Palette) -> Rect {
    let p = ui.painter();
    p.rect_filled(bar, 0.0, pal.plate);
    hairline(p, bar.left_top(), bar.right_top(), pal.line, k);
    Rect::from_center_size(
        bar.center(),
        Vec2::new(
            bar.width() - 2.0 * theme::GUTTER * k,
            theme::BUTTON_SIZE[1] * k,
        ),
    )
}

pub(super) fn button_text_width(ui: &egui::Ui, label: &str, k: f32) -> f32 {
    text_width(
        ui.painter(),
        label,
        &FontId::proportional(theme::BUTTON_FONT * k),
        0.0,
    )
}

/// The tops of a row's two lines, centred on `cy`.
pub(super) fn two_lines(cy: f32, k: f32) -> (f32, f32) {
    let h = NAME_FONT * k * 1.2 + LINE_GAP * k + MONO_FONT * k * 1.2;
    let top = cy - h * 0.5;
    (
        top + NAME_FONT * k * 0.6,
        top + NAME_FONT * k * 1.2 + LINE_GAP * k + MONO_FONT * k * 0.6,
    )
}

pub(super) fn name_text(p: &egui::Painter, at: Pos2, text: &str, col: Color32, k: f32) -> f32 {
    p.text(
        at,
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(NAME_FONT * k),
        col,
    )
    .width()
}

pub(super) fn mono(p: &egui::Painter, at: Pos2, text: &str, col: Color32, k: f32) {
    p.text(
        at,
        Align2::LEFT_CENTER,
        text,
        FontId::monospace(MONO_FONT * k),
        col,
    );
}

pub(super) fn status_word(p: &egui::Painter, at: Pos2, text: &str, col: Color32, k: f32) {
    tracked(
        p,
        at,
        text,
        &FontId::proportional(STATUS_FONT * k),
        col,
        STATUS_TRACK * k,
    );
}

pub(super) fn chip_width(p: &egui::Painter, text: &str, k: f32) -> f32 {
    text_width(
        p,
        text,
        &FontId::proportional(CHIP_FONT * k),
        CHIP_TRACK * k,
    ) + 2.0 * CHIP_PAD * k
}

/// Draws a word such as `BUILT-IN` or `DEV` in a hairline box whose left
/// edge is at `x`.
pub(super) fn chip(p: &egui::Painter, x: f32, cy: f32, text: &str, k: f32, pal: &Palette) {
    let w = chip_width(p, text, k);
    let rect = Rect::from_min_size(
        Pos2::new(x, cy - CHIP_H * k * 0.5),
        Vec2::new(w, CHIP_H * k),
    );
    p.rect_stroke(
        rect,
        theme::ROUND_CHIP * k,
        Stroke::new(theme::HAIRLINE * k, pal.line_strong),
        egui::StrokeKind::Middle,
    );
    tracked(
        p,
        Pos2::new(rect.left() + CHIP_PAD * k, cy),
        text,
        &FontId::proportional(CHIP_FONT * k),
        pal.muted,
        CHIP_TRACK * k,
    );
}

/// Returns `text` cut to the width `max`, with an ellipsis.
pub(super) fn elide(p: &egui::Painter, text: &str, font: &FontId, max: f32) -> String {
    if text_width(p, text, font, 0.0) <= max {
        return text.to_owned();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "…";
        if text_width(p, &candidate, font, 0.0) <= max {
            return candidate;
        }
    }
    "…".into()
}

/// Like [`elide`], but cuts from the front, so a path or a URL keeps its end.
pub(super) fn elide_start(p: &egui::Painter, text: &str, font: &FontId, max: f32) -> String {
    if text_width(p, text, font, 0.0) <= max {
        return text.to_owned();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.remove(0);
        let candidate: String = "…".to_string() + &chars.iter().collect::<String>();
        if text_width(p, &candidate, font, 0.0) <= max {
            return candidate;
        }
    }
    "…".into()
}

pub(super) fn checkbox(p: &egui::Painter, rect: Rect, on: bool, hot: bool, k: f32, pal: &Palette) {
    let round = theme::ROUND_CHIP * k;
    if on {
        p.rect_filled(rect, round, if hot { pal.ink_hover } else { pal.ink });
        let s = rect.width();
        let pts = vec![
            Pos2::new(rect.left() + s * 0.24, rect.top() + s * 0.52),
            Pos2::new(rect.left() + s * 0.42, rect.top() + s * 0.70),
            Pos2::new(rect.left() + s * 0.77, rect.top() + s * 0.32),
        ];
        p.add(Shape::line(pts, Stroke::new(1.6 * k, pal.on_ink)));
    } else {
        p.rect_filled(rect, round, pal.plate);
        p.rect_stroke(
            rect,
            round,
            Stroke::new(
                theme::HAIRLINE * k,
                if hot { pal.ink } else { pal.line_strong },
            ),
            egui::StrokeKind::Middle,
        );
    }
}

pub(super) fn tool_width(p: &egui::Painter, label: &str, plus: bool, k: f32) -> f32 {
    let mut w = text_width(p, label, &FontId::proportional(theme::BUTTON_FONT * k), 0.0)
        + 2.0 * TOOL_PAD * k;
    if plus {
        w += (TOOL_GLYPH + TOOL_GLYPH_GAP) * k;
    }
    w
}

/// Draws a toolbar button with its right edge at `right`, either outlined or
/// filled with ink.
#[allow(clippy::too_many_arguments)]
pub(super) fn tool_button(
    ui: &egui::Ui,
    right: f32,
    cy: f32,
    h: f32,
    label: &str,
    plus: bool,
    filled: bool,
    k: f32,
    pal: &Palette,
) -> egui::Response {
    let p = ui.painter();
    let w = tool_width(p, label, plus, k);
    let rect = Rect::from_min_size(Pos2::new(right - w, cy - h * 0.5), Vec2::new(w, h));
    let resp = theme::pointer(ui.interact(rect, ui.id().with(("tool", label)), Sense::click()));
    let hot = resp.hovered();
    let (fill, edge, fg) = match (filled, hot) {
        (true, false) => (pal.ink, pal.ink, pal.on_ink),
        (true, true) => (pal.ink_hover, pal.ink_hover, pal.on_ink),
        (false, false) => (pal.plate, pal.line_strong, pal.ink),
        (false, true) => (pal.plate_hover, pal.ink, pal.ink),
    };
    p.rect_filled(rect, theme::ROUND * k, fill);
    p.rect_stroke(
        rect,
        theme::ROUND * k,
        Stroke::new(theme::HAIRLINE * k, edge),
        egui::StrokeKind::Middle,
    );
    let mut x = rect.left() + TOOL_PAD * k;
    if plus {
        let g = Rect::from_min_size(
            Pos2::new(x, cy - TOOL_GLYPH * k * 0.5),
            Vec2::splat(TOOL_GLYPH * k),
        );
        glyph(p, Glyph::Plus, g, fg, k);
        x += (TOOL_GLYPH + TOOL_GLYPH_GAP) * k;
    }
    p.text(
        Pos2::new(x, cy),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(theme::BUTTON_FONT * k),
        fg,
    );
    resp
}

/// A 40 x 32 button with only a glyph on it.
pub(super) fn glyph_button(
    ui: &egui::Ui,
    rect: Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    g: Glyph,
    col: Color32,
    k: f32,
    pal: &Palette,
) -> egui::Response {
    let resp = theme::pointer(ui.interact(rect, ui.id().with(id), Sense::click()));
    let hot = resp.hovered();
    let p = ui.painter();
    p.rect_filled(
        rect,
        theme::ROUND * k,
        if hot { pal.plate_hover } else { pal.plate },
    );
    p.rect_stroke(
        rect,
        theme::ROUND * k,
        Stroke::new(
            theme::HAIRLINE * k,
            if hot { pal.ink } else { pal.line_strong },
        ),
        egui::StrokeKind::Middle,
    );
    glyph(
        p,
        g,
        Rect::from_center_size(rect.center(), Vec2::splat(ACTION_GLYPH * k)),
        col,
        k,
    );
    resp
}

/// Draws a link that opens outside the app: a glyph and a label, underlined
/// while the pointer is over it. `at` is its left end, centred vertically on
/// the line.
pub(super) fn link(
    ui: &egui::Ui,
    at: Pos2,
    label: &str,
    g: Glyph,
    k: f32,
    pal: &Palette,
) -> egui::Response {
    let p = ui.painter();
    let font = FontId::proportional(LINK_FONT * k);
    let tw = text_width(p, label, &font, 0.0);
    let gw = LINK_GLYPH * k;
    let rect = Rect::from_min_size(
        Pos2::new(at.x, at.y - LINKS_H * k * 0.5),
        Vec2::new(gw + 8.0 * k + tw, LINKS_H * k),
    );
    let resp = theme::pointer(ui.interact(rect, ui.id().with(("link", label)), Sense::click()));
    glyph(
        p,
        g,
        Rect::from_center_size(Pos2::new(at.x + gw * 0.5, at.y), Vec2::splat(gw)),
        pal.link,
        k,
    );
    let tx = at.x + gw + 8.0 * k;
    p.text(
        Pos2::new(tx, at.y),
        Align2::LEFT_CENTER,
        label,
        font,
        pal.link,
    );
    if resp.hovered() {
        let uy = at.y + LINK_FONT * k * 0.62;
        p.line_segment(
            [Pos2::new(tx, uy), Pos2::new(tx + tw, uy)],
            Stroke::new(theme::HAIRLINE * k, pal.link),
        );
    }
    resp
}

pub(in crate::app) fn dashed_rect(p: &egui::Painter, r: Rect, col: Color32, k: f32) {
    let stroke = Stroke::new(theme::HAIRLINE * k, col);
    let pts = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    p.extend(Shape::dashed_line(&pts, stroke, DASH * k, DASH_GAP * k));
}

pub(super) fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}
