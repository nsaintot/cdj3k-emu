//! A mod's log page: its description, version, author and project page, how
//! it ran on the last boot, and its journal.

use super::*;
use crate::app::table;

pub(super) fn draw_log(ui: &mut egui::Ui, page: &mut ModsPage) -> Option<ModsAction> {
    let Some(log) = page.log.clone() else {
        return Some(ModsAction::CloseLog);
    };
    let k = k_of(ui.max_rect());
    let pal = theme::palette(ui.ctx());
    let area = ui.max_rect();

    // The title, the DEV chip after it, and the status pill at the right end;
    // the title is shortened to fit between them.
    let (pill_text, pill_col) = pill(&log, pal);
    let font = FontId::proportional(STATUS_FONT * k);
    let title_font = FontId::proportional(theme::TITLE_FONT * k);
    let (title, title_w, pw) = {
        let p = ui.painter();
        let pw = text_width(p, &pill_text, &font, STATUS_TRACK * k) + 2.0 * PILL_PAD * k;
        let chip_w = if log.dev {
            chip_width(p, "DEV", k) + 10.0 * k
        } else {
            0.0
        };
        let room = area.width() - 2.0 * theme::GUTTER * k - pw - chip_w - 16.0 * k;
        let title = elide(p, &log.name, &title_font, room);
        let title_w = text_width(p, &title, &title_font, 0.0);
        (title, title_w, pw)
    };
    let head = Header {
        slot: Some(page.slot),
        model: page.model,
        release: page.release.as_deref(),
        state: None,
        title: &title,
        sub: LOG_SUB,
        switchable: false,
        warn: false,
    };
    let body = draw_header(ui, &head).body;
    let (space, bar) = footer_split(body, k);
    let p = ui.painter();
    let title_cy = area.top() + (theme::TOPBAR_H + 20.0) * k + theme::TITLE_FONT * k * 0.56;
    if log.dev {
        chip(
            p,
            area.left() + theme::GUTTER * k + title_w + 10.0 * k,
            title_cy,
            "DEV",
            k,
            pal,
        );
    }
    let pill_rect = Rect::from_min_size(
        Pos2::new(
            area.right() - theme::GUTTER * k - pw,
            title_cy - PILL_H * k * 0.5,
        ),
        Vec2::new(pw, PILL_H * k),
    );
    p.rect_stroke(
        pill_rect,
        theme::ROUND_CHIP * k,
        Stroke::new(theme::HAIRLINE * k, pill_col),
        egui::StrokeKind::Middle,
    );
    tracked(
        p,
        Pos2::new(pill_rect.left() + PILL_PAD * k, pill_rect.center().y),
        &pill_text,
        &font,
        pill_col,
        STATUS_TRACK * k,
    );

    let left = space.left() + theme::GUTTER * k;
    let width = space.width() - 2.0 * theme::GUTTER * k;
    let mut y = space.top();
    let mut action = None;
    // No details box when mod.toml cannot be read.
    let lines = meta_lines(&log);
    if !lines.is_empty() {
        let meta = Rect::from_min_size(
            Pos2::new(left, y),
            Vec2::new(width, (lines.len() as f32 * META_ROW + 2.0 * META_PAD) * k),
        );
        action = draw_meta(ui, meta, &lines, k, pal);
        y = meta.bottom() + SECTION_GAP * k;
    }
    let dl = Rect::from_min_size(Pos2::new(left, y), Vec2::new(width, DL_H * k));
    draw_facts(ui.painter(), dl, &log, k, pal);
    y = dl.bottom() + SECTION_GAP * k;

    let box_rect = Rect::from_min_max(
        Pos2::new(left, y),
        Pos2::new(left + width, space.bottom() - LOG_BOTTOM * k),
    );
    draw_journal(ui, box_rect, &log, k, pal);

    // The footer has the Back and Copy log buttons.
    let inner = plain_footer(ui, bar, k, pal);
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
        theme::apply_setup_style(ui, pal);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(text) = &log.text {
                let w = button_text_width(ui, "Copy log", k) + COPY_PAD * k;
                if theme::secondary(ui, [w, theme::BUTTON_SIZE[1] * k], "Copy log", pal).clicked() {
                    ui.ctx().copy_text(text.clone());
                }
            }
            if theme::secondary(ui, [BACK_W * k, theme::BUTTON_SIZE[1] * k], "Back", pal).clicked()
            {
                action = Some(ModsAction::CloseLog);
            }
        });
    });
    action
}

/// The font and colour of a value in the details box.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Value {
    Text,
    Mono,
    /// A web address, drawn as a link that opens in the browser.
    Link,
}

/// The DESCRIPTION, VERSION, AUTHOR and PROJECT lines, for the fields that
/// mod.toml fills in. A PROJECT that is an http or https address is a link;
/// anything else is plain text.
pub(super) fn meta_lines(log: &LogView) -> Vec<(&'static str, String, Value)> {
    let mut lines = Vec::new();
    if let Some(d) = &log.description {
        lines.push(("DESCRIPTION", d.replace('\n', " "), Value::Text));
    }
    if let Some(v) = &log.version {
        lines.push(("VERSION", v.clone(), Value::Mono));
    }
    if let Some(a) = &log.author {
        lines.push(("AUTHOR", a.clone(), Value::Text));
    }
    if let Some(u) = &log.url {
        let kind = if web_url(u) { Value::Link } else { Value::Mono };
        lines.push(("PROJECT", u.clone(), kind));
    }
    lines
}

/// Whether `url` is an http or https address with a host.
pub(super) fn web_url(url: &str) -> bool {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    let host = rest.and_then(|r| r.split(['/', '?', '#']).next());
    host.is_some_and(|h| {
        !h.is_empty()
            && h.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
    }) && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Draws the details box, one line per detail with its label on the left.
/// Hovering a shortened value shows it in full.
fn draw_meta(
    ui: &egui::Ui,
    rect: Rect,
    lines: &[(&'static str, String, Value)],
    k: f32,
    pal: &Palette,
) -> Option<ModsAction> {
    let p = ui.painter();
    plate(p, rect, k, pal);
    let mut action = None;
    let label_font = FontId::proportional(STATUS_FONT * k);
    let x = rect.left() + DL_PAD * k;
    let vx = x + META_LABEL * k;
    let room = rect.right() - DL_PAD * k - vx;
    for (i, (label, value, kind)) in lines.iter().enumerate() {
        let cy = rect.top() + META_PAD * k + (i as f32 + 0.5) * META_ROW * k;
        tracked(
            p,
            Pos2::new(x, cy),
            label,
            &label_font,
            pal.dim,
            STATUS_TRACK * k,
        );
        let font = match kind {
            Value::Text => FontId::proportional(DL_VALUE_FONT * k),
            _ => FontId::monospace(DL_VALUE_FONT * k),
        };
        let shown = elide(p, value, &font, room);
        let col = if *kind == Value::Link {
            pal.link
        } else {
            pal.ink
        };
        let at = p.text(Pos2::new(vx, cy), Align2::LEFT_CENTER, &shown, font, col);
        let id = ui.id().with(("mod_meta", *label));
        if *kind == Value::Link {
            let resp = theme::pointer(ui.interact(at, id, Sense::click()));
            if resp.hovered() {
                let uy = cy + DL_VALUE_FONT * k * 0.62;
                p.line_segment(
                    [Pos2::new(at.left(), uy), Pos2::new(at.right(), uy)],
                    Stroke::new(theme::HAIRLINE * k, pal.link),
                );
            }
            let resp = if shown != *value {
                resp.on_hover_text(value)
            } else {
                resp
            };
            if resp.clicked() {
                action = Some(ModsAction::Open(value.clone()));
            }
        } else if shown != *value {
            ui.interact(at, id, Sense::hover()).on_hover_text(value);
        }
    }
    action
}

fn plate(p: &egui::Painter, rect: Rect, k: f32, pal: &Palette) {
    p.rect_filled(rect, theme::ROUND * k, pal.plate);
    p.rect_stroke(
        rect,
        theme::ROUND * k,
        Stroke::new(theme::HAIRLINE * k, pal.line),
        egui::StrokeKind::Middle,
    );
}

pub(super) fn pill(log: &LogView, pal: &Palette) -> (String, Color32) {
    match &log.status {
        RowStatus::Loaded | RowStatus::Declared(_) => ("LOADED".into(), pal.ok),
        RowStatus::Failed(d) => (
            format!("BOOT SCRIPT FAILED · {}", d.to_uppercase()),
            pal.danger,
        ),
        RowStatus::Incompatible(_) => ("INCOMPATIBLE".into(), pal.danger),
        RowStatus::Invalid(_) => ("INVALID".into(), pal.danger),
        RowStatus::NotRun => ("NOT IN THE LAST BOOT".into(), pal.faint),
        RowStatus::Unreadable => ("UNRECOGNIZED REPORT".into(), pal.warn),
    }
}

/// ORDER, BOOT SCRIPT and LIBRARIES, side by side.
pub(super) fn draw_facts(p: &egui::Painter, rect: Rect, log: &LogView, k: f32, pal: &Palette) {
    plate(p, rect, k, pal);
    let ran = matches!(
        log.status,
        RowStatus::Loaded | RowStatus::Declared(_) | RowStatus::Failed(_)
    );
    let order = log
        .order
        .map_or("—".to_string(), |(i, n)| format!("{i} of {n}"));
    let (script, script_col) = match &log.status {
        RowStatus::Loaded => ("exit 0".to_string(), pal.ink),
        RowStatus::Declared(_) => ("none".to_string(), pal.faint),
        RowStatus::Failed(d) => (d.clone(), pal.danger),
        RowStatus::Unreadable => ("—".to_string(), pal.faint),
        _ => ("did not run".to_string(), pal.faint),
    };
    let (libs, libs_col) = match (ran, log.libs) {
        (false, _) => ("—".to_string(), pal.faint),
        (true, 0) => ("none".to_string(), pal.ink),
        (true, n) => (format!("{n} loaded"), pal.ink),
    };
    let cells = [
        ("ORDER", order, pal.ink),
        ("BOOT SCRIPT", script, script_col),
        ("LIBRARIES", libs, libs_col),
    ];
    let columns = table::columns(rect, &[table::Width::Fill; 3], k);
    for (i, ((label, value, col), cell)) in cells.iter().zip(columns).enumerate() {
        let x = cell.left();
        if i > 0 {
            hairline(
                p,
                Pos2::new(x, rect.top()),
                Pos2::new(x, rect.bottom()),
                pal.line,
                k,
            );
        }
        tracked(
            p,
            Pos2::new(x + DL_PAD * k, rect.top() + 16.0 * k),
            label,
            &FontId::proportional(STATUS_FONT * k),
            pal.dim,
            STATUS_TRACK * k,
        );
        p.text(
            Pos2::new(x + DL_PAD * k, rect.top() + 34.0 * k),
            Align2::LEFT_CENTER,
            value,
            FontId::monospace(DL_VALUE_FONT * k),
            *col,
        );
    }
}

pub(super) fn draw_journal(ui: &mut egui::Ui, rect: Rect, log: &LogView, k: f32, pal: &Palette) {
    let p = ui.painter();
    p.rect_filled(rect, theme::ROUND * k, pal.field);
    p.rect_stroke(
        rect,
        theme::ROUND * k,
        Stroke::new(theme::HAIRLINE * k, pal.line),
        egui::StrokeKind::Middle,
    );
    let inner = rect.shrink(LOG_PAD * k);
    let font = FontId::monospace(LOG_FONT * k);
    let mut job = egui::text::LayoutJob::default();
    match &log.text {
        Some(text) => {
            for line in text.lines() {
                let col = if line.starts_with("-- ") {
                    pal.faint
                } else if line.contains(" dropped: ")
                    || (line.starts_with("loader.sh exited with status ") && !line.ends_with(" 0"))
                    || line.starts_with("loader.sh stopped")
                {
                    pal.danger
                } else {
                    pal.ink
                };
                job.append(
                    &format!("{line}\n"),
                    0.0,
                    egui::TextFormat {
                        font_id: font.clone(),
                        color: col,
                        line_height: Some(LOG_FONT * k * 1.65),
                        ..Default::default()
                    },
                );
            }
        }
        None => {
            // For a mod that did not run, the box shows the full reason.
            let (text, color) = match &log.status {
                RowStatus::Invalid(e) => (e.clone(), pal.danger),
                RowStatus::Incompatible(d) => (
                    format!("Incompatible with this slot's firmware: {d}"),
                    pal.danger,
                ),
                _ => (
                    format!("{} has no journal from the last boot.", log.name),
                    pal.faint,
                ),
            };
            job.append(
                &text,
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    color,
                    line_height: Some(LOG_FONT * k * 1.65),
                    ..Default::default()
                },
            );
        }
    }
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add(egui::Label::new(job).extend());
            });
    });
}
