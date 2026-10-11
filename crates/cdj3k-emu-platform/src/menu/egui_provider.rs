//! The in-window menu, for hosts with no native menu bar.
//!
//! A strip across the top of the emulation window. On Linux it sits under the
//! window manager's title bar. On Windows it is the title bar
//! ([`desktop::CAPTION_IN_STRIP`]): it carries the app's name and the caption
//! buttons, and its empty stretches drag the window.
//!
//! The strip offers three ways into the same [`MenuModel`]: a hamburger that
//! holds every menu, a pill per menu that also shows what that menu is set to,
//! and icons for the menus reached in one click. Every click becomes a
//! [`MenuId`] that the service applies.
//!
//! egui's own menu machinery does the opening, nesting and dismissing. Rows
//! are painted here because a row has a checkmark gutter and a right-aligned
//! value that a plain button cannot express. A row that opens a submenu goes
//! through `Ui::menu_button`, so that the submenu opens beside its parent.
//!
//! `strip` lays out and draws the strip, `panel` the menus it opens, `icon`
//! and `paint` the glyphs and marks both use, and `style` holds the metrics
//! and palette.

use std::collections::BTreeSet;

use egui::Rect;

use crate::desktop::{self, CaptionArea, CAPTION_IN_STRIP};

use super::id::MenuId;
use super::model::{MenuModel, MenuNode, Predefined};
use super::provider::MenuProvider;

mod icon;
mod paint;
mod panel;
mod strip;
mod style;

pub use style::BAR_HEIGHT;
use style::{style_menus, BAR_BG, BAR_PAD_X};

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
    /// This frame's widgets on the strip, where a press does not drag the
    /// window.
    widgets: Vec<Rect>,
}

impl MenuProvider for EguiProvider {
    fn apply(&mut self, model: &MenuModel) {
        // egui is immediate mode, so there is nothing to reconcile: the next
        // frame draws whatever this holds.
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
        desktop::set_caption_area(ctx, None);
        self.take_shortcuts(ctx);
    }

    /// Draw the strip at the top of `ui`, the window's root. Call once per
    /// frame, before the chassis.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = &ui.ctx().clone();
        if !self.styled {
            style_menus(ctx);
            self.styled = true;
        }
        self.take_shortcuts(ctx);
        self.chrome.clear();
        self.widgets.clear();

        let frame = egui::Frame::NONE
            .fill(BAR_BG)
            .inner_margin(egui::epaint::MarginF32 {
                left: BAR_PAD_X,
                right: if CAPTION_IN_STRIP { 0.0 } else { BAR_PAD_X },
                top: 0.0,
                bottom: 0.0,
            });
        let strip = egui::Panel::top("cdj3k_menu_bar")
            .exact_size(BAR_HEIGHT)
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| self.bar(ui));
            });
        self.chrome.push(strip.response.rect);
        if CAPTION_IN_STRIP {
            desktop::set_caption_area(
                ctx,
                Some(CaptionArea {
                    strip: strip.response.rect,
                    widgets: std::mem::take(&mut self.widgets),
                    pixels_per_point: ctx.pixels_per_point(),
                }),
            );
        }
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

    // ── The model, as the strip reads it ──────────────────────────────────────

    /// A pill or icon opens one menu. It shows that menu without the Quit row,
    /// which the native app menu needs at the end of the first menu, and
    /// without the update row, which the sandwich lists first.
    fn menu_rows(children: &[MenuNode]) -> Vec<MenuNode> {
        let mut rows: Vec<MenuNode> = children
            .iter()
            .filter(|n| !matches!(n, MenuNode::Predefined(Predefined::Quit)) && !is_update_row(n))
            .cloned()
            .skip_while(|n| matches!(n, MenuNode::Separator))
            .collect();
        while matches!(rows.last(), Some(MenuNode::Separator)) {
            rows.pop();
        }
        rows
    }

    /// The update row, wherever the model puts it.
    fn update_row(&self) -> Option<MenuNode> {
        self.model.roots.iter().find_map(|root| match root {
            MenuNode::Submenu { children, .. } => {
                children.iter().find(|n| is_update_row(n)).cloned()
            }
            _ => None,
        })
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

/// Whether `node` is the update row, which the sandwich lists first and the
/// pills leave out.
fn is_update_row(node: &MenuNode) -> bool {
    matches!(
        node,
        MenuNode::Item {
            id: MenuId::CheckForUpdate,
            ..
        }
    )
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
        // A binding that egui cannot name would never fire, while the row
        // still shows it.
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
mod frame_tests;
