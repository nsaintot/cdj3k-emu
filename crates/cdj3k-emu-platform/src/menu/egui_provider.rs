//! The in-window menu, for hosts with no native menu bar.
//!
//! A strip across the top of the emulation window — inside it, not replacing
//! the title bar: a Linux desktop cannot be relied on to cede its decorations,
//! so the window manager keeps them and this sits beneath.
//!
//! Three ways into the same [`MenuModel`]: a hamburger holding every menu, a
//! pill per menu that also shows what that menu is set to, and an icon strip
//! for the ones worth reaching in one click. None of them decides anything —
//! every click becomes a [`MenuId`] the service applies.
//!
//! egui's own menu machinery does the opening, nesting and dismissing. Rows
//! are painted here because a row has a checkmark gutter and a right-aligned
//! value that a plain button cannot express; a row that opens a submenu goes
//! through `Ui::menu_button`, which is what makes it open beside its parent
//! rather than on top of it.

use std::collections::BTreeSet;

use egui::{
    Align, Color32, CursorIcon, FontId, Layout, Pos2, Rect, Response, Rounding, Sense, Stroke, Ui,
    Vec2,
};

use super::id::MenuId;
use super::model::{MenuIcon, MenuModel, MenuNode, NetKind, Notice, Predefined};
use super::provider::MenuProvider;

// ── Metrics, from the design canvas ───────────────────────────────────────────

/// Height of the strip. The window's aspect lock adds this to the chassis, so
/// changing it here moves the whole layout with it.
pub const BAR_HEIGHT: f32 = 44.0;

const BAR_PAD_X: f32 = 16.0;
const PILL_H: f32 = 26.0;
const ICON_BTN: Vec2 = Vec2::new(30.0, 26.0);
const ICON_GAP: f32 = 2.0;
/// The rule and its air, which travel with the restart button.
const RESTART_RULE_W: f32 = 9.0;
/// Air kept between the last pill and the first shortcut.
const SHORTCUT_GUTTER: f32 = 12.0;

/// Width of a notice's hover card.
const CARD_W: f32 = 280.0;

/// Width a panel starts at. It grows to fit its widest row, up to [`PANEL_MAX_W`].
const PANEL_MIN_W: f32 = 224.0;
/// Past this a device name is truncated rather than shoving the panel wider.
const PANEL_MAX_W: f32 = 420.0;
const PANEL_PAD: f32 = 4.0;

const ROW_H: f32 = 30.0;
const ROW_PAD_X: f32 = 10.0;
/// The checkmark column, which holds its width whether a row is ticked or not.
const GUTTER_W: f32 = 12.0;
const GUTTER_GAP: f32 = 8.0;
/// Where a row's label starts, ticked or not, submenu or not.
const TEXT_LEFT: f32 = ROW_PAD_X + GUTTER_W + GUTTER_GAP;

const FONT_ROW: f32 = 11.5;
const FONT_TRAIL: f32 = 10.5;
const FONT_SECTION: f32 = 9.0;
const FONT_PILL: f32 = 10.0;

/// Letter spacing on the strip's tracked capitals, as a fraction of the size.
const TRACK_WORDMARK: f32 = 0.18;
const TRACK_SECTION: f32 = 0.14;

// The chrome's own palette, independent of the deck theme.
const BAR_BG: Color32 = Color32::from_rgb(0x23, 0x23, 0x20);
const PANEL_BG: Color32 = Color32::from_rgb(0x23, 0x23, 0x20);
const PANEL_BORDER: Color32 = Color32::from_rgb(0x34, 0x33, 0x2E);
const ROW_HOVER: Color32 = Color32::from_rgb(0x2B, 0x2B, 0x27);
/// The strip sits on the same ground its rows do, so a row's hover plate is
/// too faint out here; this is the panel border's value, used as a fill.
const STRIP_HOVER: Color32 = Color32::from_rgb(0x34, 0x33, 0x2E);
const TEXT: Color32 = Color32::from_rgb(0xF0, 0xEE, 0xE8);
const TEXT_MUTED: Color32 = Color32::from_rgb(0xA5, 0xA1, 0x99);
const TEXT_DETAIL: Color32 = Color32::from_rgb(0x8E, 0x8A, 0x81);
const TEXT_DIM: Color32 = Color32::from_rgb(0x7C, 0x78, 0x6F);
const TEXT_DISABLED: Color32 = Color32::from_rgb(0x6A, 0x67, 0x5F);
const PILL_BORDER: Color32 = Color32::from_rgb(0x4A, 0x48, 0x42);
const ICON_RULE: Color32 = Color32::from_rgb(0x34, 0x33, 0x2E);
const ACCENT: Color32 = Color32::from_rgb(0x38, 0x84, 0xFF);
const OK_GREEN: Color32 = Color32::from_rgb(0x5F, 0xBE, 0x8A);
const WARN_AMBER: Color32 = Color32::from_rgb(0xD9, 0xA2, 0x3C);

/// Draws the model as a strip, and collects what was clicked.
#[derive(Default)]
pub struct EguiProvider {
    model: MenuModel,
    clicked: Vec<MenuId>,
    /// Openers whose menu was up last frame, so a pill can show itself as the
    /// open one. egui settles that during the frame, which is too late to
    /// style the button that opens it.
    open: BTreeSet<String>,
    styled: bool,
    /// Where this frame's strip and open panels were drawn, so an effect
    /// painted over the whole window can leave them alone.
    chrome: Vec<Rect>,
}

impl MenuProvider for EguiProvider {
    fn apply(&mut self, model: &MenuModel) {
        // Immediate mode: there is nothing to reconcile, the next frame draws
        // whatever this holds.
        self.model = model.clone();
    }

    fn poll(&mut self) -> Vec<MenuId> {
        std::mem::take(&mut self.clicked)
    }

    fn as_egui(&mut self) -> Option<&mut EguiProvider> {
        Some(self)
    }
}

impl EguiProvider {
    /// The strip and every panel it showed this frame, in points.
    pub fn chrome(&self) -> &[Rect] {
        &self.chrome
    }

    /// Take the keyboard shortcuts without drawing anything.
    pub fn shortcuts_only(&mut self, ctx: &egui::Context) {
        self.chrome.clear();
        self.take_shortcuts(ctx);
    }

    /// Draw the strip. Call once per frame, before the chassis.
    pub fn draw(&mut self, ctx: &egui::Context) {
        if !self.styled {
            style_menus(ctx);
            self.styled = true;
        }
        self.take_shortcuts(ctx);
        self.chrome.clear();

        let frame = egui::Frame::none()
            .fill(BAR_BG)
            .inner_margin(egui::Margin::symmetric(BAR_PAD_X, 0.0));
        let strip = egui::TopBottomPanel::top("cdj3k_menu_bar")
            .exact_height(BAR_HEIGHT)
            .frame(frame)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| self.bar(ui));
            });
        self.chrome.push(strip.response.rect);
    }

    /// Run any action whose accelerator was pressed.
    ///
    /// The bindings come from [`MenuId::accelerator`], so they cannot drift
    /// from what the menu shows beside each row.
    fn take_shortcuts(&mut self, ctx: &egui::Context) {
        let mut hits = Vec::new();
        ctx.input(|i| {
            for id in ids_in(&self.model) {
                let Some(accel) = id.accelerator() else {
                    continue;
                };
                let Some(key) = egui::Key::from_name(accel.key) else {
                    continue;
                };
                let mods = egui::Modifiers {
                    command: accel.primary,
                    shift: accel.shift,
                    ..egui::Modifiers::NONE
                };
                if i.key_pressed(key) && i.modifiers.matches_logically(mods) {
                    hits.push(id.clone());
                }
            }
        });
        self.clicked.extend(hits);
    }

    fn bar(&mut self, ui: &mut Ui) {
        let mut open = BTreeSet::new();
        // Our own layout rather than `egui::menu::bar`, which fixes the band
        // to `interact_size.y` and pins it to the top of the strip, and which
        // overwrites the button padding the pills are measured against.
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            style_strip(ui);
            self.hamburger(ui, &mut open);

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
            let mut room = ui.available_width() - SHORTCUT_GUTTER;
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
        let mut button = egui::Button::new("").min_size(ICON_BTN).rounding(3.0);
        if self.open.contains(HAMBURGER) {
            button = button.fill(STRIP_HOVER);
        }
        // Emulation and Instances have their own pill an inch away, so the
        // sandwich holds what the strip cannot reach in one click.
        let mut roots: Vec<MenuNode> = self
            .model
            .roots
            .iter()
            .filter(|n| !matches!(n, MenuNode::Submenu { label, .. } if label == "Emulation" || label == "Instances"))
            .cloned()
            .collect();
        roots.push(MenuNode::Separator);
        roots.push(MenuNode::Predefined(Predefined::Quit));
        let res = egui::menu::menu_custom_button(ui, button, |ui| {
            self.panel(ui, &roots);
        });
        if res.inner.is_some() {
            open.insert(HAMBURGER.into());
        }
        let c = res.response.rect.center();
        let s = Stroke::new(1.4_f32, TEXT);
        for dy in [-4.0, 0.0, 4.0] {
            ui.painter().hline(c.x - 6.0..=c.x + 6.0, c.y + dy, s);
        }
        res.response.on_hover_cursor(CursorIcon::PointingHand);
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
        // The accent square is painted, so the label leads past where it goes.
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
            .rounding(3.0)
            .stroke(Stroke::new(1.0_f32, if was_open { TEXT } else { PILL_BORDER }));
        if was_open {
            button = button.fill(STRIP_HOVER);
        }
        let children = Self::menu_rows(children);
        let res = egui::menu::menu_custom_button(ui, button, |ui| {
            self.panel(ui, &children);
        });
        if res.inner.is_some() {
            open.insert(label.clone());
        }

        let rect = res.response.rect;
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
        res.response.on_hover_cursor(CursorIcon::PointingHand);
    }

    /// One glyph, one click, the menu behind it.
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
        let was_open = self.open.contains(label);

        let mut button = egui::Button::new("").min_size(ICON_BTN).rounding(3.0);
        if was_open {
            button = button.fill(STRIP_HOVER);
        }
        let glyph = *icon;
        let children = Self::menu_rows(children);
        let tip = match status {
            Some(s) => format!("{label} — {s}"),
            None => label.clone(),
        };

        // A disabled menu keeps its place and its glyph, faded, and opens
        // nothing.
        ui.add_enabled_ui(*enabled, |ui| {
            let res = egui::menu::menu_custom_button(ui, button, |ui| {
                self.panel(ui, &children);
            });
            if res.inner.is_some() {
                open.insert(label.clone());
            }
            if let Some(g) = glyph {
                paint_icon(ui, res.response.rect, g);
            }
            res.response
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(tip)
                .on_disabled_hover_text(format!("{label} — available once the deck has booted"));
        });
    }

    /// A condition flagged on the strip: the design's gauge glyph, whose card
    /// shows on hover and stays up on a click.
    fn notice_badge(&mut self, ui: &mut Ui, notice: &Notice, open: &mut BTreeSet<String>) {
        let was_open = self.open.contains(NOTICE);
        let mut button = egui::Button::new("").min_size(ICON_BTN).rounding(3.0);
        if was_open {
            button = button.fill(STRIP_HOVER);
        }
        let res = egui::menu::menu_custom_button(ui, button, |ui| {
            egui::Frame::none()
                .inner_margin(egui::Margin::same(ROW_PAD_X - PANEL_PAD))
                .show(ui, |ui| notice_card(ui, notice));
            self.chrome.push(ui.min_rect().expand(PANEL_PAD + 1.0));
        });
        if res.inner.is_some() {
            open.insert(NOTICE.into());
        }
        let pen = Pen::new(ui, res.response.rect, Vec2::splat(15.0), Vec2::splat(15.0), WARN_AMBER);
        pen.arc(7.5, 9.0, 6.0, 0.4286, 1.0714, 1.1);
        pen.line((7.5, 9.0), (4.0, 8.2), 1.3);
        pen.fill_circle(7.5, 9.0, 1.1);
        let res = res.response.on_hover_cursor(CursorIcon::PointingHand);
        if !was_open {
            res.on_hover_ui(|ui| notice_card(ui, notice));
        }
    }

    /// Restart, as a verb rather than a menu: the one action worth a click.
    ///
    /// Its state comes from the model — the row is disabled until a deck is
    /// installed — and only the picture is decided here.
    fn restart_button(&mut self, ui: &mut Ui) {
        let Some(MenuNode::Item { enabled, .. }) = self.find(&MenuId::Restart) else {
            return;
        };
        let armed = enabled;
        let res = ui.add(
            egui::Button::new("")
                .min_size(ICON_BTN)
                .rounding(3.0)
                .sense(if armed {
                    Sense::click()
                } else {
                    Sense::hover()
                }),
        );
        paint_icon(ui, res.rect, MenuIcon::Restart { armed });
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

    // ── Panels ────────────────────────────────────────────────────────────────

    /// A pill or icon opens one menu, so it shows that menu without the Quit
    /// the native app menu needs at the end of the first one.
    fn menu_rows(children: &[MenuNode]) -> Vec<MenuNode> {
        let mut rows: Vec<MenuNode> = children
            .iter()
            .filter(|n| !matches!(n, MenuNode::Predefined(Predefined::Quit)))
            .cloned()
            .collect();
        while matches!(rows.last(), Some(MenuNode::Separator)) {
            rows.pop();
        }
        rows
    }

    /// A menu's rows, sized to the widest of them.
    ///
    /// The panel itself — fill, border, rounding, margin — is egui's own menu
    /// frame, styled by [`style_menus`]; a second frame here would double the
    /// border.
    fn panel(&mut self, ui: &mut Ui, nodes: &[MenuNode]) {
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
            MenuNode::Separator => {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(row_w, 9.0), Sense::hover());
                ui.painter().hline(
                    rect.x_range(),
                    rect.center().y,
                    Stroke::new(1.0_f32, PANEL_BORDER),
                );
            }
            MenuNode::Section(text) => {
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
            MenuNode::Label(text) => {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(row_w, ROW_H), Sense::hover());
                ui.painter().text(
                    Pos2::new(rect.left() + TEXT_LEFT, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    text,
                    FontId::proportional(FONT_ROW),
                    TEXT_DISABLED,
                );
            }
            MenuNode::Readout {
                title,
                value,
                detail,
            } => {
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
            // Quit is ours to draw: the platform owns it only where there is a
            // native menu bar.
            MenuNode::Predefined(Predefined::Quit) => {
                if self
                    .row(ui, row_w, "Quit", None, true, Some("Ctrl Q"))
                    .clicked()
                {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    ui.close_menu();
                }
            }
            MenuNode::Item {
                id,
                label,
                enabled,
                detail,
            } => {
                let trailing = detail
                    .clone()
                    .or_else(|| id.accelerator().map(|a| a.label()));
                if self
                    .row(ui, row_w, label, None, *enabled, trailing.as_deref())
                    .clicked()
                {
                    self.clicked.push(id.clone());
                    ui.close_menu();
                }
            }
            MenuNode::Check {
                id,
                label,
                enabled,
                checked,
                detail,
            } => {
                let trailing = detail
                    .clone()
                    .or_else(|| id.accelerator().map(|a| a.label()));
                if self
                    .row(
                        ui,
                        row_w,
                        label,
                        Some(*checked),
                        *enabled,
                        trailing.as_deref(),
                    )
                    .clicked()
                {
                    self.clicked.push(id.clone());
                    ui.close_menu();
                }
            }
            MenuNode::Submenu {
                label,
                enabled,
                children,
                ..
            } => {
                // `Ui::menu_button` inside a menu is what opens the child
                // beside its parent instead of on top of it, and draws the
                // arrow that says it will.
                let children = Self::menu_rows(children);
                let mut job = egui::text::LayoutJob::default();
                job.append(
                    label,
                    TEXT_LEFT - ROW_PAD_X,
                    egui::TextFormat {
                        font_id: FontId::proportional(FONT_ROW),
                        color: if *enabled { TEXT } else { TEXT_DISABLED },
                        ..Default::default()
                    },
                );
                ui.add_enabled_ui(*enabled, |ui| {
                    style_submenu(ui, row_w);
                    let res = ui.menu_button(job, |ui| self.panel(ui, &children));
                    res.response.on_hover_cursor(CursorIcon::PointingHand);
                });
            }
        }
    }

    /// One painted row: checkmark gutter, label, right-aligned value.
    ///
    /// Painted rather than built from a button because egui has no way to put
    /// a trailing column inside one, and the gutter has to hold its width
    /// whether the row is ticked or not or nothing lines up.
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

    /// The node carrying an action, wherever it sits.
    fn find(&self, want: &MenuId) -> Option<MenuNode> {
        fn walk(nodes: &[MenuNode], want: &MenuId) -> Option<MenuNode> {
            for n in nodes {
                if n.id() == Some(want) {
                    return Some(n.clone());
                }
                if let MenuNode::Submenu { children, .. } = n {
                    if let Some(hit) = walk(children, want) {
                        return Some(hit);
                    }
                }
            }
            None
        }
        walk(&self.model.roots, want)
    }

    fn root(&self, label: &str) -> Option<MenuNode> {
        self.model
            .roots
            .iter()
            .find(|n| matches!(n, MenuNode::Submenu { label: l, .. } if l == label))
            .cloned()
    }
}

// ── Sizing ────────────────────────────────────────────────────────────────────

/// How wide a panel has to be for its widest row to fit.
///
/// A menu whose rows are device names is far wider than one of verbs, and a
/// fixed width would either truncate the first or waste half the second.
fn panel_width(ui: &Ui, nodes: &[MenuNode]) -> f32 {
    let mut want: f32 = PANEL_MIN_W;
    for n in nodes {
        let (label, detail, trailing) = match n {
            MenuNode::Item {
                label, detail, id, ..
            }
            | MenuNode::Check {
                label, detail, id, ..
            } => (
                label.as_str(),
                detail
                    .clone()
                    .or_else(|| id.accelerator().map(|a| a.label())),
                true,
            ),
            MenuNode::Predefined(Predefined::Quit) => ("Quit", Some("Ctrl Q".into()), true),
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

fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.fonts(|f| f.layout_no_wrap(text.to_owned(), font, TEXT).size().x)
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

// ── Text ──────────────────────────────────────────────────────────────────────

fn tracked_fmt(size: f32, track: f32, color: Color32) -> egui::TextFormat {
    egui::TextFormat {
        font_id: FontId::proportional(size),
        extra_letter_spacing: size * track,
        color,
        ..Default::default()
    }
}

fn mono_fmt(size: f32, color: Color32) -> egui::TextFormat {
    egui::TextFormat {
        font_id: FontId::monospace(size),
        color,
        ..Default::default()
    }
}

fn paint_tracked(
    ui: &Ui,
    pos: Pos2,
    align: egui::Align2,
    text: &str,
    size: f32,
    track: f32,
    color: Color32,
) {
    let mut job = egui::text::LayoutJob::default();
    job.append(text, 0.0, tracked_fmt(size, track, color));
    job.wrap.max_width = f32::INFINITY;
    let galley = ui.fonts(|f| f.layout_job(job));
    let at = align.align_size_within_rect(galley.size(), Rect::from_center_size(pos, Vec2::ZERO));
    ui.painter().galley(at.min, galley, color);
}

/// The id under which the sandwich records itself as open. Not a menu label,
/// so it can never collide with one.
const HAMBURGER: &str = "\u{1}menu";
/// The same, for the notice badge's card.
const NOTICE: &str = "\u{1}notice";

/// The strip's buttons: flat until hovered, with the design's inner padding.
///
/// Set here rather than on each button, because a `Button` given an explicit
/// fill uses it in every state and so can never light up.
fn style_strip(ui: &mut Ui) {
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
        v.rounding = Rounding::same(3.0);
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
/// It is built from the context's style rather than the opener's, so this has
/// to be set there. The setup window overrides the same fields on its own Ui
/// tree, so its popups keep their palette.
fn style_menus(ctx: &egui::Context) {
    ctx.style_mut(|s| {
        s.visuals.window_fill = PANEL_BG;
        s.visuals.window_stroke = Stroke::new(1.0_f32, PANEL_BORDER);
        s.visuals.menu_rounding = Rounding::same(3.0);
        s.visuals.popup_shadow = egui::epaint::Shadow::NONE;
        s.spacing.menu_margin = egui::Margin::same(PANEL_PAD);
        s.spacing.menu_spacing = 6.0;
    });
}

/// Make `Ui::menu_button` draw like one of our rows: same height, same left
/// edge, same hover plate.
fn style_submenu(ui: &mut Ui, row_w: f32) {
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
        v.rounding = Rounding::same(2.0);
        v.bg_stroke = Stroke::NONE;
        v.fg_stroke = Stroke::new(1.0_f32, TEXT);
        v.expansion = 0.0;
    }
    s.visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    s.visuals.widgets.hovered.weak_bg_fill = ROW_HOVER;
    s.visuals.widgets.active.weak_bg_fill = ROW_HOVER;
    s.visuals.widgets.open.weak_bg_fill = ROW_HOVER;
}

// ── Marks ─────────────────────────────────────────────────────────────────────

fn vertical_rule(ui: &mut Ui, h: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, h), Sense::hover());
    ui.painter()
        .vline(rect.center().x, rect.y_range(), Stroke::new(1.0_f32, color));
}

/// The design's chevron: an open V, centred on `c`, 9 x 6.
fn chevron(ui: &Ui, c: Pos2, color: Color32) {
    let s = Stroke::new(1.3_f32, color);
    ui.painter().add(egui::Shape::line(
        vec![
            c + Vec2::new(-3.5, -1.7),
            c + Vec2::new(0.0, 1.7),
            c + Vec2::new(3.5, -1.7),
        ],
        s,
    ));
}

/// The design's tick, drawn from its left-centre.
fn check_mark(p: &egui::Painter, left: Pos2, color: Color32) {
    let s = Stroke::new(1.4_f32, color);
    p.add(egui::Shape::line(
        vec![
            left + Vec2::new(1.0, 0.1),
            left + Vec2::new(4.2, 3.1),
            left + Vec2::new(11.0, -3.3),
        ],
        s,
    ));
}

/// Draws a design icon in the coordinates it was drawn in.
///
/// Every glyph in the strip comes from the canvas as an SVG on a viewBox; this
/// maps that box onto the button so the proportions survive, and keeps the
/// stroke widths in the same units.
struct Pen<'a> {
    p: &'a egui::Painter,
    origin: Pos2,
    k: f32,
    color: Color32,
}

impl<'a> Pen<'a> {
    /// `vb` is the SVG viewBox size, `draw` the size it is drawn at.
    fn new(ui: &'a Ui, rect: Rect, vb: Vec2, draw: Vec2, color: Color32) -> Self {
        let k = draw.x / vb.x;
        Self {
            p: ui.painter(),
            origin: rect.center() - draw * 0.5,
            k,
            color,
        }
    }

    fn at(&self, x: f32, y: f32) -> Pos2 {
        self.origin + Vec2::new(x, y) * self.k
    }

    fn stroke(&self, w: f32) -> Stroke {
        Stroke::new(w * self.k, self.color)
    }

    fn line(&self, a: (f32, f32), b: (f32, f32), w: f32) {
        self.p
            .line_segment([self.at(a.0, a.1), self.at(b.0, b.1)], self.stroke(w));
    }

    fn poly(&self, pts: &[(f32, f32)], w: f32) {
        self.p.add(egui::Shape::line(
            pts.iter().map(|(x, y)| self.at(*x, *y)).collect(),
            self.stroke(w),
        ));
    }

    fn closed_poly(&self, pts: &[(f32, f32)], w: f32) {
        self.p.add(egui::Shape::closed_line(
            pts.iter().map(|(x, y)| self.at(*x, *y)).collect(),
            self.stroke(w),
        ));
    }

    fn rect(&self, x: f32, y: f32, w: f32, h: f32, r: f32, sw: f32) {
        self.p.rect_stroke(
            Rect::from_min_size(self.at(x, y), Vec2::new(w, h) * self.k),
            Rounding::same(r * self.k),
            self.stroke(sw),
        );
    }

    fn fill_rect(&self, x: f32, y: f32, w: f32, h: f32) {
        self.p.rect_filled(
            Rect::from_min_size(self.at(x, y), Vec2::new(w, h) * self.k),
            0.0,
            self.color,
        );
    }

    fn fill_circle(&self, x: f32, y: f32, r: f32) {
        self.p.circle_filled(self.at(x, y), r * self.k, self.color);
    }

    /// An arc, angles in turns measured the way SVG does (clockwise on screen).
    fn arc(&self, cx: f32, cy: f32, r: f32, from: f32, to: f32, w: f32) {
        let n = 32;
        let pts = (0..=n)
            .map(|i| {
                let t = from + (to - from) * i as f32 / n as f32;
                let a = t * std::f32::consts::TAU;
                self.at(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        self.p.add(egui::Shape::line(pts, self.stroke(w)));
    }

    fn bezier(&self, pts: [(f32, f32); 4], w: f32) {
        self.p
            .add(egui::epaint::CubicBezierShape::from_points_stroke(
                [
                    self.at(pts[0].0, pts[0].1),
                    self.at(pts[1].0, pts[1].1),
                    self.at(pts[2].0, pts[2].1),
                    self.at(pts[3].0, pts[3].1),
                ],
                false,
                Color32::TRANSPARENT,
                self.stroke(w),
            ));
    }
}

/// Which glyph stands for a state is this provider's business; the state
/// itself came from the model. Each is the design canvas's own SVG, in its
/// own coordinates.
fn paint_icon(ui: &Ui, rect: Rect, icon: MenuIcon) {
    let sq = Vec2::splat(15.0);
    match icon {
        MenuIcon::Restart { armed } => {
            let pen = Pen::new(
                ui,
                rect,
                sq,
                sq,
                if armed { TEXT_MUTED } else { TEXT_DISABLED },
            );
            pen.arc(7.5, 7.5, 4.7, 0.0, 0.8690, 1.2);
            pen.poly(&[(11.4, 1.6), (11.4, 4.5), (8.5, 4.5)], 1.2);
        }
        MenuIcon::Storage { mounted } => {
            let vb = Vec2::new(11.0, 16.0);
            let pen = Pen::new(
                ui,
                rect,
                vb,
                vb,
                if mounted { TEXT_MUTED } else { TEXT_DISABLED },
            );
            // The connector shell and its two contacts, over the body.
            pen.rect(2.5, 0.5, 6.0, 4.0, 0.0, 1.0);
            pen.fill_rect(4.0, 2.0, 1.0, 1.0);
            pen.fill_rect(6.0, 2.0, 1.0, 1.0);
            pen.rect(0.5, 4.5, 10.0, 11.0, 2.0, 1.0);
            if mounted {
                pen.p.circle_filled(pen.at(5.5, 11.0), 1.75 * pen.k, OK_GREEN);
            }
        }
        MenuIcon::Network(kind) => match kind {
            NetKind::Nat => paint_tracked(
                ui,
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "NAT",
                9.0,
                0.12,
                TEXT_MUTED,
            ),
            NetKind::LinkLocal => {
                let pen = Pen::new(ui, rect, sq, sq, TEXT_MUTED);
                pen.rect(1.6, 4.6, 4.8, 5.8, 1.0, 1.1);
                pen.rect(8.6, 4.6, 4.8, 5.8, 1.0, 1.1);
                pen.line((6.4, 7.5), (8.6, 7.5), 1.1);
            }
            NetKind::Bridged => {
                let pen = Pen::new(ui, rect, sq, sq, TEXT_MUTED);
                pen.rect(5.1, 1.5, 4.8, 3.6, 0.8, 1.1);
                pen.rect(1.3, 9.9, 4.8, 3.6, 0.8, 1.1);
                pen.rect(8.9, 9.9, 4.8, 3.6, 0.8, 1.1);
                pen.line((7.5, 5.1), (7.5, 7.5), 1.1);
                pen.poly(&[(3.7, 9.9), (3.7, 7.5), (11.3, 7.5), (11.3, 9.9)], 1.1);
            }
        },
        MenuIcon::Audio { on } => {
            let pen = Pen::new(
                ui,
                rect,
                sq,
                sq,
                if on { TEXT_MUTED } else { TEXT_DISABLED },
            );
            pen.closed_poly(
                &[
                    (2.6, 5.8),
                    (5.0, 5.8),
                    (8.0, 3.2),
                    (8.0, 11.8),
                    (5.0, 9.2),
                    (2.6, 9.2),
                ],
                1.1,
            );
            if on {
                pen.bezier([(10.3, 5.5), (11.3, 6.7), (11.3, 8.3), (10.3, 9.5)], 1.1);
                pen.bezier([(12.1, 4.0), (13.9, 6.1), (13.9, 8.9), (12.1, 11.0)], 1.1);
            } else {
                pen.line((10.0, 6.0), (13.2, 9.2), 1.1);
                pen.line((13.2, 6.0), (10.0, 9.2), 1.1);
            }
        }
        MenuIcon::View | MenuIcon::Instances | MenuIcon::Emulation => {
            let pen = Pen::new(ui, rect, sq, sq, TEXT_MUTED);
            pen.rect(1.8, 2.8, 8.6, 6.4, 1.1, 1.1);
            pen.rect(4.8, 5.8, 8.6, 6.4, 1.1, 1.1);
        }
    }
}

/// Every action the model can raise, for the shortcut sweep.
fn ids_in(model: &MenuModel) -> Vec<MenuId> {
    fn walk(nodes: &[MenuNode], out: &mut Vec<MenuId>) {
        for n in nodes {
            match n {
                MenuNode::Item { id, .. } | MenuNode::Check { id, .. } => out.push(id.clone()),
                MenuNode::Submenu { children, .. } => walk(children, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&model.roots, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::model::MenuIcon;

    fn model() -> MenuModel {
        MenuModel {
            roots: vec![
                MenuNode::submenu(
                    "Emulation",
                    vec![
                        MenuNode::item(MenuId::Restart, "Restart Emulation"),
                        MenuNode::Predefined(Predefined::Quit),
                    ],
                )
                .with_status("CDJ-3000", MenuIcon::Emulation)
                .with_detail("3.20"),
                MenuNode::submenu(
                    "Audio",
                    vec![MenuNode::check(MenuId::Audio, "Enable Audio", true)],
                )
                .with_status("141 ms", MenuIcon::Audio { on: true }),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn the_shortcut_sweep_reaches_nested_actions() {
        let ids = ids_in(&model());
        assert!(ids.contains(&MenuId::Restart), "{ids:?}");
        assert!(ids.contains(&MenuId::Audio), "{ids:?}");
        // Quit is predefined, not an action of ours.
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn every_bound_key_is_one_egui_knows() {
        // A binding egui cannot name would be silently dead, and the row
        // would still advertise it.
        for id in ids_in(&model()) {
            if let Some(accel) = id.accelerator() {
                assert!(
                    egui::Key::from_name(accel.key).is_some(),
                    "{:?} binds {:?}, which egui cannot name",
                    id,
                    accel.key
                );
            }
        }
    }

    #[test]
    fn a_pill_reads_out_its_state_rather_than_its_name() {
        let p = EguiProvider {
            model: model(),
            ..Default::default()
        };
        let MenuNode::Submenu { status, .. } = p.root("Audio").unwrap() else {
            panic!("expected a submenu")
        };
        assert_eq!(status.as_deref(), Some("141 ms"));
        assert!(p.root("Nothing").is_none());
    }
}

#[cfg(test)]
mod frame_tests {
    use super::*;

    /// Everything the strip painted this frame, as (text, rect) pairs.
    ///
    /// Rows are painted rather than built from widgets, so tests locate them
    /// in the shape list.
    fn texts(out: &egui::FullOutput) -> Vec<(String, Rect)> {
        out.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some((t.galley.job.text.clone(), t.visual_bounding_rect())),
                _ => None,
            })
            .collect()
    }

    fn click_at(pos: Pos2) -> egui::RawInput {
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::new(0.0, 0.0),
                Vec2::new(900.0, 600.0),
            )),
            ..Default::default()
        };
        input.events.push(egui::Event::PointerMoved(pos));
        for pressed in [true, false] {
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            });
        }
        input
    }

    /// A model with the shapes the panel has to be able to draw.
    fn model() -> MenuModel {
        MenuModel {
            roots: vec![MenuNode::submenu(
                "Storage",
                vec![
                    MenuNode::item(MenuId::ManageEmulation, "Manage Emulation"),
                    MenuNode::Separator,
                    MenuNode::Section("Slots".into()),
                    MenuNode::check(MenuId::ServiceMode, "Service Mode", true),
                    MenuNode::Readout {
                        title: "Pipeline latency".into(),
                        value: "141 ms".into(),
                        detail: None,
                    },
                    MenuNode::Predefined(Predefined::Quit),
                ],
            )
            .with_status("CDJ-3000", MenuIcon::Emulation)
            .with_detail("3.20")],
            ..Default::default()
        }
    }

    /// The hamburger opens, the rows are drawn, and clicking one raises its
    /// action — the whole path a menu click takes, without a window.
    #[test]
    fn clicking_a_row_in_the_opened_menu_raises_its_action() {
        let ctx = egui::Context::default();
        let mut p = EguiProvider::default();
        p.apply(&model());

        // Frame one finds the hamburger; nothing is open yet.
        let out = ctx.run(Default::default(), |ctx| p.draw(ctx));
        assert!(
            !texts(&out).iter().any(|(t, _)| t == "Manage Emulation"),
            "the menu drew its rows before anything opened it"
        );
        let hamburger = Pos2::new(BAR_PAD_X + 15.0, BAR_HEIGHT / 2.0);

        // Frame two opens it; the hamburger holds the menus, so what appears
        // is one opener per menu.
        let _ = ctx.run(click_at(hamburger), |ctx| p.draw(ctx));
        let out = ctx.run(Default::default(), |ctx| p.draw(ctx));
        let opener = texts(&out)
            .into_iter()
            .find(|(t, _)| t == "Storage")
            .map(|(_, r)| r)
            .expect("the hamburger did not list the menus");

        // Frame three opens that menu, frame four sees its rows.
        let _ = ctx.run(click_at(opener.center()), |ctx| p.draw(ctx));
        let out = ctx.run(Default::default(), |ctx| p.draw(ctx));
        let drawn = texts(&out);
        for want in [
            "Manage Emulation",
            "Service Mode",
            "SLOTS",
            "141 ms",
            "Quit",
        ] {
            assert!(
                drawn.iter().any(|(t, _)| t == want),
                "{want:?} missing from {:?}",
                drawn.iter().map(|(t, _)| t).collect::<Vec<_>>()
            );
        }

        let row = drawn
            .iter()
            .find(|(t, _)| t == "Service Mode")
            .map(|(_, r)| *r)
            .expect("the row was just asserted present");
        let _ = ctx.run(click_at(row.center()), |ctx| p.draw(ctx));
        assert_eq!(p.poll(), vec![MenuId::ServiceMode]);
    }

    /// The strip's pills read out state rather than menu names.
    #[test]
    fn the_strip_shows_the_deck_not_the_word_emulation() {
        let ctx = egui::Context::default();
        let mut p = EguiProvider::default();
        p.apply(&MenuModel {
            roots: vec![MenuNode::submenu("Emulation", vec![])
                .with_status("CDJ-3000", MenuIcon::Emulation)
                .with_detail("3.20")],
            ..Default::default()
        });
        let out = ctx.run(Default::default(), |ctx| p.draw(ctx));
        let drawn: Vec<_> = texts(&out).into_iter().map(|(t, _)| t).collect();
        assert!(drawn.iter().any(|t| t.contains("CDJ-3000")), "{drawn:?}");
        assert!(drawn.iter().any(|t| t.contains("3.20")), "{drawn:?}");
        assert!(!drawn.iter().any(|t| t == "Emulation"), "{drawn:?}");
    }

    /// A notice adds its glyph to the strip; without one, nothing is drawn.
    #[test]
    fn a_notice_puts_its_badge_on_the_strip() {
        let ctx = egui::Context::default();
        let shapes = |notice: Option<Notice>| {
            let mut p = EguiProvider::default();
            p.apply(&MenuModel {
                roots: vec![MenuNode::submenu("Instances", vec![])
                    .with_status("Slot 1", MenuIcon::Instances)],
                notice,
            });
            ctx.run(Default::default(), |ctx| p.draw(ctx)).shapes.len()
        };
        let flagged = shapes(Some(Notice {
            title: "Software emulation (TCG)".into(),
            body: vec!["why".into()],
        }));
        assert!(flagged > shapes(None), "the notice drew nothing");
    }

    /// The strip reports itself as chrome, so a whole-window pass can skip it.
    #[test]
    fn the_strip_is_chrome() {
        let ctx = egui::Context::default();
        let mut p = EguiProvider::default();
        p.apply(&model());
        let _ = ctx.run(Default::default(), |ctx| p.draw(ctx));
        assert!(
            p.chrome().iter().any(|r| (r.height() - BAR_HEIGHT).abs() < 0.5),
            "{:?}",
            p.chrome()
        );
    }

    /// A panel is as wide as its widest row, not as wide as the window, with
    /// a separator in it too.
    #[test]
    fn a_panel_fits_its_rows_rather_than_the_window() {
        let ctx = egui::Context::default();
        let mut measured = (0.0, 0.0);
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                measured.0 = panel_width(ui, model().roots[0].children());
                measured.1 = panel_width(
                    ui,
                    &[MenuNode::check(
                        MenuId::PhysicalDisk(0),
                        "Some Very Long External Drive Name · 2 TB",
                        false,
                    )
                    .with_detail("disk4")],
                );
            });
        });
        assert_eq!(measured.0, PANEL_MIN_W, "short verbs stay at the minimum");
        assert!(
            measured.1 > PANEL_MIN_W && measured.1 <= PANEL_MAX_W,
            "a long device name widens the panel, to a point: {measured:?}"
        );
    }

    /// Hovering lights a strip button and a menu row, as seen in the painted
    /// shapes.
    #[test]
    fn hovering_lights_the_thing_under_the_pointer() {
        // A menu Area fades in, so what reaches the shape list is the plate
        // multiplied by the animation's opacity. Compare loosely.
        fn filled(out: &egui::FullOutput, want: Color32) -> bool {
            out.shapes.iter().any(|c| match &c.shape {
                egui::Shape::Rect(r) => {
                    let a = r.fill.a() as f32 / 255.0;
                    a > 0.5
                        && [0, 1, 2].iter().all(|&i| {
                            let got = r.fill.to_array()[i] as f32 / a;
                            (got - want.to_array()[i] as f32).abs() <= 6.0
                        })
                }
                _ => false,
            })
        }
        fn hover(pos: Pos2) -> egui::RawInput {
            let mut input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    Pos2::new(0.0, 0.0),
                    Vec2::new(900.0, 600.0),
                )),
                ..Default::default()
            };
            input.events.push(egui::Event::PointerMoved(pos));
            input
        }

        let ctx = egui::Context::default();
        let mut p = EguiProvider::default();
        p.apply(&model());

        let out = ctx.run(Default::default(), |ctx| p.draw(ctx));
        assert!(!filled(&out, STRIP_HOVER), "lit with the pointer elsewhere");

        let burger = Pos2::new(BAR_PAD_X + 15.0, BAR_HEIGHT / 2.0);
        let _ = ctx.run(hover(burger), |ctx| p.draw(ctx));
        let out = ctx.run(hover(burger), |ctx| p.draw(ctx));
        assert!(filled(&out, STRIP_HOVER), "the sandwich did not light up");

        // Open it, then hover its first row.
        let _ = ctx.run(click_at(burger), |ctx| p.draw(ctx));
        let out = ctx.run(Default::default(), |ctx| p.draw(ctx));
        let row = texts(&out)
            .into_iter()
            .find(|(t, _)| t == "Quit")
            .map(|(_, r)| r)
            .expect("the sandwich did not list Quit");
        let _ = ctx.run(hover(row.center()), |ctx| p.draw(ctx));
        let out = ctx.run(hover(row.center()), |ctx| p.draw(ctx));
        assert!(filled(&out, ROW_HOVER), "the row did not light up");
    }

    /// The pills have their own place in the strip, so the sandwich holds the
    /// rest — not a second copy of them.
    #[test]
    fn the_sandwich_leaves_out_what_the_pills_already_reach() {
        let ctx = egui::Context::default();
        let mut p = EguiProvider::default();
        p.apply(&MenuModel {
            roots: vec![
                model().roots[0].clone(),
                MenuNode::submenu(
                    "Instances",
                    vec![MenuNode::check(MenuId::Instance(1), "Slot 1", true)],
                )
                .with_status("Slot 1", MenuIcon::Instances),
                MenuNode::submenu(
                    "View",
                    vec![MenuNode::check(
                        MenuId::ScreenExtended,
                        "Extend Screen",
                        false,
                    )],
                )
                .with_icon(MenuIcon::View),
            ],
            ..Default::default()
        });
        let burger = Pos2::new(BAR_PAD_X + 15.0, BAR_HEIGHT / 2.0);
        let _ = ctx.run(Default::default(), |ctx| p.draw(ctx));
        let _ = ctx.run(click_at(burger), |ctx| p.draw(ctx));
        let out = ctx.run(Default::default(), |ctx| p.draw(ctx));
        let drawn: Vec<_> = texts(&out).into_iter().map(|(t, _)| t).collect();
        assert!(drawn.iter().any(|t| t == "View"), "{drawn:?}");
        assert!(drawn.iter().any(|t| t == "Quit"), "{drawn:?}");
        assert!(!drawn.iter().any(|t| t == "Emulation"), "{drawn:?}");
        assert!(!drawn.iter().any(|t| t == "Instances"), "{drawn:?}");
    }

    /// Every strip button lights under the pointer, not just some of them.
    #[test]
    fn every_strip_button_lights() {
        fn hover(pos: Pos2) -> egui::RawInput {
            let mut input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    Pos2::new(0.0, 0.0),
                    Vec2::new(900.0, 600.0),
                )),
                ..Default::default()
            };
            input.events.push(egui::Event::PointerMoved(pos));
            input
        }
        let ctx = egui::Context::default();
        let mut p = EguiProvider::default();
        p.apply(&MenuModel {
            roots: vec![
                MenuNode::submenu(
                    "Emulation",
                    vec![MenuNode::item_enabled(
                        MenuId::Restart,
                        "Restart Emulation",
                        true,
                    )],
                )
                .with_status("CDJ-3000", MenuIcon::Emulation),
                MenuNode::submenu("View", vec![]).with_icon(MenuIcon::View),
            ],
            ..Default::default()
        });
        let _ = ctx.run(Default::default(), |ctx| p.draw(ctx));

        // Sweep the strip rather than guess where each button landed: what
        // matters is that every one of them lights, wherever it is.
        let mut lit_rects: Vec<Rect> = Vec::new();
        let mut x = 18.0;
        while x < 890.0 {
            let at = Pos2::new(x, BAR_HEIGHT / 2.0);
            let _ = ctx.run(hover(at), |ctx| p.draw(ctx));
            let out = ctx.run(hover(at), |ctx| p.draw(ctx));
            if let Some(r) = out.shapes.iter().find_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.fill == STRIP_HOVER && r.rect.contains(at) => {
                    Some(r.rect)
                }
                _ => None,
            }) {
                if !lit_rects.iter().any(|k| (k.left() - r.left()).abs() < 1.0) {
                    lit_rects.push(r);
                }
            }
            x += 2.0;
        }
        // Sandwich, the Emulation pill, Restart, View.
        assert_eq!(
            lit_rects.len(),
            4,
            "only these lit: {:?}",
            lit_rects.iter().map(|r| r.x_range()).collect::<Vec<_>>()
        );
    }

    /// A shortcut that cannot be used is dimmed, not outlined.
    ///
    /// egui draws a disabled widget from `noninteractive`, which is a
    /// different entry from the four a hover walks through.
    #[test]
    fn a_disabled_shortcut_has_no_border() {
        let ctx = egui::Context::default();
        let mut p = EguiProvider::default();
        p.apply(&MenuModel {
            roots: vec![MenuNode::submenu(
                "Emulation",
                vec![MenuNode::item_enabled(
                    MenuId::Restart,
                    "Restart Emulation",
                    false,
                )],
            )
            .with_status("No firmware", MenuIcon::Emulation)],
            ..Default::default()
        });
        let _ = ctx.run(Default::default(), |ctx| p.draw(ctx));
        let out = ctx.run(Default::default(), |ctx| p.draw(ctx));
        // The pill is the only thing in the strip that carries an outline.
        let outlined: Vec<_> = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Rect(r)
                    if r.stroke.width > 0.0 && (r.rect.height() - PILL_H).abs() < 1.0 =>
                {
                    Some(r.rect.width())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            outlined.len(),
            1,
            "only the pill should be outlined, got {outlined:?}"
        );
    }

    /// The sandwich ends in Quit; a pill's own menu does not repeat it.
    ///
    /// The model keeps it in the first menu because the native bar needs it
    /// there, so leaving it out is the provider's job.
    #[test]
    fn only_the_sandwich_carries_quit() {
        let with_quit = MenuModel {
            roots: vec![MenuNode::submenu(
                "Emulation",
                vec![
                    MenuNode::item(MenuId::ManageEmulation, "Manage Emulation"),
                    MenuNode::Separator,
                    MenuNode::Predefined(Predefined::Quit),
                ],
            )
            .with_status("CDJ-3000", MenuIcon::Emulation)],
            ..Default::default()
        };
        let rows = EguiProvider::menu_rows(with_quit.roots[0].children());
        assert!(!rows
            .iter()
            .any(|n| matches!(n, MenuNode::Predefined(Predefined::Quit))));
        // And the separator that led to it goes with it.
        assert!(
            !matches!(rows.last(), Some(MenuNode::Separator)),
            "{rows:?}"
        );

        let ctx = egui::Context::default();
        let mut p = EguiProvider::default();
        p.apply(&with_quit);
        let _ = ctx.run(Default::default(), |ctx| p.draw(ctx));
        let burger = Pos2::new(BAR_PAD_X + 15.0, BAR_HEIGHT / 2.0);
        let _ = ctx.run(click_at(burger), |ctx| p.draw(ctx));
        let out = ctx.run(Default::default(), |ctx| p.draw(ctx));
        let drawn: Vec<_> = texts(&out).into_iter().map(|(t, _)| t).collect();
        assert!(drawn.iter().any(|t| t == "Quit"), "{drawn:?}");
    }

    /// A narrow window sheds shortcuts instead of drawing them over the pills.
    #[test]
    fn the_shortcuts_give_way_to_the_pills() {
        fn strip_at(width: f32) -> Vec<Rect> {
            let ctx = egui::Context::default();
            let mut p = EguiProvider::default();
            p.apply(&MenuModel {
                roots: vec![
                    MenuNode::submenu(
                        "Emulation",
                        vec![MenuNode::item(MenuId::Restart, "Restart Emulation")],
                    )
                    .with_status("CDJ-3000", MenuIcon::Emulation)
                    .with_detail("3.22"),
                    MenuNode::submenu("Instances", vec![])
                        .with_status("Slot 1", MenuIcon::Instances),
                    MenuNode::submenu("Storage", vec![])
                        .with_icon(MenuIcon::Storage { mounted: false }),
                    MenuNode::submenu("Network", vec![]).with_icon(MenuIcon::Network(NetKind::Nat)),
                    MenuNode::submenu("Audio", vec![]).with_icon(MenuIcon::Audio { on: true }),
                    MenuNode::submenu("View", vec![]).with_icon(MenuIcon::View),
                ],
                ..Default::default()
            });
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    Pos2::new(0.0, 0.0),
                    Vec2::new(width, 400.0),
                )),
                ..Default::default()
            };
            let out = ctx.run(input.clone(), |ctx| p.draw(ctx));
            let _ = out;
            let out = ctx.run(input, |ctx| p.draw(ctx));
            // The buttons' own plates: the artwork inside an icon is smaller
            // than any of them, and the bar's ground is taller.
            out.shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Rect(r) => Some(r.rect),
                    _ => None,
                })
                .filter(|r| {
                    r.top() < BAR_HEIGHT
                        && (r.height() - PILL_H).abs() < 1.0
                        && r.width() >= ICON_BTN.x - 1.0
                })
                .collect()
        }

        fn overlaps(rects: &[Rect]) -> bool {
            rects.iter().enumerate().any(|(i, a)| {
                rects[i + 1..]
                    .iter()
                    .any(|b| a.intersects(*b) && a.width() > 1.0 && b.width() > 1.0)
            })
        }

        let wide = strip_at(900.0);
        assert!(!overlaps(&wide), "{wide:?}");
        let wide_n = wide.len();

        for width in [520.0, 460.0, 400.0, 340.0] {
            let got = strip_at(width);
            assert!(!overlaps(&got), "at {width}: {got:?}");
            assert!(
                got.len() <= wide_n,
                "at {width} the strip grew rather than shed: {got:?}"
            );
        }
        assert!(
            strip_at(340.0).len() < wide_n,
            "a narrow strip shed nothing"
        );
    }

    /// Each View menu row is reachable and raises its own action.
    #[test]
    fn every_view_row_raises_its_own_action() {
        let rows = [
            (MenuId::ScreenExtended, "Extend Screen"),
            (MenuId::JogScreen, "External Jog Screen"),
            (MenuId::MainScreen, "External Main Screen"),
            (MenuId::DebugScreen, "Debug Panel"),
        ];
        for (id, label) in &rows {
            let ctx = egui::Context::default();
            let mut p = EguiProvider::default();
            p.apply(&MenuModel {
                roots: vec![MenuNode::submenu(
                    "View",
                    rows.iter()
                        .map(|(i, l)| MenuNode::check(i.clone(), *l, false))
                        .collect(),
                )
                .with_icon(MenuIcon::View)],
                ..Default::default()
            });
            let burger = Pos2::new(BAR_PAD_X + 15.0, BAR_HEIGHT / 2.0);
            let _ = ctx.run(Default::default(), |ctx| p.draw(ctx));
            let _ = ctx.run(click_at(burger), |ctx| p.draw(ctx));
            let out = ctx.run(Default::default(), |ctx| p.draw(ctx));
            let opener = texts(&out)
                .into_iter()
                .find(|(t, _)| t == "View")
                .map(|(_, r)| r)
                .expect("the sandwich did not list View");
            let _ = ctx.run(click_at(opener.center()), |ctx| p.draw(ctx));
            let out = ctx.run(Default::default(), |ctx| p.draw(ctx));
            let row = texts(&out)
                .into_iter()
                .find(|(t, _)| t == label)
                .map(|(_, r)| r)
                .unwrap_or_else(|| panic!("{label:?} is not in the View menu"));
            let _ = ctx.run(click_at(row.center()), |ctx| p.draw(ctx));
            assert_eq!(p.poll(), vec![id.clone()], "clicking {label:?}");
        }
    }

    /// Every glyph the strip types has to exist in the font, or it draws as a
    /// box. The marks that do not are painted; these are the ones that are not.
    #[test]
    fn the_typed_glyphs_all_exist() {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |_| {});
        let font = FontId::proportional(FONT_ROW);
        for s in ["CDJ3K EMULATOR", "NAT", "Ctrl Q", "141 ms", "·", "—", "…"] {
            assert!(
                ctx.fonts(|f| f.has_glyphs(&font, s)),
                "{s:?} has no glyph and would draw as a box"
            );
        }
    }
}
