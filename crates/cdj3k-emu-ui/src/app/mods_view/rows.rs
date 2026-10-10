//! The table of mods on the list screen: the emulator's shim pinned on top,
//! then one row per mod, reordered by its drag handle.

use super::*;
use crate::app::table::{from_right, Column, Line, Row, Table, Width};

const HEAD_H: f32 = 32.0;
const ROW_H: f32 = 54.0;
const ROW_PAD: f32 = 12.0;
const COL_ON: f32 = 60.0;
const COL_STATUS: f32 = 210.0;
const HANDLE_W: f32 = 18.0;
const HANDLE_H: f32 = 28.0;
const ON_GAP: f32 = 10.0;
const CHECK: f32 = 15.0;
/// The gap after a name, before its chip or its author.
const CHIP_GAP: f32 = 8.0;
/// The size of the remove and log buttons at a row's right end.
const ACTION: [f32; 2] = [40.0, 32.0];
const ACTION_GAP: f32 = 8.0;

const ON: usize = 0;
const MOD: usize = 1;
const STATUS: usize = 2;
const ACTIONS: usize = 3;
const COLUMNS: [Column; 4] = [
    Column {
        label: "ON",
        width: Width::Fixed(COL_ON),
    },
    Column {
        label: "MOD",
        width: Width::Fill,
    },
    Column {
        label: "STATUS",
        width: Width::Fixed(COL_STATUS),
    },
    Column {
        label: "",
        width: Width::Fixed(2.0 * ACTION[0] + 2.0 * ACTION_GAP),
    },
];

pub(super) fn draw_table(
    ui: &mut egui::Ui,
    page: &ModsPage,
    area: Rect,
    k: f32,
    pal: &Palette,
) -> Option<ModsAction> {
    let table = Table {
        id: "mods_table",
        columns: &COLUMNS,
        head_h: HEAD_H,
        row_h: ROW_H,
        pad: ROW_PAD,
        k,
    };
    let mut action = None;
    let rows = &page.rows;
    let key = |m: &ModRow| egui::Id::new(&m.name);
    let moved = table.show(ui, area, pal, true, rows, key, |row, line| match line {
        Line::Pinned => draw_shim_row(row, k, pal),
        Line::Item(m) => {
            if let Some(a) = draw_row(row, m, k, pal) {
                action = Some(a);
            }
        }
    });
    moved
        .map(|(from, to)| ModsAction::Reorder(from, to))
        .or(action)
}

/// Where the checkbox sits in the ON cell, after the drag handle.
fn check_rect(on: Rect, k: f32) -> Rect {
    Rect::from_center_size(
        Pos2::new(
            on.left() + (HANDLE_W + ON_GAP + CHECK * 0.5) * k,
            on.center().y,
        ),
        Vec2::splat(CHECK * k),
    )
}

/// The emulator's own shim, pinned as the first row: it ships with the app
/// and is not a mod.
fn draw_shim_row(row: &mut Row, k: f32, pal: &Palette) {
    let p = row.painter();
    p.rect_filled(row.rect, 0.0, lerp_color(pal.plate, pal.paper, 0.45));
    glyph(p, Glyph::Lock, check_rect(row.cells[ON], k), pal.dim, k);
    let (top, bottom) = two_lines(row.rect.center().y, k);
    let x = row.cells[MOD].left();
    let w = name_text(p, Pos2::new(x, top), "Emulator shim", pal.ink, k);
    chip(p, x + w + CHIP_GAP * k, top, "BUILT-IN", k, pal);
    mono(
        p,
        Pos2::new(x, bottom),
        &format!("deck_shim.so · v{}", cdj3k_emu_platform::app_meta::VERSION),
        pal.faint,
        k,
    );
    let sx = row.cells[STATUS].left();
    status_word(p, Pos2::new(sx, top), "DEFAULT", pal.faint, k);
}

fn draw_row(row: &mut Row, m: &ModRow, k: f32, pal: &Palette) -> Option<ModsAction> {
    let on = row.cells[ON];
    let handle = Rect::from_center_size(
        Pos2::new(on.left() + HANDLE_W * k * 0.5, on.center().y),
        Vec2::new(HANDLE_W * k, HANDLE_H * k),
    );
    row.handle(handle, |p, r, hot| {
        let col = if hot { pal.ink } else { pal.faint };
        glyph(p, Glyph::Handle, r.shrink(2.0 * k), col, k);
    });
    let toggled = row_switch(row.ui, check_rect(on, k), m, pal, k);
    let asked = row_buttons(row.ui, row.cells[ACTIONS], m, pal, k);
    row_name(row, m, pal, k);
    row_status(row, m, pal, k);
    asked.or(toggled)
}

/// Draws the mod's checkbox, or a warning when the slot's firmware cannot run
/// the mod. The mod keeps its on/off setting either way.
fn row_switch(ui: &egui::Ui, check: Rect, m: &ModRow, pal: &Palette, k: f32) -> Option<ModsAction> {
    let p = ui.painter();
    if let RowStatus::Incompatible(_) = m.status {
        glyph(p, Glyph::Warn, check, pal.danger, k);
        ui.interact(check, ui.id().with("mod_warn"), Sense::hover())
            .on_hover_text(format!(
                "{} is incompatible with this slot's firmware",
                m.name
            ));
        return None;
    }
    let resp = theme::pointer(ui.interact(
        check.expand(4.0 * k),
        ui.id().with("mod_on"),
        Sense::click(),
    ));
    checkbox(p, check, m.enabled, resp.hovered(), k, pal);
    resp.clicked().then(|| ModsAction::Toggle(m.name.clone()))
}

/// Draws the Remove button (Eject for a folder used in place) and the Log
/// button at the row's right end.
fn row_buttons(ui: &egui::Ui, cell: Rect, m: &ModRow, pal: &Palette, k: f32) -> Option<ModsAction> {
    let mut action = None;
    let size = Vec2::new(ACTION[0] * k, ACTION[1] * k);
    let [remove, log] = from_right(cell, size, ACTION_GAP * k, 2)[..] else {
        unreachable!()
    };
    if m.dev {
        if glyph_button(ui, remove, "mod_eject", Glyph::Eject, pal.ink, k, pal)
            .on_hover_text("Eject (the folder stays on disk)")
            .clicked()
        {
            action = Some(ModsAction::Eject(m.name.clone()));
        }
    } else if glyph_button(ui, remove, "mod_remove", Glyph::Trash, pal.danger, k, pal)
        .on_hover_text("Remove")
        .clicked()
    {
        action = Some(ModsAction::AskRemove(m.name.clone()));
    }
    if glyph_button(ui, log, "mod_log", Glyph::Doc, pal.ink, k, pal)
        .on_hover_text("Log")
        .clicked()
    {
        action = Some(ModsAction::OpenLog(m.name.clone()));
    }
    action
}

/// Draws the MOD cell: the name, its DEV chip and its author on the first
/// line, and its source on the second. The name gets at least half of the
/// width left after the DEV chip; the author gets the rest.
fn row_name(row: &Row, m: &ModRow, pal: &Palette, k: f32) {
    let name = row.cells[MOD];
    let ui: &egui::Ui = row.ui;
    let p = ui.painter();
    let (top, bottom) = two_lines(row.rect.center().y, k);
    let name_w = name.width() - ROW_PAD * k;
    let chip_w = if m.dev {
        chip_width(p, "DEV", k) + CHIP_GAP * k
    } else {
        0.0
    };
    let font = FontId::proportional(NAME_FONT * k);
    let author_font = FontId::proportional(NOTE_FONT * k);
    let author = m.author.as_ref().map(|a| format!("by {a}"));
    let author_w = author
        .as_ref()
        .map_or(0.0, |a| text_width(p, a, &author_font, 0.0) + CHIP_GAP * k);
    let room = name_w - chip_w;
    let label = elide(p, &m.name, &font, (room - author_w).max(room * 0.5));
    let name_col = if m.enabled { pal.ink } else { pal.faint };
    let w = name_text(p, Pos2::new(name.left(), top), &label, name_col, k);
    if m.dev {
        chip(p, name.left() + w + CHIP_GAP * k, top, "DEV", k, pal);
    }
    // The tooltip over the whole MOD cell: the full name if it was shortened,
    // then the description.
    let full = (label != m.name).then_some(m.name.as_str());
    let hover: Vec<&str> = full.into_iter().chain(m.description.as_deref()).collect();
    if !hover.is_empty() && !row.dragged {
        let at = Rect::from_x_y_ranges(name.x_range(), row.rect.y_range());
        ui.interact(at, ui.id().with("mod_name"), Sense::hover())
            .on_hover_text(hover.join("\n"));
    }
    if let Some(author) = &author {
        let x = name.left() + w + chip_w + CHIP_GAP * k;
        p.text(
            Pos2::new(x, top),
            Align2::LEFT_CENTER,
            elide(p, author, &author_font, name.left() + name_w - x),
            author_font,
            pal.muted,
        );
    }
    let mono_font = FontId::monospace(MONO_FONT * k);
    mono(
        p,
        Pos2::new(name.left(), bottom),
        &source_line(p, m, &mono_font, name_w),
        pal.faint,
        k,
    );
}

/// Draws the STATUS cell: the status word, and the detail under it.
/// Hovering shows the full reason when the row shows only part of it.
fn row_status(row: &Row, m: &ModRow, pal: &Palette, k: f32) {
    let status = row.cells[STATUS];
    let ui: &egui::Ui = row.ui;
    let p = ui.painter();
    let (top, bottom) = two_lines(row.rect.center().y, k);
    status_word(
        p,
        Pos2::new(status.left(), top),
        m.status.word(),
        m.status.tone(pal),
        k,
    );
    let mono_font = FontId::monospace(MONO_FONT * k);
    let detail = m.status.detail();
    let shown = elide(p, &detail, &mono_font, status.width());
    mono(p, Pos2::new(status.left(), bottom), &shown, pal.faint, k);
    let full = match &m.status {
        RowStatus::Invalid(d) | RowStatus::Incompatible(d) => d.as_str(),
        _ => detail.as_str(),
    };
    if (shown != full) && !row.dragged {
        let at = Rect::from_x_y_ranges(status.x_range(), row.rect.y_range());
        ui.interact(at, ui.id().with("mod_status"), Sense::hover())
            .on_hover_text(full);
    }
}

/// Returns the second line under a mod's name, fitted to `max`: its source,
/// shortened from the front, then its version, shortened when longer than
/// half the line.
pub(super) fn source_line(p: &egui::Painter, row: &ModRow, font: &FontId, max: f32) -> String {
    let Some(v) = &row.version else {
        return elide_start(p, &row.source, font, max);
    };
    let version = format!(" · {}", elide(p, &version_label(v), font, max * 0.5));
    let source = elide_start(
        p,
        &row.source,
        font,
        max - text_width(p, &version, font, 0.0),
    );
    source + &version
}

/// `0.1.1` becomes `v0.1.1`; a version that already starts with `v` is kept.
pub(super) fn version_label(v: &str) -> String {
    if v.starts_with(['v', 'V']) {
        v.to_string()
    } else {
        format!("v{v}")
    }
}
