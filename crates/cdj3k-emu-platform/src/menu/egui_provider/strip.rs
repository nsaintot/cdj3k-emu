//! The strip itself: the sandwich, the pills, the notice badge, the
//! shortcuts and, where the strip is the title bar, the caption buttons.

use std::collections::BTreeSet;

use egui::{Align, Color32, CursorIcon, FontId, Layout, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use super::icon::{paint_gauge, paint_icon, Pen};
use super::paint::{chevron, mono_fmt, tracked_fmt, vertical_rule};
use super::style::{
    style_strip, ACCENT, CAPTION_BTN, CAPTION_PAD_LEFT, CAPTION_PAD_RIGHT, CAPTION_W, CARD_W,
    CLOSE_RED, FONT_PILL, FONT_ROW, FONT_SECTION, ICON_BTN, ICON_GAP, ICON_RULE, PANEL_PAD,
    PILL_BORDER, PILL_H, RESTART_RULE_W, ROW_PAD_X, SHORTCUT_GUTTER, STRIP_HOVER, TEXT,
    TEXT_DETAIL, TEXT_DIM, TEXT_MUTED, TRACK_SECTION, TRACK_WORDMARK, WARN_AMBER,
    WORDMARK_MIN_STRIP,
};
use super::EguiProvider;
use crate::desktop::CAPTION_IN_STRIP;
use crate::menu::id::MenuId;
use crate::menu::model::{MenuIcon, MenuNode, Notice, Predefined};

/// The id under which the sandwich records itself as open. It starts with a
/// control character, so it cannot collide with a menu label.
const HAMBURGER: &str = "\u{1}menu";
/// The same, for the notice badge's card.
const NOTICE: &str = "\u{1}notice";

#[derive(Clone, Copy)]
enum Caption {
    Minimize,
    Maximize,
    Close,
}

impl EguiProvider {
    pub(super) fn bar(&mut self, ui: &mut Ui) {
        let mut open = BTreeSet::new();
        // Use a custom layout instead of `egui::menu::bar`, which fixes the
        // band to `interact_size.y`, pins it to the top of the strip, and
        // overwrites the button padding that the pills are measured against.
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            style_strip(ui);
            self.hamburger(ui, &mut open);
            if CAPTION_IN_STRIP && ui.max_rect().width() >= WORDMARK_MIN_STRIP {
                wordmark(ui);
            }

            // Emulation and Instances get a pill that reads out their state:
            // which deck this window is, and which slot.
            for label in ["Emulation", "Instances"] {
                if let Some(root) = self.root(label) {
                    self.pill(ui, &root, &mut open);
                }
            }
            if let Some(notice) = self.model.notice.clone() {
                self.notice_badge(ui, &notice, &mut open);
            }

            // What the shortcuts get is whatever the pills left, less a
            // gutter. A narrow window sheds them rather than letting the
            // right-hand group draw over the pills.
            let caption = if CAPTION_IN_STRIP { CAPTION_W } else { 0.0 };
            let mut room = ui.available_width() - SHORTCUT_GUTTER - caption;
            let mut icons = Vec::new();
            for label in ["View", "Audio", "Network", "Storage"] {
                if room < ICON_BTN.x + ICON_GAP {
                    break;
                }
                if let Some(root) = self.root(label) {
                    room -= ICON_BTN.x + ICON_GAP;
                    icons.push(root);
                }
            }
            // Restart carries its own rule, and is the first thing dropped:
            // it is the one shortcut that is also a row in a menu.
            let restart = room >= ICON_BTN.x + RESTART_RULE_W;

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if CAPTION_IN_STRIP {
                    self.caption_buttons(ui);
                }
                ui.spacing_mut().item_spacing.x = ICON_GAP;
                // Right-to-left, so this reads backwards on screen: View is
                // rightmost, Restart leads the group behind its own rule.
                for root in &icons {
                    self.icon_button(ui, root, &mut open);
                }
                if restart {
                    ui.add_space(4.0);
                    vertical_rule(ui, 16.0, ICON_RULE);
                    ui.add_space(4.0);
                    self.restart_button(ui);
                }
            });
        });
        self.open = open;
    }

    /// Every menu, as the design's sandwich: the same tree the pills open.
    fn hamburger(&mut self, ui: &mut Ui, open: &mut BTreeSet<String>) {
        let button = plate(self.open.contains(HAMBURGER));
        // Emulation and Instances have their own pills next to it, so the
        // sandwich holds the update row and the menus the strip cannot reach
        // in one click.
        let mut roots: Vec<MenuNode> = self.update_row().into_iter().collect();
        if !roots.is_empty() {
            roots.push(MenuNode::Separator);
        }
        roots.extend(
            self.model
                .roots
                .iter()
                .filter(|n| !matches!(n, MenuNode::Submenu { label, .. } if label == "Emulation" || label == "Instances"))
                .cloned(),
        );
        roots.push(MenuNode::Separator);
        roots.push(MenuNode::Predefined(Predefined::Quit));
        let (response, inner) =
            egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
                self.panel(ui, &roots);
            });
        if inner.is_some() {
            open.insert(HAMBURGER.into());
        }
        self.widgets.push(response.rect);
        let c = response.rect.center();
        let s = Stroke::new(1.4_f32, TEXT);
        for dy in [-4.0, 0.0, 4.0] {
            ui.painter().hline(c.x - 6.0..=c.x + 6.0, c.y + dy, s);
        }
        response.on_hover_cursor(CursorIcon::PointingHand);
    }

    /// A menu opener that also reads out its state.
    fn pill(&mut self, ui: &mut Ui, root: &MenuNode, open: &mut BTreeSet<String>) {
        let MenuNode::Submenu {
            label,
            status,
            detail,
            icon,
            children,
            ..
        } = root
        else {
            return;
        };
        let was_open = self.open.contains(label);

        let mut job = egui::text::LayoutJob::default();
        // The accent square is painted separately, so the label starts after
        // the space it takes.
        let lead = if matches!(icon, Some(MenuIcon::Emulation)) {
            6.0 + 8.0
        } else {
            0.0
        };
        job.append(
            status.as_deref().unwrap_or(label.as_str()),
            lead,
            tracked_fmt(FONT_PILL, TRACK_WORDMARK, TEXT),
        );
        if let Some(d) = detail {
            job.append(d, 6.0, mono_fmt(FONT_PILL, TEXT_DIM));
        }
        // Trailing blanks reserve the chevron's width; it is painted below
        // because the default font has no such glyph.
        job.append("    ", 0.0, tracked_fmt(FONT_PILL, 0.0, TEXT));

        let mut button = egui::Button::new(job)
            .min_size(Vec2::new(0.0, PILL_H))
            .corner_radius(3.0)
            .stroke(Stroke::new(
                1.0_f32,
                if was_open { TEXT } else { PILL_BORDER },
            ));
        if was_open {
            button = button.fill(STRIP_HOVER);
        }
        let children = Self::menu_rows(children);
        let (response, inner) =
            egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
                self.panel(ui, &children);
            });
        if inner.is_some() {
            open.insert(label.clone());
        }

        let rect = response.rect;
        self.widgets.push(rect);
        if matches!(icon, Some(MenuIcon::Emulation)) {
            ui.painter().rect_filled(
                Rect::from_min_size(
                    Pos2::new(rect.left() + ROW_PAD_X, rect.center().y - 3.0),
                    Vec2::splat(6.0),
                ),
                1.0,
                ACCENT,
            );
        }
        chevron(
            ui,
            Pos2::new(rect.right() - ROW_PAD_X - 4.5, rect.center().y - 1.0),
            if was_open { TEXT } else { TEXT_DETAIL },
        );
        response.on_hover_cursor(CursorIcon::PointingHand);
    }

    /// A glyph button that opens its menu in one click.
    fn icon_button(&mut self, ui: &mut Ui, root: &MenuNode, open: &mut BTreeSet<String>) {
        let MenuNode::Submenu {
            label,
            enabled,
            icon,
            status,
            children,
            ..
        } = root
        else {
            return;
        };
        let button = plate(self.open.contains(label));
        let glyph = *icon;
        let children = Self::menu_rows(children);
        let tip = match status {
            Some(s) => format!("{label} — {s}"),
            None => label.clone(),
        };

        // A disabled menu keeps its place and its glyph, greyed, and opens
        // nothing. The glyph is painted outside the disabled scope, which
        // would fade it.
        let rect = ui
            .add_enabled_ui(*enabled, |ui| {
                let (response, inner) =
                    egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
                        self.panel(ui, &children);
                    });
                if inner.is_some() {
                    open.insert(label.clone());
                }
                let rect = response.rect;
                response
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .on_hover_text(tip)
                    .on_disabled_hover_text(format!(
                        "{label} — available once the deck has booted"
                    ));
                rect
            })
            .inner;
        self.widgets.push(rect);
        if let Some(g) = glyph {
            paint_icon(ui, rect, g, *enabled);
        }
    }

    /// A condition flagged on the strip: the design's gauge glyph, whose card
    /// shows on hover and stays up on a click.
    fn notice_badge(&mut self, ui: &mut Ui, notice: &Notice, open: &mut BTreeSet<String>) {
        let was_open = self.open.contains(NOTICE);
        let (response, inner) = egui::containers::menu::MenuButton::from_button(plate(was_open))
            .ui(ui, |ui| {
                egui::Frame::NONE
                    .inner_margin(egui::Margin::from(ROW_PAD_X - PANEL_PAD))
                    .show(ui, |ui| notice_card(ui, notice));
                self.chrome.push(ui.min_rect().expand(PANEL_PAD + 1.0));
            });
        if inner.is_some() {
            open.insert(NOTICE.into());
        }
        self.widgets.push(response.rect);
        paint_gauge(ui, response.rect);
        let res = response.on_hover_cursor(CursorIcon::PointingHand);
        if !was_open {
            res.on_hover_ui(|ui| notice_card(ui, notice));
        }
    }

    /// The Restart button, which runs an action directly instead of opening a
    /// menu.
    ///
    /// Its state comes from the model, where the row is disabled until a deck
    /// is installed; only the picture is decided here.
    fn restart_button(&mut self, ui: &mut Ui) {
        let Some(MenuNode::Item { enabled, .. }) = self.find(&MenuId::Restart) else {
            return;
        };
        let armed = enabled;
        let res = ui.add(
            egui::Button::new("")
                .min_size(ICON_BTN)
                .corner_radius(3.0)
                .sense(if armed {
                    Sense::click()
                } else {
                    Sense::hover()
                }),
        );
        self.widgets.push(res.rect);
        paint_icon(ui, res.rect, MenuIcon::Restart { armed }, true);
        let res = res.on_hover_text(if armed {
            "Restart Emulation"
        } else {
            "Restart Emulation — no firmware installed"
        });
        let res = if armed {
            res.on_hover_cursor(CursorIcon::PointingHand)
        } else {
            res
        };
        if res.clicked() {
            self.clicked.push(MenuId::Restart);
        }
    }

    /// Minimise, maximise or restore, and close, at the strip's right end,
    /// laid out right to left.
    fn caption_buttons(&mut self, ui: &mut Ui) {
        let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(CAPTION_PAD_RIGHT);
        for button in [Caption::Close, Caption::Maximize, Caption::Minimize] {
            let (rect, res) = ui.allocate_exact_size(CAPTION_BTN, Sense::click());
            self.widgets.push(rect);
            let (fill, glyph) = match (button, res.hovered()) {
                (Caption::Close, true) => (CLOSE_RED, Color32::WHITE),
                (_, true) => (STRIP_HOVER, TEXT),
                _ => (Color32::TRANSPARENT, TEXT_MUTED),
            };
            ui.painter().rect_filled(rect, 3.0, fill);
            caption_glyph(ui, rect, button, maximized, glyph);
            if res.clicked() {
                let cmd = match button {
                    Caption::Minimize => egui::ViewportCommand::Minimized(true),
                    Caption::Maximize => egui::ViewportCommand::Maximized(!maximized),
                    Caption::Close => egui::ViewportCommand::Close,
                };
                ui.ctx().send_viewport_cmd(cmd);
            }
        }
        ui.add_space(CAPTION_PAD_LEFT);
    }
}

/// A strip button with no text of its own, lit while its menu is open.
fn plate(open: bool) -> egui::Button<'static> {
    let button = egui::Button::new("").min_size(ICON_BTN).corner_radius(3.0);
    if open {
        return button.fill(STRIP_HOVER);
    }
    button
}

/// A caption button's mark: a bar, one window or two, or a cross.
fn caption_glyph(ui: &Ui, rect: Rect, button: Caption, maximized: bool, color: Color32) {
    let pen = Pen::new(ui, rect, Vec2::splat(11.0), Vec2::splat(11.0), color);
    match button {
        Caption::Minimize => pen.line((1.0, 5.5), (10.0, 5.5), 1.1),
        Caption::Maximize if maximized => {
            pen.rect(1.3, 3.3, 6.4, 6.4, 1.0, 1.1);
            pen.poly(
                &[(3.3, 3.3), (3.3, 1.3), (9.7, 1.3), (9.7, 7.7), (7.7, 7.7)],
                1.1,
            );
        }
        Caption::Maximize => pen.rect(1.3, 1.3, 8.4, 8.4, 1.0, 1.1),
        Caption::Close => {
            pen.line((1.6, 1.6), (9.4, 9.4), 1.2);
            pen.line((9.4, 1.6), (1.6, 9.4), 1.2);
        }
    }
}

/// The app's name, where the strip is the title bar, and the rule after it.
fn wordmark(ui: &mut Ui) {
    let mut job = egui::text::LayoutJob::default();
    let mut format = tracked_fmt(FONT_PILL, TRACK_WORDMARK, TEXT);
    format.font_id = FontId::new(
        FONT_PILL,
        egui::FontFamily::Name(crate::fonts::NIMBUS_SANS_BOLD.into()),
    );
    job.append(
        &crate::app_meta::APP_DISPLAY_NAME.to_uppercase(),
        0.0,
        format,
    );
    ui.label(job);
    vertical_rule(ui, 12.0, PILL_BORDER);
}

/// The card behind a notice badge.
fn notice_card(ui: &mut Ui, notice: &Notice) {
    ui.set_max_width(CARD_W);
    ui.spacing_mut().item_spacing.y = 6.0;
    let mut title = egui::text::LayoutJob::default();
    title.append(
        &notice.title.to_uppercase(),
        0.0,
        tracked_fmt(FONT_SECTION, TRACK_SECTION, WARN_AMBER),
    );
    ui.label(title);
    for para in &notice.body {
        ui.label(egui::RichText::new(para).size(FONT_ROW).color(TEXT_MUTED));
    }
}
