//! Release notes from the index (GitHub Markdown): headings, nested bullets
//! and paragraphs; inline formatting is dropped.

use egui::{RichText, Stroke, Vec2};

use super::super::theme::{self, Palette};

/// Box height, at the reference width.
const NOTES_H: f32 = 150.0;
/// Indent per bullet level, at the reference width.
const NOTES_INDENT: f32 = 14.0;

pub(super) fn notes_box(ui: &mut egui::Ui, markdown: &str, pal: &Palette, k: f32) {
    if markdown.trim().is_empty() {
        return;
    }
    ui.add_space(6.0 * k);
    egui::Frame::NONE
        .fill(pal.field)
        .stroke(Stroke::new(theme::HAIRLINE * k, pal.line))
        .corner_radius(theme::ROUND * k)
        .inner_margin(egui::epaint::MarginF32::symmetric(12.0 * k, 10.0 * k))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Plain text rows; the window style sets rows to field height.
            ui.spacing_mut().interact_size.y = 0.0;
            ui.spacing_mut().item_spacing = Vec2::new(6.0 * k, 3.0 * k);
            egui::ScrollArea::vertical()
                .max_height(NOTES_H * k)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    let body = |text: String| {
                        egui::Label::new(
                            RichText::new(text)
                                .size(theme::BODY_FONT * k)
                                .color(pal.muted),
                        )
                        .wrap()
                    };
                    for line in notes_lines(markdown) {
                        match line {
                            NotesLine::Heading(text) => {
                                ui.add_space(3.0 * k);
                                ui.label(
                                    RichText::new(text)
                                        .size(theme::BODY_FONT * k * 1.08)
                                        .strong()
                                        .color(pal.ink),
                                );
                            }
                            NotesLine::Bullet(depth, text) => {
                                ui.horizontal_top(|ui| {
                                    ui.add_space(NOTES_INDENT * k * depth as f32);
                                    ui.label(
                                        RichText::new("•")
                                            .size(theme::BODY_FONT * k)
                                            .color(pal.dim),
                                    );
                                    ui.add(body(text));
                                });
                            }
                            NotesLine::Text(text) => {
                                ui.add(body(text));
                            }
                            NotesLine::Gap => ui.add_space(4.0 * k),
                        }
                    }
                });
        });
}

#[derive(Debug, PartialEq, Eq)]
enum NotesLine {
    Heading(String),
    /// Nesting depth (two spaces per level) and text.
    Bullet(usize, String),
    Text(String),
    /// A blank line; consecutive blanks collapse to one.
    Gap,
}

/// Lines of a GitHub release body to draw.
fn notes_lines(markdown: &str) -> Vec<NotesLine> {
    let plain = |s: &str| s.replace("**", "").replace('`', "").trim().to_owned();
    let mut out: Vec<NotesLine> = Vec::new();
    for raw in markdown.lines() {
        let line = raw.trim_end();
        let text = line.trim_start();
        if text.starts_with("<!--") {
            continue;
        }
        let next = if text.is_empty() {
            // Skip leading and repeated blank lines.
            if matches!(out.last(), None | Some(NotesLine::Gap)) {
                continue;
            }
            NotesLine::Gap
        } else if let Some(h) = heading(text) {
            NotesLine::Heading(plain(h))
        } else if let Some(b) = text.strip_prefix("* ").or_else(|| text.strip_prefix("- ")) {
            let indent = line.len() - text.len();
            NotesLine::Bullet(indent / 2, plain(b))
        } else {
            NotesLine::Text(plain(text))
        };
        out.push(next);
    }
    if matches!(out.last(), Some(NotesLine::Gap)) {
        out.pop();
    }
    out
}

/// A heading's text: one to six `#` and a space before it.
fn heading(text: &str) -> Option<&str> {
    let rest = text.trim_start_matches('#');
    let level = text.len() - rest.len();
    ((1..=6).contains(&level))
        .then(|| rest.strip_prefix(' '))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::{notes_lines, NotesLine};

    #[test]
    fn generated_notes_become_headings_bullets_and_text() {
        let md = "# What's Changed\r\n\r\n## Added\r\n#123 is a PR\r\n* [platform] **Windows host** (#17)\r\n  * (sub-bullets unchanged)\r\n\r\n\r\n**Full Changelog**: https://github.com/x/y/compare/v0.2.0...v0.3.0\r\n";
        assert_eq!(
            notes_lines(md),
            vec![
                NotesLine::Heading("What's Changed".into()),
                NotesLine::Gap,
                NotesLine::Heading("Added".into()),
                NotesLine::Text("#123 is a PR".into()),
                NotesLine::Bullet(0, "[platform] Windows host (#17)".into()),
                NotesLine::Bullet(1, "(sub-bullets unchanged)".into()),
                NotesLine::Gap,
                NotesLine::Text(
                    "Full Changelog: https://github.com/x/y/compare/v0.2.0...v0.3.0".into()
                ),
            ]
        );
    }
}
