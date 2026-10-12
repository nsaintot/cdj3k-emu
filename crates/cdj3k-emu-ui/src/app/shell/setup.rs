//! The setup window: one window for the whole job of giving a slot an
//! emulation, and the step it shows.

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;
use std::time::Duration;

use cdj3k_emu_platform::desktop;

use super::{CdjShell, Screen, SetupStep};
use crate::app::mods_view;
use crate::app::picker::{
    draw_busy, draw_delete_confirm, draw_header, draw_picker, Header, PickerAction, SubTone,
};
use crate::app::theme;

/// The setup window's size on every step, which is the size of the picker it
/// opens over.
const SETUP_WINDOW_SIZE: [f32; 2] = desktop::PICKER_SIZE;

impl CdjShell {
    /// Shows the setup window, one window for every step of giving this slot
    /// an emulation.
    pub(super) fn show_setup_window(&mut self, ctx: &egui::Context) -> Option<PickerAction> {
        if self.setup.is_none() {
            self.setup_pos = None;
            self.setup_shaped = false;
            return None;
        }
        let closed = Arc::new(AtomicBool::new(false));
        // Over the startup picker, use a second window of the same size and
        // chrome only where the host can centre it on the first.
        let over_the_picker = matches!(self.screen, Screen::Picker);
        let action = if over_the_picker && !desktop::PLACES_WINDOWS {
            self.setup_in_main_window(ctx, &closed)
        } else {
            self.setup_in_own_window(ctx, &closed)
        };
        if closed.load(Relaxed) {
            self.close_setup();
        }
        action
    }

    /// Shows the setup screen as a second OS window, centred on the first
    /// where the host allows it. Wayland ignores the position, and the
    /// compositor places the window.
    fn setup_in_own_window(
        &mut self,
        ctx: &egui::Context,
        closed: &Arc<AtomicBool>,
    ) -> Option<PickerAction> {
        if self.setup_pos.is_none() {
            self.setup_pos = ctx
                .input(|i| i.viewport().outer_rect)
                .map(|r| r.center() - egui::Vec2::from(SETUP_WINDOW_SIZE) * 0.5);
        }
        let title = match self.setup {
            Some(SetupStep::Install) => "Install Firmware",
            Some(SetupStep::Mods) => "Manage Mods",
            _ => "Manage Emulation",
        };
        // The window is not built as non-resizable, because winit would pin
        // min and max to the size at creation, before it knows the scale, and
        // a compositor places the window by that size. The size is pinned from
        // the first frame instead.
        let mut builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size(SETUP_WINDOW_SIZE);
        if let Some(pos) = self.setup_pos {
            builder = builder.with_position(pos);
        }
        let mut action = None;
        let first_frame = !std::mem::replace(&mut self.setup_shaped, true);
        let this = &mut *self;
        let closed_inner = closed.clone();
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("cdj3k_setup"),
            builder,
            |root, _class| {
                if root.input(|i| i.viewport().close_requested()) {
                    closed_inner.store(true, Relaxed);
                }
                // Setting min and max to the same size holds the size; set
                // them again whenever the window's size is wrong.
                let size = egui::Vec2::from(SETUP_WINDOW_SIZE);
                if first_frame || (root.content_rect().size() - size).length() > 1.0 {
                    root.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(size));
                    root.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(size));
                    root.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                }
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(theme::palette(root).paper))
                    .show(root, |ui| {
                        action = this.draw_setup(ui, &closed_inner);
                    });
            },
        );
        action
    }

    /// Shows the setup screen inside the window it came from, over whatever
    /// that window was showing.
    pub(super) fn setup_in_main_window(
        &mut self,
        ctx: &egui::Context,
        closed: &Arc<AtomicBool>,
    ) -> Option<PickerAction> {
        let mut action = None;
        let pal = theme::palette(ctx);
        egui::Area::new(egui::Id::new("cdj3k_setup_over"))
            .order(egui::Order::Middle)
            .fixed_pos(egui::pos2(0.0, 0.0))
            .show(ctx, |ui| {
                let screen = ctx.content_rect();
                ui.painter().rect_filled(screen, 0.0, pal.paper);
                // Block clicks to everything behind the setup screen. This is
                // registered before the setup's widgets, because the last
                // widget registered over a point takes its clicks.
                ui.interact(screen, ui.id().with("block"), egui::Sense::click_and_drag());
                ui.scope_builder(egui::UiBuilder::new().max_rect(screen), |ui| {
                    ui.set_clip_rect(screen);
                    action = self.draw_setup(ui, closed);
                });
            });
        action
    }

    /// Shut the setup window, unless a provisioning run would be orphaned.
    /// Closing returns to what is behind it: the running panel or the main
    /// window's startup picker.
    pub(super) fn close_setup(&mut self) {
        if self.wizard.is_running() || matches!(self.setup, Some(SetupStep::StoppingToDelete)) {
            return;
        }
        self.setup = None;
        self.leave_mods();
        self.view_slot(self.instance);
        self.wizard.reset();
    }

    /// One step of the setup screen, wherever it is being drawn.
    fn draw_setup(&mut self, ui: &mut egui::Ui, closed: &Arc<AtomicBool>) -> Option<PickerAction> {
        match self.setup {
            Some(SetupStep::Install) => {
                self.draw_install_step(ui, closed);
                None
            }
            Some(SetupStep::ConfirmDelete) => {
                draw_delete_confirm(ui, &self.picker_view(None, None, true))
            }
            Some(SetupStep::Mods) => self.draw_mods_step(ui),
            Some(SetupStep::StoppingToDelete) => {
                self.draw_stopping_step(ui);
                None
            }
            Some(SetupStep::Pick) => draw_picker(ui, &self.picker_view(None, None, true)),
            Some(SetupStep::Confirm(model)) => {
                draw_picker(ui, &self.picker_view(None, Some(model), true))
            }
            None => None,
        }
    }

    /// The install wizard under the window's own header.
    fn draw_install_step(&mut self, ui: &mut egui::Ui, closed: &Arc<AtomicBool>) {
        let model = self.wizard.model;
        let sub = format!(
            "Decrypt the {model} .UPD (or take its decrypted .iso), patch the \
             kernel and initramfs, and provision an eMMC image."
        );
        let body = draw_header(
            ui,
            &Header {
                slot: Some(self.viewed_slot),
                model: Some(model),
                release: None,
                state: None,
                title: "Install firmware",
                sub: &sub,
                switchable: false,
                tone: SubTone::Plain,
            },
        )
        .body;
        // The wizard lays out its own gutters and action bar, so it gets the
        // whole body.
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            self.wizard.draw(ui, closed);
        });
    }

    /// Draws the slot's mods. It repaints regularly so the list follows the
    /// slot's files and the boot report, and so a finished pick or download
    /// is collected.
    fn draw_mods_step(&mut self, ui: &mut egui::Ui) -> Option<PickerAction> {
        self.poll_mods();
        let action = self
            .mods
            .as_mut()
            .and_then(|page| mods_view::draw_mods(ui, page))
            .map(PickerAction::Mods);
        ui.ctx().request_repaint_after(Duration::from_millis(500));
        action
    }

    /// Draws the wait while the emulation stops so the slot can be emptied.
    fn draw_stopping_step(&self, ui: &mut egui::Ui) {
        let going = self.model;
        draw_busy(
            ui,
            &Header {
                slot: Some(self.viewed_slot),
                model: Some(going),
                release: self.viewed_release.as_deref(),
                state: Some("STOPPING"),
                title: "Emulation",
                sub: "The emulation must stop before its installation can be changed.",
                switchable: false,
                tone: SubTone::Plain,
            },
            &format!("Stopping the {} emulation…", going.title()),
            "The slot is emptied once the emulation has stopped.",
        );
    }
}
