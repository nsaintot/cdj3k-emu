//! Top-level app shell: the model picker, then the running panel.
//!
//! The shell owns the window shape (compact picker vs aspect-locked panel),
//! the native menu, the install-firmware wizard and the graceful-close
//! sequence. Booting is delegated to a [`RuntimeHost`] supplied by the binary
//! (it owns the runtime worker); the panel itself is the persistent
//! [`CdjApp`], which keeps its LCD streams across model switches.
//!
//! A slot emulates one model. Changing which one, and installing the firmware
//! it needs, are steps of ONE setup window ([`SetupStep`]) that opens over the
//! running panel - two floating windows for two halves of the same job read as
//! clutter.
//!
//! An install never touches a running emulation. It is written beside the
//! slot's live installation, and the process that owns the slot swaps it in
//! whenever the slot's emulation is not running: at once when nothing runs,
//! otherwise on its next start - a restart from the menu, the guest
//! rebooting, or Restart on the finished install. Restart pressed in another
//! window only leaves a request in the staging dir; the owner sees it and
//! restarts itself, so no window ever stops another's emulation.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cdj3k_emu_panel::Model;
use cdj3k_emu_platform::{app_meta, desktop, menu_state};

use super::firmware_wizard::{FirmwareWizard, Installed};
use super::picker::{
    draw_busy, draw_delete_confirm, draw_header, draw_picker, Header, PickerAction, PickerView,
};
use super::screenshot::ScreenshotDriver;
use super::script::ScriptDriver;
use super::theme;
use super::CdjApp;

/// Result of asking the host to boot a model.
pub enum LaunchOutcome {
    /// A runtime worker (and QEMU) is up for the model.
    Started,
    /// No firmware is provisioned for the model in this slot.
    NotProvisioned,
    /// `--no-spawn`: nothing to boot, show the panel anyway.
    UiOnly,
}

/// The binary's side of a launch: what the slot holds and runtime bring-up.
pub trait RuntimeHost {
    /// The model installed in this slot, if any. A slot holds one
    /// installation; installing another model replaces it.
    fn installed_model(&self) -> Option<Model>;
    /// `--no-spawn`: nothing is provisioned, but every model still draws.
    fn any_model_launchable(&self) -> bool {
        false
    }
    /// Delete the slot's installation. Called only while no worker is alive -
    /// QEMU holds the eMMC open.
    fn clear_firmware(&mut self) -> std::io::Result<()>;
    /// Swap in a finished install waiting for the slot, if there is one.
    /// Called only while no worker is alive, for the same reason.
    fn apply_staged(&mut self) -> std::io::Result<Option<cdj3k_emu_storage::StagedRecord>>;
    /// Start the runtime worker for `model`. Called only while no worker is
    /// alive.
    fn launch(&mut self, model: Model) -> LaunchOutcome;
}

pub struct ShellConfig {
    pub instance: u32,
    pub socket_dir: String,
    pub profile: bool,
    /// Launch this model at startup instead of showing the picker
    /// (`--model`, or the slot's saved model).
    pub initial_model: Option<Model>,
    pub host: Box<dyn RuntimeHost>,
}

/// The setup window is one size whatever step it is on, and the same size as
/// the picker it opens over, so nothing jumps as the job moves along.
const SETUP_WINDOW_SIZE: [f32; 2] = desktop::PICKER_SIZE;

enum Screen {
    Picker,
    Panel,
}

/// Where the setup window is in the job of giving this slot an emulation.
enum SetupStep {
    /// Choosing which deck the slot should be.
    Pick,
    /// Asking whether this deck may take the slot over.
    Confirm(Model),
    /// Installing firmware.
    Install,
    /// Asking whether the slot may be emptied.
    ConfirmDelete,
    /// The emulation is being stopped so the slot can be emptied: QEMU holds
    /// the eMMC open until it is gone.
    StoppingToDelete,
}

pub struct CdjShell {
    screen: Screen,
    app: CdjApp,
    instance: u32,
    /// The model last launched (or the picker's pending choice).
    model: Model,
    /// The model a panel was opened for in this process, if any.
    last_launched: Option<Model>,
    host: Box<dyn RuntimeHost>,
    initial_model: Option<Model>,
    wizard: FirmwareWizard,
    /// A picker choice waiting for the previous runtime worker to finish
    /// its graceful stop.
    pending_launch: Option<Model>,
    /// When the slot's staging dir is next looked at for a finished install.
    next_install_check: Instant,
    /// The last [`cdj3k_emu_storage::slot_in_use`] probe: slot, when, answer.
    /// The picker asks every frame; the probe takes two locks.
    busy_probe: Cell<Option<(u32, Instant, bool)>>,
    /// The slot is to be emptied once the worker is gone.
    pending_delete: bool,
    /// The slot the setup window is showing. Its own by default; the slot
    /// chip points it at another, which is a view, not a handover - the other
    /// slot's emulation runs in its own window.
    viewed_slot: u32,
    /// The firmware release installed in [`viewed_slot`](Self::viewed_slot),
    /// re-read whenever the window is pointed somewhere.
    viewed_release: Option<String>,
    /// The setup window, and where it is; `None` while it is closed.
    setup: Option<SetupStep>,
    /// Where on screen the setup window opened, taken from the main window so
    /// the two sit exactly on top of each other. Held for as long as the
    /// window is up, so it stays where the user may have dragged it.
    setup_pos: Option<egui::Pos2>,
    /// The setup window emptied a slot whose emulation had already stopped,
    /// so the main window has to stop being a panel.
    reveal_picker: bool,
    /// The shell asked the worker to exit; cleared once it has.
    worker_stopping: bool,
    /// `true` once the user has triggered a close (red button / Cmd-Q / menu).
    /// While set, the actual window close is cancelled each frame so the
    /// boot shade can fade in over the going-away UI; the close is re-issued
    /// once the runtime worker has finished its graceful stop, at which
    /// point `on_exit` runs without a multi-second freeze.
    shutdown_in_progress: bool,
    shot: ScreenshotDriver,
    script: ScriptDriver,
}

static MENU_SETUP: std::sync::Once = std::sync::Once::new();

impl CdjShell {
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: ShellConfig) -> Self {
        let model = cfg.initial_model.unwrap_or_default();
        Self {
            screen: Screen::Picker,
            app: CdjApp::new(model, cfg.socket_dir, cc.egui_ctx.clone(), cfg.profile),
            instance: cfg.instance,
            model,
            last_launched: None,
            host: cfg.host,
            initial_model: cfg.initial_model,
            wizard: FirmwareWizard::new(),
            pending_launch: None,
            next_install_check: Instant::now(),
            busy_probe: Cell::new(None),
            pending_delete: false,
            viewed_slot: cfg.instance,
            viewed_release: cdj3k_emu_storage::slot_release(cfg.instance),
            setup: None,
            setup_pos: None,
            reveal_picker: false,
            worker_stopping: false,
            shutdown_in_progress: false,
            shot: ScreenshotDriver::from_env(),
            script: ScriptDriver::from_env(),
        }
    }

    fn window_title(&self, with_model: bool) -> String {
        let base = format!("{} - {}", app_meta::APP_DISPLAY_NAME, self.instance);
        if with_model {
            format!("{base} · {}", self.model.title())
        } else {
            base
        }
    }

    /// Boot `model` (no worker alive) and show its panel; with no firmware,
    /// open the install wizard for it instead. A finished install waiting for
    /// the slot is swapped in first, and boots whatever model it holds.
    fn launch(&mut self, ctx: &egui::Context, frame: &eframe::Frame, mut model: Model) {
        self.model = model;
        // A worker already running this model only wants its panel back; a
        // second QEMU over the live one is not a thing the runtime allows.
        if self.last_launched == Some(model)
            && !self.worker_stopping
            && !cdj3k_emu_runtime::worker_is_finished()
        {
            self.enter_panel(ctx, frame);
            return;
        }
        if let Some(installed) = self.apply_staged() {
            model = installed;
        } else if let Some(installed) = self.host.installed_model() {
            // Asked for a model the slot no longer holds - the install that
            // was to bring it was replaced before it went in: boot what is
            // there rather than open an install over a stopped deck.
            model = installed;
        }
        self.model = model;
        self.app.switch_model(model);
        match self.host.launch(model) {
            LaunchOutcome::NotProvisioned => {
                eprintln!(
                    "cdj3k-emu: no {} firmware in slot {} - opening Install Firmware",
                    model, self.instance
                );
                self.open_wizard(model, self.instance);
            }
            LaunchOutcome::Started | LaunchOutcome::UiOnly => {
                self.last_launched = Some(model);
                self.enter_panel(ctx, frame);
            }
        }
    }

    fn enter_panel(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        self.screen = Screen::Panel;
        // The setup window may have closed onto a stopped emulation and asked
        // for the picker on its way out; this deck is what the window shows
        // instead.
        self.reveal_picker = false;
        // The setup window was in front; bring the deck out from behind it.
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        menu_state::lock().model = Some(self.model);
        desktop::enter_panel_window(frame, self.instance, self.model, self.app.ref_canvas());
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.window_title(true)));
    }

    /// Ask the emulation to stop, if one is running.
    fn stop_emulation(&mut self) {
        if !cdj3k_emu_runtime::worker_is_finished() {
            menu_state::lock().worker_exit_requested = true;
            self.worker_stopping = true;
        }
        self.app.on_leave_panel();
    }

    /// Shrink the main window back to the picker, the slot having nothing to
    /// show.
    fn show_picker_window(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        self.screen = Screen::Picker;
        menu_state::lock().model = None;
        desktop::enter_picker_window(frame);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.window_title(false)));
    }

    /// Act on a card click.
    ///
    /// A slot holds one installation, so the only thing a card that is not the
    /// slot's own model can do is take the slot over - which asks first.
    fn choose_model(&mut self, model: Model) {
        if self.viewed_slot != self.instance {
            // Another slot: the deck it holds is launched from its own window,
            // so the only thing a card does here is fill the slot.
            let held = cdj3k_emu_storage::slot_summary(self.viewed_slot).map(|(m, _)| m);
            match held {
                Some(held) if held == model => {
                    cdj3k_emu_platform::menu::open_instance(self.viewed_slot)
                }
                Some(_) => self.setup = Some(SetupStep::Confirm(model)),
                None => self.begin_install(model),
            }
            return;
        }
        if self.host.installed_model() == Some(model) || self.host.any_model_launchable() {
            let already_running = self.last_launched == Some(model)
                && matches!(self.screen, Screen::Panel)
                && !self.worker_stopping;
            self.setup = None;
            if !already_running {
                self.pending_launch = Some(model);
            }
        } else if self.host.installed_model().is_some() {
            self.setup = Some(SetupStep::Confirm(model));
        } else {
            self.begin_install(model);
        }
    }

    /// Open the wizard to take the viewed slot over for `model`. Whatever
    /// runs there keeps running; the install waits for it.
    fn begin_install(&mut self, model: Model) {
        self.open_wizard(model, self.viewed_slot);
    }

    /// Lay the viewed slot's own model down afresh.
    fn begin_reinstall(&mut self, model: Model) {
        self.open_wizard(model, self.viewed_slot);
    }

    fn open_wizard(&mut self, model: Model, slot: u32) {
        self.wizard.open_for(model, slot);
        self.setup = Some(SetupStep::Install);
    }

    /// Swap in a finished install waiting for this window's slot, if nothing
    /// runs it. Returns the model it holds.
    fn apply_staged(&mut self) -> Option<Model> {
        if !cdj3k_emu_runtime::worker_is_finished() {
            return None;
        }
        match self.host.apply_staged() {
            Ok(Some(record)) => {
                eprintln!(
                    "cdj3k-emu: slot {}: the installed {} firmware is swapped in",
                    self.instance, record.model
                );
                if self.viewed_slot == self.instance {
                    self.view_slot(self.instance);
                }
                Some(record.model)
            }
            Ok(None) => None,
            Err(e) => {
                eprintln!(
                    "cdj3k-emu: slot {}: swapping in the installed firmware failed: {e}",
                    self.instance
                );
                None
            }
        }
    }

    /// Where the wizard's finished run went. This window's slot takes it now
    /// unless its emulation runs; another slot takes it now unless another
    /// window has it open, which then picks it up itself.
    fn settle_install(&mut self, slot: u32) -> Installed {
        if slot == self.instance {
            if self.apply_staged().is_some() {
                return Installed::Applied;
            }
            return Installed::Staged;
        }
        self.busy_probe.set(None);
        match cdj3k_emu_storage::SlotClaim::take(slot) {
            Ok(Some(claim)) => {
                match cdj3k_emu_storage::apply_staged(&claim) {
                    Ok(Some(_)) => Installed::Applied,
                    Ok(None) => Installed::Staged,
                    Err(e) => {
                        eprintln!("cdj3k-emu: slot {slot}: swapping in the installed firmware failed: {e}");
                        Installed::Staged
                    }
                }
            }
            Ok(None) => Installed::Staged,
            Err(e) => {
                eprintln!("cdj3k-emu: slot {slot}: claiming it failed: {e}");
                Installed::Staged
            }
        }
    }

    /// Once a second, look for a finished install waiting for this window's
    /// slot - from another window, or from `--provision`. It goes in at once
    /// if nothing runs. A running emulation restarts onto it when Restart was
    /// pressed for it elsewhere, and otherwise takes it at its next start.
    fn check_for_install(&mut self) {
        let now = Instant::now();
        if now < self.next_install_check {
            return;
        }
        self.next_install_check = now + Duration::from_secs(1);
        // This window's own run settles its install when it finishes.
        if self.wizard.target_instance() == self.instance && self.wizard.is_running() {
            return;
        }
        let Some(record) = cdj3k_emu_storage::pending_install(self.instance) else {
            return;
        };
        if self.worker_stopping || self.pending_launch.is_some() {
            return;
        }
        if cdj3k_emu_runtime::worker_is_finished() {
            self.apply_staged();
        } else if record.restart {
            self.reboot_into(record.model);
        }
    }

    /// Restart this window's emulation as `model`, which boots whatever
    /// install is waiting for the slot.
    fn reboot_into(&mut self, model: Model) {
        if !cdj3k_emu_runtime::worker_is_finished() {
            menu_state::lock().shade_forced = true;
            self.stop_emulation();
        }
        self.pending_launch = Some(model);
    }

    /// Empty the slot. The emulation stops first - QEMU holds the eMMC open -
    /// and nothing is installed in its place, so the window falls back to the
    /// picker with a slot that now holds nothing.
    fn begin_delete(&mut self) {
        if self.viewed_slot != self.instance {
            let slot = self.viewed_slot;
            if !self.slot_is_open(slot) {
                if let Err(e) = cdj3k_emu_storage::FirmwarePaths::new(slot).remove() {
                    eprintln!("cdj3k-emu: clearing slot {slot}'s installation failed: {e}");
                }
                self.viewed_release = None;
            }
            self.setup = Some(SetupStep::Pick);
            return;
        }
        if cdj3k_emu_runtime::worker_is_finished() {
            self.empty_own_slot();
            // The main window behind has a dead panel on it; it becomes the
            // picker this window was, rather than a second one beside it.
            self.reveal_picker = true;
        } else {
            self.pending_delete = true;
            self.stop_emulation();
            self.setup = Some(SetupStep::StoppingToDelete);
        }
    }

    /// The slot this window owns goes back to empty. Nothing is left to
    /// manage, so the setup window goes with the installation: the picker the
    /// main window falls back to is the same window in another state, not a
    /// second one over it.
    fn empty_own_slot(&mut self) {
        self.wipe_slot();
        self.viewed_release = None;
        self.setup = None;
        self.wizard.reset();
    }

    fn wipe_slot(&mut self) {
        if let Err(e) = self.host.clear_firmware() {
            eprintln!(
                "cdj3k-emu: clearing slot {}'s installation failed: {e}",
                self.instance
            );
        }
    }

    /// Point the setup window at `slot`.
    fn view_slot(&mut self, slot: u32) {
        self.viewed_slot = slot;
        self.viewed_release = cdj3k_emu_storage::slot_release(slot);
        self.busy_probe.set(None);
    }

    /// Whether another window has `slot` open, or an emulation runs it.
    /// Probed at most once a second.
    fn slot_is_open(&self, slot: u32) -> bool {
        if slot == self.instance {
            return false;
        }
        let now = Instant::now();
        if let Some((probed, at, busy)) = self.busy_probe.get() {
            if probed == slot && now.duration_since(at) < Duration::from_secs(1) {
                return busy;
            }
        }
        let busy = cdj3k_emu_storage::slot_in_use(slot);
        self.busy_probe.set(Some((slot, now, busy)));
        busy
    }

    /// How the picker should present the slot right now.
    fn picker_view<'a>(
        &'a self,
        status: Option<&'a str>,
        confirm_replace: Option<Model>,
        dismissable: bool,
    ) -> PickerView<'a> {
        let foreign = self.viewed_slot != self.instance;
        PickerView {
            slot: self.viewed_slot,
            status,
            installed: if foreign {
                cdj3k_emu_storage::slot_summary(self.viewed_slot).map(|(model, _)| model)
            } else {
                self.host.installed_model()
            },
            running: (!foreign && matches!(self.screen, Screen::Panel))
                .then_some(self.last_launched)
                .flatten(),
            all_launchable: !foreign && self.host.any_model_launchable(),
            confirm_replace,
            dismissable,
            foreign,
            busy: self.slot_is_open(self.viewed_slot),
            release: self.viewed_release.as_deref(),
        }
    }

    /// The setup window: one window for the whole job of giving this slot an
    /// emulation, whichever step it is on.
    fn show_setup_window(&mut self, ctx: &egui::Context) -> Option<PickerAction> {
        let Some(step) = &self.setup else {
            self.setup_pos = None;
            return None;
        };
        // The setup window opens centred on the main one rather than wherever
        // the window manager would have put it.
        if self.setup_pos.is_none() {
            self.setup_pos = ctx
                .input(|i| i.viewport().outer_rect)
                .map(|r| r.center() - egui::Vec2::from(SETUP_WINDOW_SIZE) * 0.5);
        }
        let title = match step {
            SetupStep::Install => "Install Firmware",
            _ => "Manage Emulation",
        };
        let mut action = None;
        let closed = Arc::new(AtomicBool::new(false));
        let closed_inner = closed.clone();
        let mut builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size(SETUP_WINDOW_SIZE)
            .with_resizable(false);
        if let Some(pos) = self.setup_pos {
            builder = builder.with_position(pos);
        }
        let this = &mut *self;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("cdj3k_setup"),
            builder,
            |ctx, _class| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    closed_inner.store(true, Relaxed);
                }
                let slot = this.viewed_slot;
                let going = this.model;
                egui::CentralPanel::default()
                    .frame(egui::Frame::none().fill(theme::palette(ctx).paper))
                    .show(ctx, |ui| match this.setup {
                        Some(SetupStep::Install) => {
                            let model = this.wizard.model;
                            let sub = format!(
                                "Decrypt the {model} .UPD (or take its decrypted .iso), patch the \
                                 kernel and initramfs, and provision an eMMC image."
                            );
                            let body = draw_header(
                                ui,
                                &Header {
                                    slot,
                                    model: Some(model),
                                    release: None,
                                    state: None,
                                    title: "Install firmware",
                                    sub: &sub,
                                    switchable: false,
                                },
                            )
                            .body;
                            // The wizard lays out its own gutters and its own
                            // action bar, so it takes the body whole.
                            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(body), |ui| {
                                this.wizard.draw(ui, &closed_inner);
                            });
                        }
                        Some(SetupStep::ConfirmDelete) => {
                            let view = this.picker_view(None, None, true);
                            action = draw_delete_confirm(ui, &view);
                        }
                        Some(SetupStep::StoppingToDelete) => draw_busy(
                            ui,
                            &Header {
                                slot,
                                model: Some(going),
                                release: this.viewed_release.as_deref(),
                                state: Some("STOPPING"),
                                title: "Emulation",
                                sub: "The emulation has to stop before its installation may be touched.",
                                switchable: false,
                            },
                            &format!("Stopping the {} emulation…", going.title()),
                            "The slot is emptied once it has.",
                        ),
                        Some(SetupStep::Pick) | Some(SetupStep::Confirm(_)) => {
                            let confirm = match this.setup {
                                Some(SetupStep::Confirm(m)) => Some(m),
                                _ => None,
                            };
                            let view = this.picker_view(None, confirm, true);
                            action = draw_picker(ui, &view);
                        }
                        None => {}
                    });
            },
        );
        if closed.load(Relaxed) {
            self.close_setup();
        }
        action
    }

    /// Shut the setup window, unless a provisioning run would be orphaned.
    ///
    /// Whatever is behind it is the running panel or the main window's own
    /// startup picker. So closing always means going back to that rather than
    /// to this window's `Pick` step, which would be the same cards again in a
    /// second window.
    fn close_setup(&mut self) {
        if self.wizard.is_running() || matches!(self.setup, Some(SetupStep::StoppingToDelete)) {
            return;
        }
        self.setup = None;
        self.view_slot(self.instance);
        self.wizard.reset();
    }

    fn apply_picker_action(&mut self, action: Option<PickerAction>) {
        match action {
            Some(PickerAction::View(slot)) => self.view_slot(slot),
            Some(PickerAction::Open(slot)) => cdj3k_emu_platform::menu::open_instance(slot),
            Some(PickerAction::Choose(model)) => self.choose_model(model),
            Some(PickerAction::Reinstall(model)) => self.begin_reinstall(model),
            Some(PickerAction::Replace(model)) => self.begin_install(model),
            Some(PickerAction::CancelReplace) => self.setup = Some(SetupStep::Pick),
            Some(PickerAction::Dismiss) => self.close_setup(),
            Some(PickerAction::Delete) => self.setup = Some(SetupStep::ConfirmDelete),
            Some(PickerAction::CancelDelete) => self.setup = Some(SetupStep::Pick),
            Some(PickerAction::ConfirmDelete) => self.begin_delete(),
            None => {}
        }
    }

    /// The Done banner's button: boot this window's slot onto its new
    /// installation, restarting a running emulation. For another slot, ask
    /// its window to restart onto it, or bring up the window of a slot it
    /// already went into.
    fn on_firmware_provisioned(&mut self, slot: u32, model: Model) {
        if slot == self.instance {
            self.reboot_into(model);
            return;
        }
        match cdj3k_emu_storage::request_restart(slot) {
            Ok(true) => {}
            Ok(false) => cdj3k_emu_platform::menu::open_instance(slot),
            Err(e) => eprintln!("cdj3k-emu: asking slot {slot} to restart failed: {e}"),
        }
    }
}

impl eframe::App for CdjShell {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Graceful close: instead of blocking inside `on_exit` for the full
        // runtime-stop budget (which freezes the window for 1-2 s), defer
        // the actual close until the worker has finished.
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.wizard.is_running() && !self.shutdown_in_progress {
                // An install under way finishes first; its window says so.
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
        if self.shutdown_in_progress {
            if cdj3k_emu_runtime::worker_is_finished() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                ctx.request_repaint();
            }
        }

        MENU_SETUP.call_once(cdj3k_emu_platform::menu::setup_menu);
        let want_manage = std::mem::take(&mut menu_state::lock().manage_emulation_requested);

        if let Some(model) = self.initial_model.take() {
            self.launch(ctx, frame, model);
        }

        if self.worker_stopping && cdj3k_emu_runtime::reap_finished_worker() {
            self.worker_stopping = false;
        }
        // The worker retired itself to restart onto a finished install.
        if cdj3k_emu_runtime::worker_is_finished()
            && std::mem::take(&mut menu_state::lock().relaunch_requested)
        {
            cdj3k_emu_runtime::reap_finished_worker();
            self.pending_launch = Some(self.model);
        }
        if !self.worker_stopping {
            if std::mem::take(&mut self.pending_delete) {
                self.empty_own_slot();
                self.reveal_picker = false;
                self.show_picker_window(ctx, frame);
            }
            if let Some(model) = self.pending_launch.take() {
                self.launch(ctx, frame, model);
            }
        }
        if self.worker_stopping {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // The startup picker is already a choice of emulation; a second one
        // over it would be the same cards twice. A run under way keeps the
        // window, as does a slot being emptied.
        if want_manage
            && matches!(self.screen, Screen::Panel)
            && !self.wizard.is_running()
            && !matches!(self.setup, Some(SetupStep::StoppingToDelete))
        {
            self.view_slot(self.instance);
            self.setup = Some(SetupStep::Pick);
        }

        match self.screen {
            Screen::Picker => {
                desktop::apply_macos_resize_constraints_from_frame(frame, None);
                // The startup picker, for a slot with nothing to show. The
                // setup window handles every later choice.
                let view = self.picker_view(None, None, false);
                let mut action = None;
                egui::CentralPanel::default()
                    .frame(egui::Frame::none().fill(theme::palette(ctx).paper))
                    .show(ctx, |ui| {
                        action = draw_picker(ui, &view);
                    });
                self.apply_picker_action(action);
            }
            Screen::Panel => self.app.update(ctx, frame),
        }

        // One window for switching an emulation and installing its firmware.
        // Exempt from the shade gate: the install step is the tool that
        // provisions firmware, so it must be reachable precisely when QEMU
        // isn't running.
        self.wizard.poll();
        if self.wizard.take_finished() {
            let installed = self.settle_install(self.wizard.target_instance());
            self.wizard.set_installed(installed);
        }
        self.check_for_install();
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

        cdj3k_emu_platform::menu::sync_menu();
        if matches!(self.screen, Screen::Panel) {
            self.script.tick(&mut self.app, ctx, &mut self.shot);
        }
        self.shot.tick(ctx);
    }

    fn on_exit(&mut self, gl: Option<&glow::Context>) {
        self.app.on_exit(gl);
    }
}
