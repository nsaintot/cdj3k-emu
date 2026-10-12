//! The frame every setup step shares: the identity strip and title block at
//! the top, the action bar at the foot, and the size factor `k` that scales
//! them both.

use cdj3k_emu_panel::Model;
use cdj3k_emu_platform::app_meta::{APP_DISPLAY_NAME, VERSION};
use egui::{Align2, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use super::paint::{hairline, text_width, tracked};
use super::slot_menu::draw_slot_switch;
use crate::app::theme::{self, Palette};

/// The window width at which `k` is 1.0 and the pixel sizes apply as they
/// are written.
const REF_WIDTH: f32 = 760.0;
const K_MIN: f32 = 0.70;
const K_MAX: f32 = 1.40;

// ── Identity strip ────────────────────────────────────────────────────────────
/// The gap between the wordmark, the slot and the deck.
pub(super) const STRIP_GAP: f32 = 12.0;
/// The short upright rule that separates them.
const STRIP_RULE_H: f32 = 12.0;
/// The square of deck colour before a model's name.
pub(super) const STRIP_SWATCH: f32 = 6.0;
/// The gap between that square and the name, and between the state dot and
/// its word.
pub(super) const STRIP_MARK_GAP: f32 = 6.0;
const STRIP_STATE_DOT: f32 = 7.0;

// ── Title block ───────────────────────────────────────────────────────────────
const TITLE_TOP_GAP: f32 = 20.0;
const TITLE_SUB_GAP: f32 = 8.0;
/// The gap below the subtitle, where a step's own content starts.
const HEADER_BODY_GAP: f32 = 16.0;

/// The upright rule that sets the slot's own action apart from the window's
/// in the action bar.
const FOOTER_RULE_H: f32 = 20.0;

/// What the identity strip and the title block say for a step.
pub(in crate::app) struct Header<'a> {
    /// The slot the step is about; `None` for a window about the whole app.
    pub slot: Option<u32>,
    /// The deck this step is about, named in the strip beside its colour.
    pub model: Option<Model>,
    /// A word at the strip's right end while something is under way.
    pub state: Option<&'a str>,
    /// The firmware release on that deck, beside its name.
    pub release: Option<&'a str>,
    pub title: &'a str,
    pub sub: &'a str,
    /// Whether the slot chip opens its menu. A step in the middle of a job
    /// turns it off so that the window stays on its slot.
    pub switchable: bool,
    /// How `sub` reads: plain, a warning or an error.
    pub tone: SubTone,
}

/// The colour of a header's subtitle.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum SubTone {
    Plain,
    /// Something to fix that does not stop the slot, such as an outdated
    /// install.
    Warn,
    /// A problem that keeps the slot from booting, such as unreadable settings.
    Error,
}

/// What a header drew, and what the user asked of it.
pub(in crate::app) struct HeaderOut {
    /// The area the step's own content gets, the action bar included.
    pub body: Rect,
    /// A slot picked from the chip's menu.
    pub switch_to: Option<u32>,
}

/// Paint the window's ground, its identity strip and its title block, and
/// return the area for the step's own content, including the action bar.
/// Every step draws this, so the window looks the same from choosing a deck
/// to installing its firmware.
pub(in crate::app) fn draw_header(ui: &egui::Ui, head: &Header<'_>) -> HeaderOut {
    let area = ui.max_rect();
    let k = k_of(area);
    let pal = theme::palette(ui.ctx());
    ui.painter().rect_filled(area, 0.0, pal.paper);

    let strip = Rect::from_min_max(
        area.left_top(),
        Pos2::new(area.right(), area.top() + theme::TOPBAR_H * k),
    );
    let switch_to = draw_strip(ui, strip, head, pal, k);
    let body_top = draw_title(ui.painter(), area, strip.bottom(), head, pal, k);
    HeaderOut {
        body: Rect::from_min_max(Pos2::new(area.left(), body_top), area.max),
        switch_to,
    }
}

/// Draws the identity strip, left to right: the wordmark, the slot chip, the
/// deck and its release, and at the right end the state word. Returns a slot
/// picked from the chip's menu.
fn draw_strip(ui: &egui::Ui, strip: Rect, head: &Header<'_>, pal: &Palette, k: f32) -> Option<u32> {
    let p = ui.painter();
    p.rect_filled(strip, 0.0, pal.plate);
    hairline(p, strip.left_bottom(), strip.right_bottom(), pal.line, k);

    let micro = FontId::proportional(theme::MICRO_FONT * k);
    let cy = strip.center().y;
    let mut x = strip.left() + theme::GUTTER * k;
    x += tracked(
        p,
        Pos2::new(x, cy),
        APP_DISPLAY_NAME,
        &micro,
        pal.ink,
        theme::WORDMARK_TRACK * k,
    ) + STRIP_GAP * k;
    let mut switch_to = None;
    if let Some(slot) = head.slot {
        x += strip_rule(p, x, cy, pal.line, k) + STRIP_GAP * k;
        let (chip_w, picked) =
            draw_slot_switch(ui, Pos2::new(x, cy), slot, head.switchable, &micro, pal, k);
        switch_to = picked;
        x += chip_w + STRIP_GAP * k;
    }
    if let Some(model) = head.model {
        x += strip_rule(p, x, cy, pal.line, k) + STRIP_GAP * k;
        strip_deck(p, Pos2::new(x, cy), model, head.release, &micro, pal, k);
    }
    if let Some(state) = head.state {
        strip_state(p, strip.right(), cy, state, &micro, pal, k);
    }
    switch_to
}

/// Draws the deck's colour, its name, and the firmware release it was
/// installed from, in a fainter colour than the name.
fn strip_deck(
    p: &egui::Painter,
    at: Pos2,
    model: Model,
    release: Option<&str>,
    micro: &FontId,
    pal: &Palette,
    k: f32,
) {
    let (mut x, cy) = (at.x, at.y);
    let s = STRIP_SWATCH * k;
    p.rect_filled(
        Rect::from_center_size(Pos2::new(x + s * 0.5, cy), Vec2::splat(s)),
        theme::HAIRLINE * k,
        crate::app::ui::slate::for_model(model).accent,
    );
    x += s + STRIP_MARK_GAP * k;
    x += tracked(
        p,
        Pos2::new(x, cy),
        model.title(),
        micro,
        pal.dim,
        theme::MICRO_TRACK * k,
    );
    if let Some(release) = release {
        tracked(
            p,
            Pos2::new(x + STRIP_MARK_GAP * k, cy),
            release,
            micro,
            pal.faint,
            theme::MICRO_TRACK * k,
        );
    }
}

/// Draws the state word at the strip's right end, after a dot.
fn strip_state(
    p: &egui::Painter,
    right: f32,
    cy: f32,
    state: &str,
    micro: &FontId,
    pal: &Palette,
    k: f32,
) {
    let w = text_width(p, state, micro, theme::MICRO_TRACK * k);
    let left = right - theme::GUTTER * k - w;
    let r = STRIP_STATE_DOT * k * 0.5;
    p.circle_filled(Pos2::new(left - STRIP_MARK_GAP * k - r, cy), r, pal.ink);
    tracked(
        p,
        Pos2::new(left, cy),
        state,
        micro,
        pal.ink,
        theme::MICRO_TRACK * k,
    );
}

/// Draws the title and the subtitle under the strip, whose foot is at `top`.
/// Returns where a step's own content starts.
fn draw_title(
    p: &egui::Painter,
    area: Rect,
    top: f32,
    head: &Header<'_>,
    pal: &Palette,
    k: f32,
) -> f32 {
    let left = area.left() + theme::GUTTER * k;
    let mut y = top + TITLE_TOP_GAP * k;
    p.text(
        Pos2::new(left, y),
        Align2::LEFT_TOP,
        head.title,
        FontId::proportional(theme::TITLE_FONT * k),
        pal.ink,
    );
    y += theme::TITLE_FONT * k * 1.12 + TITLE_SUB_GAP * k;
    let sub_col = match head.tone {
        SubTone::Plain => pal.muted,
        SubTone::Warn => pal.warn,
        SubTone::Error => pal.danger,
    };
    let galley = p.layout(
        head.sub.to_owned(),
        FontId::proportional(theme::SUB_FONT * k),
        sub_col,
        area.width() - 2.0 * theme::GUTTER * k,
    );
    let sub_h = galley.size().y;
    p.galley(Pos2::new(left, y), galley, sub_col);
    y + sub_h + HEADER_BODY_GAP * k
}

/// Split a step's area into its own space and the action bar at its foot.
pub(in crate::app) fn footer_split(area: Rect, k: f32) -> (Rect, Rect) {
    let y = area.bottom() - theme::FOOTER_H * k;
    (
        Rect::from_min_max(area.min, Pos2::new(area.right(), y)),
        Rect::from_min_max(Pos2::new(area.left(), y), area.max),
    )
}

/// Paint the action bar, a plate under a hairline with the app version at its
/// left end, and return the area for the step's buttons.
pub(in crate::app) fn draw_footer(ui: &egui::Ui, bar: Rect, pal: &Palette, k: f32) -> Rect {
    let p = ui.painter();
    p.rect_filled(bar, 0.0, pal.plate);
    hairline(p, bar.left_top(), bar.right_top(), pal.line, k);
    p.text(
        Pos2::new(bar.left() + theme::GUTTER * k, bar.center().y),
        Align2::LEFT_CENTER,
        format!("v{VERSION}"),
        FontId::monospace(theme::MICRO_FONT * k),
        pal.faint,
    );
    Rect::from_center_size(
        bar.center(),
        Vec2::new(
            bar.width() - 2.0 * theme::GUTTER * k,
            theme::BUTTON_SIZE[1] * k,
        ),
    )
}

/// The size of every action button in the bar, so a row of them lines up
/// whatever the labels are.
pub(in crate::app) fn button_size(k: f32) -> [f32; 2] {
    [theme::BUTTON_SIZE[0] * k, theme::BUTTON_SIZE[1] * k]
}

/// The window's size factor. Every pixel size is written for a window
/// [`REF_WIDTH`] wide.
pub(in crate::app) fn k_of(area: Rect) -> f32 {
    (area.width() / REF_WIDTH).clamp(K_MIN, K_MAX)
}

/// Draws a waiting step, shown while the emulation stops before its
/// installation can be changed.
pub(in crate::app) fn draw_busy(
    ui: &mut egui::Ui,
    head: &Header<'_>,
    headline: &str,
    detail: &str,
) {
    let body = draw_header(ui, head).body;
    let k = k_of(ui.max_rect());
    let pal = theme::palette(ui.ctx());
    ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
        theme::apply_setup_style(ui, pal);
        ui.vertical_centered(|ui| {
            ui.add_space(body.height() * 0.28);
            ui.spinner();
            ui.add_space(14.0 * k);
            ui.label(egui::RichText::new(headline).size(16.0 * k).color(pal.ink));
            ui.add_space(6.0 * k);
            ui.label(
                egui::RichText::new(detail)
                    .size(theme::BODY_FONT * k)
                    .color(pal.muted),
            );
        });
    });
}

/// The upright rule in the action bar, between the window's own action and
/// the slot's. It takes its own place in the row, so the bar's item spacing
/// applies on both sides.
pub(in crate::app) fn footer_rule(ui: &mut egui::Ui, pal: &Palette, k: f32) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(theme::HAIRLINE * k, FOOTER_RULE_H * k),
        Sense::hover(),
    );
    let x = rect.center().x;
    hairline(
        ui.painter(),
        Pos2::new(x, rect.top()),
        Pos2::new(x, rect.bottom()),
        pal.line,
        k,
    );
}

/// The short upright rule between the identity strip's parts. Returns its
/// width, so the caller can go on laying the strip out left to right.
fn strip_rule(p: &egui::Painter, x: f32, cy: f32, col: egui::Color32, k: f32) -> f32 {
    let h = STRIP_RULE_H * k * 0.5;
    p.line_segment(
        [Pos2::new(x, cy - h), Pos2::new(x, cy + h)],
        Stroke::new(theme::HAIRLINE * k, col),
    );
    theme::HAIRLINE * k
}
