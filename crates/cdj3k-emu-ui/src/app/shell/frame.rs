//! What the shell does each frame, as named phases in the order they run.

use std::time::{Duration, Instant};

use cdj3k_emu_platform::{desktop, menu_state};

use super::{CdjShell, Screen, SetupStep};
use crate::app::picker::draw_picker;
use crate::app::theme;
use crate::app::updater::UpdaterEvent;

/// How long the window waits for Sparkle to end the process after handing it
/// the quit.
const QUIT_HAND_OFF_WAIT: Duration = Duration::from_secs(15);

static MENU_SETUP: std::sync::Once = std::sync::Once::new();

impl eframe::App for CdjShell {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if matches!(self.screen, Screen::Panel) {
            self.app.tilt_input_hook(raw_input);
        }
    }

    /// eframe calls this before every [`ui`](eframe::App::ui), and also while the
    /// window is minimized or covered, when it does not call `ui`. It closes
    /// the window, relaunches the emulation, sends the requested power-off
    /// frame, serves the window socket and applies finished installs, so these
    /// continue while the window is hidden.
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.hold_close_request(ctx);
        self.finish_shutdown(ctx);
        if let Some(model) = self.initial_model.take() {
            self.launch(ctx, frame, model);
        }
        self.advance_worker(ctx, frame);
        self.app.send_requested_power_off();
        // Settle a finished install before the socket and the install check
        // look at the wizard.
        self.settle_finished_install();
        self.serve_window_socket();
        self.take_replies();
        self.watch_worker(ctx);
        if std::mem::take(&mut self.close_window) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();
        MENU_SETUP.call_once(cdj3k_emu_platform::menu::setup_menu);
        // Hosts with no native menu bar draw one here, above the deck; on
        // macOS this does nothing. The startup picker has its own wordmark and
        // slot switcher, so it gets only the shortcuts.
        if matches!(self.screen, Screen::Panel) {
            cdj3k_emu_platform::menu::draw_in_window(ui);
        } else {
            cdj3k_emu_platform::menu::shortcuts_only(ctx);
        }
        let want_manage = std::mem::take(&mut menu_state::lock().manage_emulation_requested);
        self.save_mods_toggle();
        self.open_manage_on_request(want_manage);
        self.drop_left_mods();
        self.refresh_mods_bar();

        match self.screen {
            Screen::Picker => self.draw_startup_picker(ui, ctx, frame),
            Screen::Panel => self.app.update(ctx, ui, frame),
        }

        // The picker and the setup window read other slots' state on each
        // frame. egui draws no frames while the window is idle, so request a
        // repaint once a second.
        if self.setup.is_some() || matches!(self.screen, Screen::Picker) {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        // The setup window switches the emulation and installs its firmware.
        // The shade gate does not apply to it, because the install step must be
        // reachable while QEMU is not running.
        let action = self.show_setup_window(ctx);
        self.apply_picker_action(action);
        if let Some((slot, model)) = self.wizard.take_boot_request() {
            self.setup = None;
            self.wizard.reset();
            self.on_firmware_provisioned(slot, model);
        }
        if std::mem::take(&mut self.reveal_picker) {
            self.show_picker_window(ctx, frame);
        }
        self.follow_updater(ctx);
        if std::mem::take(&mut self.close_window) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        cdj3k_emu_platform::menu::sync_menu();
        if matches!(self.screen, Screen::Panel) {
            self.script.tick(&mut self.app, ctx, &mut self.shot);
        }
        self.shot.tick(ctx);
    }

    fn on_exit(&mut self, gl: Option<&glow::Context>) {
        self.window_socket = None;
        self.app.on_exit(gl);
    }
}

impl CdjShell {
    /// Defer the actual close until the worker has finished. Blocking inside
    /// `on_exit` for the full runtime-stop budget would freeze the window for
    /// 1-2 s.
    fn hold_close_request(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) {
            return;
        }
        if self.wizard.is_running() && !self.shutdown_in_progress {
            // An install under way finishes first, and its window says so.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        } else if !self.shutdown_in_progress {
            self.shutdown_in_progress = true;
            menu_state::APP_SHUTDOWN.store(true, std::sync::atomic::Ordering::Relaxed);
            menu_state::lock().shade_forced = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        } else if !cdj3k_emu_runtime::worker_is_finished() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
    }

    /// Close the window once the worker has exited after a close request. A
    /// prepared update installs now; Sparkle ends the process itself, and
    /// after [`QUIT_HAND_OFF_WAIT`] the window closes anyway.
    fn finish_shutdown(&mut self, ctx: &egui::Context) {
        if !self.shutdown_in_progress {
            return;
        }
        if !cdj3k_emu_runtime::worker_is_finished() {
            ctx.request_repaint();
            return;
        }
        if self.quit_handed_off.is_none() && self.updater.apply_on_quit() {
            self.quit_handed_off = Some(Instant::now());
        }
        match self.quit_handed_off {
            Some(at) if at.elapsed() < QUIT_HAND_OFF_WAIT => ctx.request_repaint(),
            _ => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    /// Reap a stopped worker, and once none is stopping, empty the slot or
    /// launch what is waiting.
    fn advance_worker(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if self.worker_stopping && cdj3k_emu_runtime::reap_finished_worker() {
            self.worker_stopping = false;
        }
        // The worker exited by itself to restart onto a finished install.
        if cdj3k_emu_runtime::worker_is_finished()
            && std::mem::take(&mut menu_state::lock().relaunch_requested)
        {
            cdj3k_emu_runtime::reap_finished_worker();
            self.pending_launch = Some(self.model);
        }
        if !self.worker_stopping {
            self.run_pending(ctx, frame);
        }
        if self.worker_stopping {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    /// Run the delete or launch that was waiting for the worker to stop.
    fn run_pending(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if std::mem::take(&mut self.pending_delete) {
            // A restart that was stopping the emulation does not boot an
            // emptied slot.
            self.pending_launch = None;
            self.empty_own_slot();
            self.reveal_picker = false;
            self.show_picker_window(ctx, frame);
        }
        if let Some(model) = self.pending_launch.take() {
            if !self.shutdown_in_progress {
                self.launch(ctx, frame, model);
            }
        }
    }

    /// Hand a finished wizard run to the slot it installed into.
    fn settle_finished_install(&mut self) {
        self.wizard.poll();
        if self.wizard.take_finished() {
            let installed = self.settle_install(self.wizard.target_instance());
            self.wizard.set_installed(installed);
        }
    }

    /// Look for an install once the emulation has stopped. While an
    /// emulation runs, the runtime worker can ask for a power-off frame or a
    /// relaunch, and only `logic` acts on them. A deck that draws nothing new
    /// produces no frames, so `logic` runs at least once a second, also while
    /// the window is hidden.
    fn watch_worker(&mut self, ctx: &egui::Context) {
        let finished = cdj3k_emu_runtime::worker_is_finished();
        if finished && !self.worker_was_finished {
            self.install_check_due = true;
        }
        self.worker_was_finished = finished;
        self.check_for_install();
        if !finished {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    /// Save Enable Mods here while no runtime worker runs to save it.
    fn save_mods_toggle(&self) {
        if !cdj3k_emu_runtime::worker_is_finished() {
            return;
        }
        let toggled = {
            let mut s = menu_state::lock();
            std::mem::take(&mut s.mods_toggle_requested).then_some(s.mods_enabled)
        };
        if let Some(on) = toggled {
            if let Err(e) =
                cdj3k_emu_storage::InstanceSettings::update(self.instance, |s| s.mods_enabled = on)
            {
                eprintln!("cdj3k-emu: saving Enable Mods failed: {e}");
            }
        }
    }

    /// Manage Emulation from the menu opens the setup window on this slot. It
    /// does not open over the startup picker, which already shows the same
    /// cards, or while an install is running or the slot is being emptied.
    fn open_manage_on_request(&mut self, asked: bool) {
        if asked
            && matches!(self.screen, Screen::Panel)
            && !self.wizard.is_running()
            && !matches!(self.setup, Some(SetupStep::StoppingToDelete))
        {
            self.view_slot(self.instance);
            self.setup = Some(SetupStep::Pick);
        }
    }

    /// Leaving the Mods view by any route drops its pick, download and
    /// pending replace, so none of them lands in another slot.
    fn drop_left_mods(&mut self) {
        if !matches!(self.setup, Some(SetupStep::Mods))
            && (self.mods.is_some()
                || self.mods_pick.is_some()
                || self.mods_download.is_some()
                || self.mods_pending.is_some())
        {
            self.leave_mods();
        }
    }

    /// Draws the startup picker for a slot with nothing to show. The setup
    /// window handles every later choice.
    fn draw_startup_picker(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        frame: &mut eframe::Frame,
    ) {
        desktop::apply_resize_constraints(ctx, frame, &mut self.resize, None);
        let view = self.picker_view(None, None, false);
        let mut action = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(theme::palette(ctx).paper))
            .show(ui, |ui| {
                action = draw_picker(ui, &view);
            });
        self.apply_picker_action(action);
    }

    /// Act on what the update window asked of the emulation.
    fn follow_updater(&mut self, ctx: &egui::Context) {
        match self
            .updater
            .show(ctx, cdj3k_emu_runtime::worker_is_finished())
        {
            UpdaterEvent::Exit => self.close_window = true,
            UpdaterEvent::StopEmulation => {
                if !self.worker_stopping && !cdj3k_emu_runtime::worker_is_finished() {
                    self.stop_emulation();
                }
            }
            UpdaterEvent::ResumeEmulation => {
                if matches!(self.screen, Screen::Panel) && self.pending_launch.is_none() {
                    self.pending_launch = Some(self.model);
                }
            }
            UpdaterEvent::None => {}
        }
    }
}
