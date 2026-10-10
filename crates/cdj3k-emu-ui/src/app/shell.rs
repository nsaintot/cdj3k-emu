//! Top-level app shell: the model picker, then the running panel.
//!
//! The shell owns the window shape (compact picker vs aspect-locked panel),
//! the native menu, the install-firmware wizard and the graceful-close
//! sequence. Booting is delegated to a [`RuntimeHost`] supplied by the binary
//! (it owns the runtime worker); the panel itself is the persistent
//! [`CdjApp`], which keeps its LCD streams across model switches.
//!
//! A slot emulates one model. Changing which one, and installing the firmware
//! it needs, are steps of one setup window ([`SetupStep`]) that opens over the
//! running panel.
//!
//! An install never touches a running emulation. It is written beside the
//! slot's live installation, and the process that owns the slot swaps it in
//! whenever the slot's emulation is not running: at once when nothing runs,
//! otherwise on its next start - a restart from the menu, the guest
//! rebooting, or Restart on the finished install. When Restart is pressed in
//! another window, that window sends `restart-install` on the slot's window
//! socket ([`cdj3k_emu_platform::window_socket`]), and the owner restarts its
//! own emulation. No window stops another window's emulation.
//!
//! | Module | Holds |
//! |---|---|
//! | `frame` | The [`eframe::App`] impl: what each frame does, in order |
//! | `setup` | The setup window and the step it shows |
//! | `slots` | Installs, deletes and what is known of other slots |
//! | `commands` | Commands to and from other windows on the window sockets |
//! | `mods` | The Mods view's reads, edits and downloads |

mod commands;
mod frame;
mod mods;
mod setup;
mod slots;

use std::cell::Cell;
use std::time::Instant;

use cdj3k_emu_panel::Model;
use cdj3k_emu_platform::window_socket::{Command, PendingReply, WindowSocket};
use cdj3k_emu_platform::{app_meta, desktop, menu_state};

use super::firmware_wizard::FirmwareWizard;
use super::mods_view::{ModsBar, ModsPage};
use super::picker::PickerAction;
use super::screenshot::ScreenshotDriver;
use super::script::ScriptDriver;
use super::updater::Updater;
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
    /// This window listens on this socket for commands from other windows. It
    /// is `None` when this process does not hold the slot's claim or binding
    /// the socket failed.
    pub window_socket: Option<WindowSocket>,
}

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
    /// The slot's mods.
    Mods,
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
    /// Whether `check_for_install` should look in the slot's staging dir for
    /// a finished install. Startup, an `install-ready` command and the end of
    /// this window's emulation set it.
    install_check_due: bool,
    /// Whether the runtime worker had finished on the previous pass of
    /// `logic`. `logic` compares it with the current state to detect that the
    /// emulation stopped.
    worker_was_finished: bool,
    /// The last probe of another slot: slot, when, whether a window holds it
    /// ([`cdj3k_emu_storage::slot_in_use`]) and whether its emulation runs
    /// ([`cdj3k_emu_storage::slot_running`]). The picker asks every frame;
    /// each probe takes locks.
    busy_probe: Cell<Option<(u32, Instant, bool, bool)>>,
    /// When this window asks another window to empty a slot, this field holds
    /// the slot, the time of the request and the picker's status line. The
    /// picker shows the line for ten seconds or until the slot is empty.
    delete_asked: Option<(u32, Instant, &'static str)>,
    /// This holds each command sent to another window whose reply has not
    /// arrived yet, with the slot it was sent to.
    sent: Vec<(u32, Command, PendingReply)>,
    /// The startup picker's Close: this window goes, the others stay.
    close_window: bool,
    /// The picker's side of the window's aspect lock: none.
    resize: desktop::ResizeState,
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
    /// The Mods view, while the setup window shows it.
    mods: Option<ModsPage>,
    /// When the Mods view last read its slot.
    mods_read: Instant,
    /// The Mods bar of [`viewed_slot`](Self::viewed_slot): the slot it was
    /// read for, and when.
    mods_bar: Option<(u32, Instant, ModsBar)>,
    /// The Add mod dialog, while it is up.
    mods_pick: Option<desktop::PendingPick>,
    /// An add whose name is already in the list, waiting on the Replace dialog.
    mods_pending: Option<cdj3k_emu_storage::mods::PendingAdd>,
    /// A mod being downloaded: its URL, and the file once it is in.
    mods_download: Option<(
        String,
        std::sync::mpsc::Receiver<Result<std::path::PathBuf, String>>,
    )>,
    /// Where on screen the setup window opened, taken from the main window so
    /// the two sit exactly on top of each other. Held for as long as the
    /// window is up, so it stays where the user may have dragged it.
    setup_pos: Option<egui::Pos2>,
    /// Whether the setup window has had its size restated since it opened.
    setup_shaped: bool,
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
    /// The update window and the check, download and install behind it.
    updater: Updater,
    /// When the quit was left to Sparkle, which installs the update waiting
    /// for it and ends the process itself.
    quit_handed_off: Option<Instant>,
    /// See [`ShellConfig::window_socket`].
    window_socket: Option<WindowSocket>,
    /// The threads that send commands to other windows use this egui context
    /// to wake this window when a reply arrives.
    ctx: egui::Context,
}

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
            install_check_due: true,
            worker_was_finished: true,
            busy_probe: Cell::new(None),
            delete_asked: None,
            sent: Vec::new(),
            close_window: false,
            resize: Default::default(),
            pending_delete: false,
            viewed_slot: cfg.instance,
            viewed_release: cdj3k_emu_storage::slot_release(cfg.instance),
            setup: None,
            mods: None,
            mods_read: Instant::now(),
            mods_bar: None,
            mods_pick: None,
            mods_pending: None,
            mods_download: None,
            setup_pos: None,
            setup_shaped: false,
            reveal_picker: false,
            worker_stopping: false,
            shutdown_in_progress: false,
            shot: ScreenshotDriver::from_env(),
            script: ScriptDriver::from_env(),
            updater: Updater::new(cfg.instance),
            quit_handed_off: None,
            window_socket: cfg.window_socket,
            ctx: cc.egui_ctx.clone(),
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
        desktop::enter_panel_window(ctx, frame, self.instance, self.model, self.app.ref_canvas());
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
        self.app.on_leave_panel();
        self.screen = Screen::Picker;
        menu_state::lock().model = None;
        desktop::enter_picker_window(ctx, frame);
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

    /// Restart this window's emulation as `model`, which boots whatever
    /// install is waiting for the slot.
    fn reboot_into(&mut self, model: Model) {
        if !cdj3k_emu_runtime::worker_is_finished() {
            menu_state::lock().shade_forced = true;
            self.stop_emulation();
        }
        self.pending_launch = Some(model);
    }

    fn apply_picker_action(&mut self, action: Option<PickerAction>) {
        match action {
            Some(PickerAction::View(slot)) => self.view_slot(slot),
            Some(PickerAction::Open(slot)) => cdj3k_emu_platform::menu::open_instance(slot),
            Some(PickerAction::Choose(model)) => self.choose_model(model),
            Some(PickerAction::Reinstall(model)) => self.begin_reinstall(model),
            Some(PickerAction::Replace(model)) => self.begin_install(model),
            Some(PickerAction::CancelReplace) => self.setup = Some(SetupStep::Pick),
            Some(PickerAction::Dismiss) if self.setup.is_none() => self.close_window = true,
            Some(PickerAction::Dismiss) => self.close_setup(),
            Some(PickerAction::Delete) => self.setup = Some(SetupStep::ConfirmDelete),
            Some(PickerAction::CancelDelete) => self.setup = Some(SetupStep::Pick),
            Some(PickerAction::ConfirmDelete) => self.begin_delete(),
            Some(PickerAction::ManageMods) => self.open_mods(self.viewed_slot),
            Some(PickerAction::Mods(a)) => self.apply_mods_action(a),
            None => {}
        }
    }

    /// The Done banner's button: boot this window's slot onto its new
    /// installation, restarting a running emulation. For another slot, send
    /// `restart-install` to that slot's window, and open or raise that window
    /// if it does not accept the restart or no window holds the slot.
    fn on_firmware_provisioned(&mut self, slot: u32, model: Model) {
        if slot == self.instance {
            self.reboot_into(model);
            return;
        }
        self.send_command(slot, Command::RestartInstall);
    }
}
