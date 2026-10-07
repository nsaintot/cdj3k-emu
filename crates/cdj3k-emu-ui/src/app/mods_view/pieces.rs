//! Drawing pieces the Mods screens share.

use super::*;

/// The footer bar, without the app version at its left end.
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

/// `BUILT-IN`, `DEV`: a word in a hairline box, left edge at `x`.
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

/// `text` cut to `max` with an ellipsis.
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

/// [`elide`] from the front: a path or a URL keeps its end.
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

/// A toolbar button, right edge at `right`: outlined, or filled with ink.
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

/// A link out of the app: glyph, label, underlined under the pointer. `at`
/// is its left end, centred on the line.
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

/// The mods icon, a cube, used by the Mods bar and the replace warning.
pub(in crate::app) fn cube_glyph(p: &egui::Painter, rect: Rect, col: Color32, k: f32) {
    glyph(p, Glyph::Cube, rect, col, k);
}

#[derive(Clone, Copy)]
pub(super) enum Glyph {
    Doc,
    Eject,
    Trash,
    Plus,
    Lock,
    Warn,
    Handle,
    Cube,
    Folder,
}

/// Paint `g` into `rect`, drawn on a 14-unit grid (the cube on 28).
pub(super) fn glyph(p: &egui::Painter, g: Glyph, rect: Rect, col: Color32, k: f32) {
    let grid = if matches!(g, Glyph::Cube) { 28.0 } else { 14.0 };
    let s = rect.width().min(rect.height()) / grid;
    let o = rect.center() - Vec2::splat(grid * 0.5 * s);
    let pt = |x: f32, y: f32| Pos2::new(o.x + x * s, o.y + y * s);
    let stroke = Stroke::new(1.1 * k, col);
    let poly = |pts: &[(f32, f32)], closed: bool| {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| pt(x, y)).collect();
        if closed {
            p.add(Shape::closed_line(v, stroke));
        } else {
            p.add(Shape::line(v, stroke));
        }
    };
    match g {
        Glyph::Doc => {
            poly(
                &[
                    (3.0, 1.8),
                    (8.4, 1.8),
                    (11.0, 4.4),
                    (11.0, 12.2),
                    (3.0, 12.2),
                ],
                true,
            );
            poly(&[(8.2, 1.8), (8.2, 4.6), (11.0, 4.6)], false);
            poly(&[(5.0, 7.0), (9.0, 7.0)], false);
            poly(&[(5.0, 9.4), (9.0, 9.4)], false);
        }
        Glyph::Eject => {
            poly(&[(7.0, 2.4), (12.0, 8.2), (2.0, 8.2)], true);
            poly(&[(2.2, 11.2), (11.8, 11.2)], false);
        }
        Glyph::Trash => trash_glyph(p, rect.shrink(rect.width() * 0.05), col, k),
        Glyph::Plus => {
            poly(&[(7.0, 1.5), (7.0, 12.5)], false);
            poly(&[(1.5, 7.0), (12.5, 7.0)], false);
        }
        Glyph::Lock => {
            poly(&[(2.5, 6.0), (11.5, 6.0), (11.5, 12.5), (2.5, 12.5)], true);
            let arc: Vec<(f32, f32)> = (0..=12)
                .map(|i| {
                    let a = std::f32::consts::PI * (i as f32 / 12.0);
                    (7.0 - 2.5 * a.cos(), 4.3 - 2.5 * a.sin())
                })
                .collect();
            let mut shackle = vec![(4.5, 6.0)];
            shackle.extend(arc);
            shackle.push((9.5, 6.0));
            poly(&shackle, false);
        }
        Glyph::Warn => {
            poly(&[(7.0, 1.4), (13.3, 12.6), (0.7, 12.6)], true);
            p.line_segment([pt(7.0, 5.4), pt(7.0, 8.8)], Stroke::new(1.3 * k, col));
            p.circle_filled(pt(7.0, 10.6), 0.8 * s, col);
        }
        Glyph::Handle => {
            for (x, y) in [
                (5.0, 3.0),
                (9.0, 3.0),
                (5.0, 7.0),
                (9.0, 7.0),
                (5.0, 11.0),
                (9.0, 11.0),
            ] {
                p.circle_filled(pt(x, y), 1.1 * s, col);
            }
        }
        Glyph::Cube => {
            let stroke = Stroke::new(1.3 * k, col);
            let v = |pts: &[(f32, f32)]| pts.iter().map(|&(x, y)| pt(x, y)).collect::<Vec<_>>();
            p.add(Shape::closed_line(
                v(&[
                    (4.0, 9.5),
                    (14.0, 4.0),
                    (24.0, 9.5),
                    (24.0, 18.5),
                    (14.0, 24.0),
                    (4.0, 18.5),
                ]),
                stroke,
            ));
            p.add(Shape::line(
                v(&[(4.0, 9.5), (14.0, 15.0), (24.0, 9.5)]),
                stroke,
            ));
            p.add(Shape::line(v(&[(14.0, 15.0), (14.0, 24.0)]), stroke));
        }
        Glyph::Folder => {
            poly(
                &[
                    (2.0, 3.2),
                    (6.0, 3.2),
                    (7.2, 4.6),
                    (12.0, 4.6),
                    (12.0, 11.2),
                    (2.0, 11.2),
                ],
                true,
            );
            poly(&[(7.0, 6.6), (7.0, 9.6)], false);
            poly(&[(5.5, 8.1), (8.5, 8.1)], false);
        }
    }
}

pub(super) fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}
