//! The panels a menu opens: their width and their rows.

use egui::{CursorIcon, FontId, Pos2, Response, Sense, Stroke, Ui, Vec2};

use super::paint::{check_mark, paint_tracked, text_width};
use super::style::{
    style_submenu, FONT_ROW, FONT_SECTION, FONT_TRAIL, PANEL_BORDER, PANEL_MAX_W, PANEL_MIN_W,
    PANEL_PAD, ROW_H, ROW_HOVER, ROW_PAD_X, TEXT, TEXT_DETAIL, TEXT_DIM, TEXT_DISABLED, TEXT_LEFT,
    TRACK_SECTION,
};
use super::EguiProvider;
use crate::menu::id::MenuId;
use crate::menu::model::{MenuNode, Predefined};

/// The Quit row's label and shortcut hint.
const QUIT_LABEL: &str = "Quit";
const QUIT_KEYS: &str = "Ctrl Q";

impl EguiProvider {
    /// A menu's rows, sized to the widest of them.
    ///
    /// The panel itself — fill, border, rounding, margin — is egui's own menu
    /// frame, styled by [`super::style::style_menus`]; a second frame here would double the
    /// border.
    pub(super) fn panel(&mut self, ui: &mut Ui, nodes: &[MenuNode]) {
        let row_w = panel_width(ui, nodes) - PANEL_PAD * 2.0;
        ui.set_min_width(row_w);
        ui.set_max_width(row_w);
        ui.spacing_mut().item_spacing.y = 0.0;
        for node in nodes {
            self.node(ui, node, row_w);
        }
        self.chrome.push(ui.min_rect().expand(PANEL_PAD + 1.0));
    }

    /// One node of a panel, whatever kind it is.
    fn node(&mut self, ui: &mut Ui, node: &MenuNode, row_w: f32) {
        match node {
            MenuNode::Separator => separator_row(ui, row_w),
            MenuNode::Section(text) => section_row(ui, row_w, text),
            MenuNode::Label(text) => label_row(ui, row_w, text),
            MenuNode::Readout {
                title,
                value,
                detail,
            } => readout_row(ui, row_w, title, value, detail.as_deref()),
            // This provider draws Quit; the platform draws it only where there
            // is a native menu bar.
            MenuNode::Predefined(Predefined::Quit) => {
                if self
                    .row(ui, row_w, QUIT_LABEL, None, true, Some(QUIT_KEYS))
                    .clicked()
                {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    ui.close();
                }
            }
            MenuNode::Item {
                id,
                label,
                enabled,
                detail,
            } => {
                let trailing = trailing_text(id, detail);
                let res = self.row(ui, row_w, label, None, *enabled, trailing.as_deref());
                self.raise_on_click(ui, &res, id);
            }
            MenuNode::Check {
                id,
                label,
                enabled,
                checked,
                detail,
            } => {
                let trailing = trailing_text(id, detail);
                let res = self.row(
                    ui,
                    row_w,
                    label,
                    Some(*checked),
                    *enabled,
                    trailing.as_deref(),
                );
                self.raise_on_click(ui, &res, id);
            }
            MenuNode::Submenu {
                label,
                enabled,
                children,
                ..
            } => self.submenu_row(ui, row_w, label, *enabled, children),
        }
    }

    /// Raise a row's action once it is clicked, and close the menu.
    fn raise_on_click(&mut self, ui: &mut Ui, res: &Response, id: &MenuId) {
        if res.clicked() {
            self.clicked.push(id.clone());
            ui.close();
        }
    }

    /// A row that opens a menu beside it.
    ///
    /// `Ui::menu_button` inside a menu is what opens the child beside its
    /// parent instead of on top of it, and draws the arrow that says it will.
    fn submenu_row(
        &mut self,
        ui: &mut Ui,
        row_w: f32,
        label: &str,
        enabled: bool,
        children: &[MenuNode],
    ) {
        let children = Self::menu_rows(children);
        let mut job = egui::text::LayoutJob::default();
        job.append(
            label,
            TEXT_LEFT - ROW_PAD_X,
            egui::TextFormat {
                font_id: FontId::proportional(FONT_ROW),
                color: if enabled { TEXT } else { TEXT_DISABLED },
                ..Default::default()
            },
        );
        ui.add_enabled_ui(enabled, |ui| {
            style_submenu(ui, row_w);
            let res = ui.menu_button(job, |ui| self.panel(ui, &children));
            res.response.on_hover_cursor(CursorIcon::PointingHand);
        });
    }

    /// One painted row: checkmark gutter, label, right-aligned value.
    ///
    /// The row is painted instead of built from a button, because egui cannot
    /// put a trailing column inside a button. The gutter keeps its width
    /// whether or not the row is ticked, so the labels line up.
    fn row(
        &self,
        ui: &mut Ui,
        row_w: f32,
        label: &str,
        check: Option<bool>,
        enabled: bool,
        trailing: Option<&str>,
    ) -> Response {
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(Vec2::new(row_w, ROW_H), sense);
        if !ui.is_rect_visible(rect) {
            return response;
        }

        let painter = ui.painter();
        if enabled && response.hovered() {
            painter.rect_filled(rect, 2.0, ROW_HOVER);
        }
        let fg = if enabled { TEXT } else { TEXT_DISABLED };

        if check == Some(true) {
            check_mark(
                painter,
                Pos2::new(rect.left() + ROW_PAD_X, rect.center().y),
                fg,
            );
        }
        let mut right = rect.right() - ROW_PAD_X;
        if let Some(t) = trailing {
            let g = painter.text(
                Pos2::new(right, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                t,
                FontId::monospace(FONT_TRAIL),
                TEXT_DIM,
            );
            right = g.left() - 8.0;
        }
        painter.text(
            Pos2::new(rect.left() + TEXT_LEFT, rect.center().y),
            egui::Align2::LEFT_CENTER,
            elide(ui, label, right - rect.left() - TEXT_LEFT),
            FontId::proportional(FONT_ROW),
            fg,
        );
        if enabled {
            return response.on_hover_cursor(CursorIcon::PointingHand);
        }
        response
    }
}

// ── Inert rows ────────────────────────────────────────────────────────────────

fn separator_row(ui: &mut Ui, row_w: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(row_w, 9.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0_f32, PANEL_BORDER),
    );
}

fn section_row(ui: &mut Ui, row_w: f32, text: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(row_w, 21.0), Sense::hover());
    paint_tracked(
        ui,
        Pos2::new(rect.left() + ROW_PAD_X, rect.center().y + 2.0),
        egui::Align2::LEFT_CENTER,
        &text.to_uppercase(),
        FONT_SECTION,
        TRACK_SECTION,
        TEXT_DIM,
    );
}

fn label_row(ui: &mut Ui, row_w: f32, text: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(row_w, ROW_H), Sense::hover());
    ui.painter().text(
        Pos2::new(rect.left() + TEXT_LEFT, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        FontId::proportional(FONT_ROW),
        TEXT_DISABLED,
    );
}

/// A tracked title over a figure, and the figure's detail under it.
fn readout_row(ui: &mut Ui, row_w: f32, title: &str, value: &str, detail: Option<&str>) {
    let h = if detail.is_some() { 56.0 } else { 40.0 };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(row_w, h), Sense::hover());
    let x = rect.left() + ROW_PAD_X;
    paint_tracked(
        ui,
        Pos2::new(x, rect.top() + 12.0),
        egui::Align2::LEFT_CENTER,
        &title.to_uppercase(),
        FONT_SECTION,
        TRACK_SECTION,
        TEXT_DIM,
    );
    ui.painter().text(
        Pos2::new(x, rect.top() + 29.0),
        egui::Align2::LEFT_CENTER,
        value,
        FontId::monospace(14.0),
        TEXT,
    );
    if let Some(d) = detail {
        ui.painter().text(
            Pos2::new(x, rect.top() + 45.0),
            egui::Align2::LEFT_CENTER,
            d,
            FontId::monospace(FONT_TRAIL),
            TEXT_DETAIL,
        );
    }
}

// ── Sizing ────────────────────────────────────────────────────────────────────

/// What a row shows at its right edge: its value, or failing that its
/// shortcut.
fn trailing_text(id: &MenuId, detail: &Option<String>) -> Option<String> {
    detail
        .clone()
        .or_else(|| id.accelerator().map(|a| a.label()))
}

/// How wide a panel has to be for its widest row to fit.
///
/// A menu whose rows are device names is far wider than one of verbs, and a
/// fixed width would either truncate the first or waste half the second.
pub(super) fn panel_width(ui: &Ui, nodes: &[MenuNode]) -> f32 {
    let mut want: f32 = PANEL_MIN_W;
    for n in nodes {
        let (label, detail, trailing) = match n {
            MenuNode::Item {
                label, detail, id, ..
            }
            | MenuNode::Check {
                label, detail, id, ..
            } => (label.as_str(), trailing_text(id, detail), true),
            MenuNode::Predefined(Predefined::Quit) => (QUIT_LABEL, Some(QUIT_KEYS.into()), true),
            // A submenu row ends in the arrow, which sits where a value would.
            MenuNode::Submenu { label, .. } => (label.as_str(), None, true),
            MenuNode::Label(t) => (t.as_str(), None, false),
            _ => continue,
        };
        let mut w = TEXT_LEFT + text_width(ui, label, FontId::proportional(FONT_ROW)) + ROW_PAD_X;
        if let Some(t) = &detail {
            w += 8.0 + text_width(ui, t, FontId::monospace(FONT_TRAIL));
        } else if trailing {
            w += 10.0;
        }
        want = want.max(w);
    }
    want.min(PANEL_MAX_W).round()
}

/// Cut a label that will not fit, so a long device name widens the panel to
/// the cap and is then trimmed rather than running under its own value.
fn elide(ui: &Ui, text: &str, avail: f32) -> String {
    let font = FontId::proportional(FONT_ROW);
    if text_width(ui, text, font.clone()) <= avail {
        return text.to_owned();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let mut probe = out.clone();
        probe.push(ch);
        probe.push('…');
        if text_width(ui, &probe, font.clone()) > avail {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}
