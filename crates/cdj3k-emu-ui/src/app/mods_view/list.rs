//! The list: the toolbar, the table of mods, and the empty view.

use super::*;
use crate::app::table::{from_right, Column, Line, Row, Table, Width};

pub(super) fn draw_list(ui: &mut egui::Ui, page: &mut ModsPage) -> Option<ModsAction> {
    let k = k_of(ui.max_rect());
    let pal = theme::palette(ui.ctx());
    let empty = page.rows.is_empty();
    let head = Header {
        slot: Some(page.slot),
        model: page.model,
        release: page.release.as_deref(),
        state: None,
        title: "Mods",
        sub: if empty { EMPTY_SUB } else { LIST_SUB },
        switchable: false,
        warn: false,
    };
    let body = draw_header(ui, &head).body;
    let (space, bar) = footer_split(body, k);
    let mut action = if empty {
        draw_empty(ui, page.note.as_ref(), space, k, pal)
    } else {
        let tb = Rect::from_min_size(
            Pos2::new(space.left() + theme::GUTTER * k, space.top()),
            Vec2::new(space.width() - 2.0 * theme::GUTTER * k, TOOLBAR_H * k),
        );
        let a = draw_toolbar(ui, page, tb, k, pal);
        let table = Rect::from_min_max(
            Pos2::new(tb.left(), tb.bottom() + TOOLBAR_GAP * k),
            Pos2::new(tb.right(), links_top(space, k)),
        );
        let a = draw_table(ui, page, table, k, pal).or(a);
        draw_links(ui, table.left(), space, k, pal).or(a)
    };
    action = draw_list_footer(ui, page, bar, k, pal).or(action);

    let dropped: Vec<PathBuf> = ui.ctx().input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_path_buf())
            .filter(|p| !p.as_os_str().is_empty())
            .collect()
    });
    if !dropped.is_empty() {
        action = Some(ModsAction::Dropped(dropped));
    }

    // While a dialog is up, it receives every click.
    if let Some(name) = page.remove.clone() {
        action = draw_remove(ui, page.slot, &name, k, pal);
    } else if let Some((name, old, new)) = page.replace.clone() {
        action = draw_replace(ui, &name, &old, &new, k, pal);
    } else if page.url.is_some() {
        action = draw_url(ui, page, k, pal);
    }
    action
}

pub(super) fn draw_toolbar(
    ui: &mut egui::Ui,
    page: &ModsPage,
    tb: Rect,
    k: f32,
    pal: &Palette,
) -> Option<ModsAction> {
    let p = ui.painter();
    let cy = tb.center().y;
    let incompatible = page
        .rows
        .iter()
        .filter(|r| matches!(r.status, RowStatus::Incompatible(_)))
        .count();
    // The note ends before the two buttons on the right.
    let buttons_w = tool_width(p, "Add mod…", true, k)
        + TOOL_GAP * k
        + tool_width(p, "Add from URL…", false, k);
    let note_right = tb.right() - buttons_w - TOOL_GAP * k;
    // Draws `text` on one line; returns where, and whether it was shortened.
    let warn = |text: &str, col: Color32| {
        let r = NOTE_DOT * k * 0.5;
        let x = tb.left() + 2.0 * r + NOTE_DOT_GAP * k;
        let font = FontId::proportional(NOTE_FONT * k);
        p.circle_filled(Pos2::new(tb.left() + r, cy), r, col);
        let line = one_line(text);
        let shown = elide(p, &line, &font, note_right - x);
        let cut = shown != text;
        let at = p.text(Pos2::new(x, cy), Align2::LEFT_CENTER, shown, font, col);
        (at, cut)
    };
    if let Some((note, error)) = &page.note {
        let (at, cut) = warn(note, if *error { pal.danger } else { pal.muted });
        if cut {
            ui.interact(at, ui.id().with("mods_note"), Sense::hover())
                .on_hover_text(note.as_str());
        }
    } else if !page.gate {
        warn(
            "Mods are disabled: toggle Emulation › Enable Mods to enable them.",
            pal.warn,
        );
    } else if incompatible > 0 {
        let text = if incompatible == 1 {
            "1 mod is incompatible".to_string()
        } else {
            format!("{incompatible} mods are incompatible")
        };
        warn(&text, pal.warn);
    } else {
        let on = page.rows.iter().filter(|r| r.enabled).count();
        let label = format!(
            "{} {} · {on} ON",
            page.rows.len(),
            if page.rows.len() == 1 { "MOD" } else { "MODS" }
        );
        tracked(
            p,
            tb.left_center(),
            &label,
            &FontId::proportional(theme::MICRO_FONT * k),
            pal.dim,
            HEAD_TRACK * k,
        );
    }

    let mut action = None;
    let mut x = tb.right();
    let add = tool_button(ui, x, cy, TOOLBAR_H * k, "Add mod…", true, false, k, pal);
    if add.clicked() {
        action = Some(ModsAction::Add);
    }
    x = add.rect.left() - TOOL_GAP * k;
    if tool_button(
        ui,
        x,
        cy,
        TOOLBAR_H * k,
        "Add from URL…",
        false,
        false,
        k,
        pal,
    )
    .clicked()
    {
        action = Some(ModsAction::AskUrl);
    }
    action
}

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
    let (on, name, status) = (row.cells[ON], row.cells[MOD], row.cells[STATUS]);
    let handle = Rect::from_center_size(
        Pos2::new(on.left() + HANDLE_W * k * 0.5, on.center().y),
        Vec2::new(HANDLE_W * k, HANDLE_H * k),
    );
    row.handle(handle, |p, r, hot| {
        let col = if hot { pal.ink } else { pal.faint };
        glyph(p, Glyph::Handle, r.shrink(2.0 * k), col, k);
    });

    let ui: &egui::Ui = row.ui;
    let p = ui.painter();
    let mut action = None;
    let check = check_rect(on, k);
    if let RowStatus::Incompatible(_) = m.status {
        // The mod keeps its on/off setting; the slot's firmware stops it
        // from running.
        glyph(p, Glyph::Warn, check, pal.danger, k);
        ui.interact(check, ui.id().with("mod_warn"), Sense::hover())
            .on_hover_text(format!(
                "{} is incompatible with this slot's firmware",
                m.name
            ));
    } else {
        let resp = theme::pointer(ui.interact(
            check.expand(4.0 * k),
            ui.id().with("mod_on"),
            Sense::click(),
        ));
        checkbox(p, check, m.enabled, resp.hovered(), k, pal);
        if resp.clicked() {
            action = Some(ModsAction::Toggle(m.name.clone()));
        }
    }

    let size = Vec2::new(ACTION[0] * k, ACTION[1] * k);
    let [remove, log] = from_right(row.cells[ACTIONS], size, ACTION_GAP * k, 2)[..] else {
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
    // The name gets at least half of the width left after the DEV chip; the
    // author gets the rest.
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
    status_word(
        p,
        Pos2::new(status.left(), top),
        m.status.word(),
        m.status.tone(pal),
        k,
    );
    let detail = m.status.detail();
    let shown = elide(p, &detail, &mono_font, status.width());
    mono(p, Pos2::new(status.left(), bottom), &shown, pal.faint, k);
    // Hovering shows the full reason when the row shows only part of it.
    let full = match &m.status {
        RowStatus::Invalid(d) | RowStatus::Incompatible(d) => d.as_str(),
        _ => detail.as_str(),
    };
    if (shown != full) && !row.dragged {
        let at = Rect::from_x_y_ranges(status.x_range(), row.rect.y_range());
        ui.interact(at, ui.id().with("mod_status"), Sense::hover())
            .on_hover_text(full);
    }
    action
}

pub(super) fn draw_list_footer(
    ui: &mut egui::Ui,
    page: &ModsPage,
    bar: Rect,
    k: f32,
    pal: &Palette,
) -> Option<ModsAction> {
    let inner = plain_footer(ui, bar, k, pal);
    let mut action = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
        theme::apply_setup_style(ui, pal);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::secondary(ui, crate::app::picker::button_size(k), "Close", pal).clicked() {
                action = Some(ModsAction::Close);
            }
            if theme::secondary(ui, [BACK_W * k, theme::BUTTON_SIZE[1] * k], "Back", pal).clicked()
            {
                action = Some(ModsAction::Back);
            }
            let run = match page.run {
                Run::Restart { pending } => {
                    Some(("Restart Emulation", pending, ModsAction::Restart))
                }
                Run::Start => Some(("Start Emulation", false, ModsAction::Start)),
                Run::None => None,
            };
            // Set apart from Back and Close by a rule, because it acts on the
            // deck, not on the window.
            if let Some((label, pending, asked)) = run {
                crate::app::picker::footer_rule(ui, pal, k);
                let size = [
                    button_text_width(ui, label, k) + COPY_PAD * k,
                    theme::BUTTON_SIZE[1] * k,
                ];
                // Filled when the list has changed since the deck booted.
                let resp = if pending {
                    theme::primary(ui, size, label, pal)
                } else {
                    theme::secondary(ui, size, label, pal)
                };
                if resp.clicked() {
                    action = Some(asked);
                }
            }
        });
    });
    action
}

/// The view with no mods: a drop zone, the two add buttons, and the doc links.
/// `note` is the result of the last add, shown under the buttons.
pub(super) fn draw_empty(
    ui: &mut egui::Ui,
    note: Option<&(String, bool)>,
    space: Rect,
    k: f32,
    pal: &Palette,
) -> Option<ModsAction> {
    let g = theme::GUTTER * k;
    let zone = Rect::from_min_max(
        Pos2::new(space.left() + g, space.top()),
        Pos2::new(space.right() - g, links_top(space, k)),
    );
    let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
    let p = ui.painter();
    p.rect_filled(zone, theme::ROUND * k, pal.plate);
    dashed_rect(p, zone, if hovering { pal.ink } else { pal.line_strong }, k);

    let title_h = ZONE_TITLE_FONT * k * 1.25;
    let sub_h = ZONE_SUB_FONT * k * 1.3;
    let block = ZONE_GLYPH * k + 14.0 * k + title_h + 4.0 * k + sub_h + 18.0 * k + TOOLBAR_H * k;
    let mut y = zone.center().y - block * 0.5;
    glyph(
        p,
        Glyph::Cube,
        Rect::from_min_size(
            Pos2::new(zone.center().x - ZONE_GLYPH * k * 0.5, y),
            Vec2::splat(ZONE_GLYPH * k),
        ),
        pal.dim,
        k,
    );
    y += ZONE_GLYPH * k + 14.0 * k;
    p.text(
        Pos2::new(zone.center().x, y),
        Align2::CENTER_TOP,
        "Drop a mod here",
        FontId::proportional(ZONE_TITLE_FONT * k),
        pal.ink,
    );
    y += title_h + 4.0 * k;
    p.text(
        Pos2::new(zone.center().x, y),
        Align2::CENTER_TOP,
        "A .tgz archive, or a mod folder",
        FontId::proportional(ZONE_SUB_FONT * k),
        pal.muted,
    );
    y += sub_h + 18.0 * k;

    let mut action = None;
    let h = TOOLBAR_H * k;
    let cy = y + h * 0.5;
    let add_w = tool_width(p, "Add mod…", true, k);
    let url_w = tool_width(p, "Add from URL…", false, k);
    let gap = 10.0 * k;
    let right = zone.center().x + (add_w + gap + url_w) * 0.5;
    if tool_button(ui, right, cy, h, "Add mod…", true, true, k, pal).clicked() {
        action = Some(ModsAction::Add);
    }
    if tool_button(
        ui,
        right - add_w - gap,
        cy,
        h,
        "Add from URL…",
        false,
        false,
        k,
        pal,
    )
    .clicked()
    {
        action = Some(ModsAction::AskUrl);
    }
    if let Some((text, error)) = note {
        let font = FontId::proportional(NOTE_FONT * k);
        let p = ui.painter();
        p.text(
            Pos2::new(zone.center().x, y + h + 14.0 * k),
            Align2::CENTER_TOP,
            elide(p, text, &font, zone.width() - 2.0 * g),
            font,
            if *error { pal.danger } else { pal.muted },
        );
    }

    draw_links(ui, zone.left(), space, k, pal).or(action)
}

/// The y where the content above the links ends.
fn links_top(space: Rect, k: f32) -> f32 {
    space.bottom() - (ZONE_BOTTOM + LINKS_H + ZONE_LINKS_GAP) * k
}

/// The documentation and the example mod, at the foot of `space`.
fn draw_links(ui: &egui::Ui, left: f32, space: Rect, k: f32, pal: &Palette) -> Option<ModsAction> {
    let mut action = None;
    let ly = links_top(space, k) + ZONE_LINKS_GAP * k + LINKS_H * k * 0.5;
    let mut x = left + 4.0 * k;
    for (label, g, url) in [
        ("Mods documentation", Glyph::Doc, DOCS_URL),
        ("Example mod", Glyph::Folder, TEMPLATE_URL),
    ] {
        let r = link(ui, Pos2::new(x, ly), label, g, k, pal);
        if r.clicked() {
            action = Some(ModsAction::Open(url.into()));
        }
        x = r.rect.right() + LINK_GAP * k;
    }
    action
}

/// The second line under a mod's name, fitted to `max`: its source, shortened
/// from the front, then its version, shortened when longer than half the line.
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
