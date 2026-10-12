//! The list screen: the toolbar, the table of mods or the empty view, the
//! doc links and the footer.

use super::*;

// ── Toolbar ───────────────────────────────────────────────────────────────────
const TOOLBAR_H: f32 = 32.0;
const TOOLBAR_GAP: f32 = 10.0;
const TOOL_GAP: f32 = 8.0;
const HEAD_TRACK: f32 = 1.4;
const NOTE_DOT: f32 = 7.0;
const NOTE_DOT_GAP: f32 = 8.0;

// ── Empty state ───────────────────────────────────────────────────────────────
const ZONE_BOTTOM: f32 = 20.0;
const ZONE_LINKS_GAP: f32 = 14.0;
const ZONE_GLYPH: f32 = 28.0;
const ZONE_TITLE_FONT: f32 = 15.0;
const ZONE_SUB_FONT: f32 = 12.5;
const LINK_GAP: f32 = 24.0;

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
        tone: SubTone::Plain,
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
    // The note ends before the two buttons on the right.
    let buttons_w = tool_width(p, "Add mod…", true, k)
        + TOOL_GAP * k
        + tool_width(p, "Add from URL…", false, k);
    toolbar_note(ui, page, tb, tb.right() - buttons_w - TOOL_GAP * k, pal, k);

    let mut action = None;
    let cy = tb.center().y;
    let add = tool_button(
        ui,
        tb.right(),
        cy,
        TOOLBAR_H * k,
        "Add mod…",
        true,
        false,
        k,
        pal,
    );
    if add.clicked() {
        action = Some(ModsAction::Add);
    }
    let x = add.rect.left() - TOOL_GAP * k;
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

/// Draws the note at the toolbar's left end, up to `note_right`: the result
/// of the last add, the reason the mods do not run, or the number of mods and
/// how many of them are enabled.
fn toolbar_note(ui: &egui::Ui, page: &ModsPage, tb: Rect, note_right: f32, pal: &Palette, k: f32) {
    let p = ui.painter();
    let cy = tb.center().y;
    let incompatible = page
        .rows
        .iter()
        .filter(|r| matches!(r.status, RowStatus::Incompatible(_)))
        .count();
    // Draws `text` on one line and returns its rect and whether it was
    // shortened.
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
            // A rule separates this button from Back and Close, because it
            // acts on the deck and they act on the window.
            if let Some((label, pending, asked)) = run {
                crate::app::picker::footer_rule(ui, pal, k);
                let size = [
                    button_text_width(ui, label, k) + COPY_PAD * k,
                    theme::BUTTON_SIZE[1] * k,
                ];
                // The button is filled when the list has changed since the deck
                // booted.
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
    let y = drop_message(p, zone, pal, k);
    let action = empty_buttons(ui, zone, y, note, pal, k);
    draw_links(ui, zone.left(), space, k, pal).or(action)
}

/// Draws the cube, "Drop a mod here" and the accepted formats, centred in
/// `zone` above the buttons. Returns the y where the buttons start.
fn drop_message(p: &egui::Painter, zone: Rect, pal: &Palette, k: f32) -> f32 {
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
    y + sub_h + 18.0 * k
}

/// Draws the two add buttons, centred in `zone` from `y`, and the result of
/// the last add under them.
fn empty_buttons(
    ui: &egui::Ui,
    zone: Rect,
    y: f32,
    note: Option<&(String, bool)>,
    pal: &Palette,
    k: f32,
) -> Option<ModsAction> {
    let p = ui.painter();
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
        p.text(
            Pos2::new(zone.center().x, y + h + 14.0 * k),
            Align2::CENTER_TOP,
            elide(p, text, &font, zone.width() - 2.0 * (theme::GUTTER * k)),
            font,
            if *error { pal.danger } else { pal.muted },
        );
    }
    action
}

/// The y where the content above the links ends.
fn links_top(space: Rect, k: f32) -> f32 {
    space.bottom() - (ZONE_BOTTOM + LINKS_H + ZONE_LINKS_GAP) * k
}

/// Draws the links to the documentation and the example mod at the foot of
/// `space`.
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
