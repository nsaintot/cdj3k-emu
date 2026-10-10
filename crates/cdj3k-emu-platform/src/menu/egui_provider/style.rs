//! The strip's metrics and palette, and the egui style that carries them.

use egui::{Color32, CornerRadius, Stroke, Ui, Vec2};

// ── Metrics, from the design canvas ───────────────────────────────────────────

/// Height of the strip. The window's aspect lock adds this to the chassis, so
/// changing it here moves the whole layout with it.
pub const BAR_HEIGHT: f32 = 44.0;

pub(super) const BAR_PAD_X: f32 = 16.0;
pub(super) const PILL_H: f32 = 26.0;
pub(super) const ICON_BTN: Vec2 = Vec2::new(30.0, 26.0);
pub(super) const ICON_GAP: f32 = 2.0;
/// The rule and its air, which travel with the restart button.
pub(super) const RESTART_RULE_W: f32 = 9.0;
/// Air kept between the last pill and the first shortcut.
pub(super) const SHORTCUT_GUTTER: f32 = 12.0;
/// A caption button, Windows' own size.
pub(super) const CAPTION_BTN: Vec2 = Vec2::new(46.0, 32.0);
/// Space around the caption buttons: before them, and at the window's edge.
pub(super) const CAPTION_PAD_LEFT: f32 = 12.0;
pub(super) const CAPTION_PAD_RIGHT: f32 = 6.0;
pub(super) const CAPTION_W: f32 = CAPTION_PAD_LEFT + 3.0 * CAPTION_BTN.x + CAPTION_PAD_RIGHT;
/// The strip width below which the wordmark is dropped. It is the width that
/// the wordmark, both pills and every shortcut need together.
pub(super) const WORDMARK_MIN_STRIP: f32 = 800.0;

/// Width of a notice's hover card.
pub(super) const CARD_W: f32 = 280.0;

/// Width a panel starts at. It grows to fit its widest row, up to [`PANEL_MAX_W`].
pub(super) const PANEL_MIN_W: f32 = 224.0;
/// Beyond this width a device name is truncated, and the panel does not grow.
pub(super) const PANEL_MAX_W: f32 = 420.0;
pub(super) const PANEL_PAD: f32 = 4.0;

pub(super) const ROW_H: f32 = 30.0;
pub(super) const ROW_PAD_X: f32 = 10.0;
/// The checkmark column, which holds its width whether a row is ticked or not.
const GUTTER_W: f32 = 12.0;
const GUTTER_GAP: f32 = 8.0;
/// Where a row's label starts, ticked or not, submenu or not.
pub(super) const TEXT_LEFT: f32 = ROW_PAD_X + GUTTER_W + GUTTER_GAP;

pub(super) const FONT_ROW: f32 = 11.5;
pub(super) const FONT_TRAIL: f32 = 10.5;
pub(super) const FONT_SECTION: f32 = 9.0;
pub(super) const FONT_PILL: f32 = 10.0;

/// Letter spacing on the strip's tracked capitals, as a fraction of the size.
pub(super) const TRACK_WORDMARK: f32 = 0.18;
pub(super) const TRACK_SECTION: f32 = 0.14;

// ── Palette ───────────────────────────────────────────────────────────────────

// The chrome's own palette, independent of the deck theme.
pub(super) const BAR_BG: Color32 = Color32::from_rgb(0x23, 0x23, 0x20);
const PANEL_BG: Color32 = Color32::from_rgb(0x23, 0x23, 0x20);
pub(super) const PANEL_BORDER: Color32 = Color32::from_rgb(0x34, 0x33, 0x2E);
pub(super) const ROW_HOVER: Color32 = Color32::from_rgb(0x2B, 0x2B, 0x27);
/// The strip sits on the same ground its rows do, so a row's hover plate is
/// too faint out here; this is the panel border's value, used as a fill.
pub(super) const STRIP_HOVER: Color32 = Color32::from_rgb(0x34, 0x33, 0x2E);
pub(super) const TEXT: Color32 = Color32::from_rgb(0xF0, 0xEE, 0xE8);
pub(super) const TEXT_MUTED: Color32 = Color32::from_rgb(0xA5, 0xA1, 0x99);
pub(super) const TEXT_DETAIL: Color32 = Color32::from_rgb(0x8E, 0x8A, 0x81);
pub(super) const TEXT_DIM: Color32 = Color32::from_rgb(0x7C, 0x78, 0x6F);
pub(super) const TEXT_DISABLED: Color32 = Color32::from_rgb(0x6A, 0x67, 0x5F);
pub(super) const PILL_BORDER: Color32 = Color32::from_rgb(0x4A, 0x48, 0x42);
pub(super) const ICON_RULE: Color32 = Color32::from_rgb(0x34, 0x33, 0x2E);
pub(super) const ACCENT: Color32 = Color32::from_rgb(0x38, 0x84, 0xFF);
pub(super) const OK_GREEN: Color32 = Color32::from_rgb(0x5F, 0xBE, 0x8A);
pub(super) const WARN_AMBER: Color32 = Color32::from_rgb(0xD9, 0xA2, 0x3C);
/// Windows' close-button red.
pub(super) const CLOSE_RED: Color32 = Color32::from_rgb(0xC4, 0x2B, 0x1C);

// ── egui style ────────────────────────────────────────────────────────────────

/// The strip's buttons: flat until hovered, with the design's inner padding.
///
/// This is set on the style instead of on each button, because a `Button`
/// given an explicit fill uses it in every state and never lights up.
pub(super) fn style_strip(ui: &mut Ui) {
    let s = ui.style_mut();
    s.spacing.item_spacing.x = 8.0;
    s.spacing.button_padding = Vec2::new(ROW_PAD_X, 0.0);
    s.visuals.button_frame = true;
    // `noninteractive` is in the list because that is what egui draws a
    // disabled widget from: without it the dimmed Restart button keeps the
    // default border and reads as the only outlined thing in the strip.
    for v in [
        &mut s.visuals.widgets.noninteractive,
        &mut s.visuals.widgets.inactive,
        &mut s.visuals.widgets.hovered,
        &mut s.visuals.widgets.active,
        &mut s.visuals.widgets.open,
    ] {
        v.corner_radius = CornerRadius::from(3.0);
        v.bg_stroke = Stroke::NONE;
        v.expansion = 0.0;
    }
    s.visuals.widgets.noninteractive.weak_bg_fill = Color32::TRANSPARENT;
    s.visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    s.visuals.widgets.hovered.weak_bg_fill = STRIP_HOVER;
    s.visuals.widgets.active.weak_bg_fill = STRIP_HOVER;
    s.visuals.widgets.open.weak_bg_fill = STRIP_HOVER;
}

/// Dress egui's own menu frame as the design's panel: one hairline border on
/// a flat ground, no shadow.
///
/// egui builds the menu frame from the context's style, not the opener's, so
/// this is set on the context. The setup window overrides the same fields on its own Ui
/// tree, so its popups keep their palette.
pub(super) fn style_menus(ctx: &egui::Context) {
    ctx.all_styles_mut(|s| {
        s.visuals.window_fill = PANEL_BG;
        s.visuals.window_stroke = Stroke::new(1.0_f32, PANEL_BORDER);
        s.visuals.menu_corner_radius = CornerRadius::from(3.0);
        s.visuals.popup_shadow = egui::epaint::Shadow::NONE;
        s.spacing.menu_margin = egui::Margin::from(PANEL_PAD);
        s.spacing.menu_spacing = 6.0;
    });
}

/// Make `Ui::menu_button` draw like one of our rows: same height, same left
/// edge, same hover plate.
pub(super) fn style_submenu(ui: &mut Ui, row_w: f32) {
    let s = ui.style_mut();
    s.spacing.button_padding = Vec2::new(ROW_PAD_X, 0.0);
    s.spacing.interact_size = Vec2::new(row_w, ROW_H);
    s.visuals.button_frame = true;
    for v in [
        &mut s.visuals.widgets.inactive,
        &mut s.visuals.widgets.hovered,
        &mut s.visuals.widgets.active,
        &mut s.visuals.widgets.open,
    ] {
        v.corner_radius = CornerRadius::from(2.0);
        v.bg_stroke = Stroke::NONE;
        v.fg_stroke = Stroke::new(1.0_f32, TEXT);
        v.expansion = 0.0;
    }
    s.visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    s.visuals.widgets.hovered.weak_bg_fill = ROW_HOVER;
    s.visuals.widgets.active.weak_bg_fill = ROW_HOVER;
    s.visuals.widgets.open.weak_bg_fill = ROW_HOVER;
}
