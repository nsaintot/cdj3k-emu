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

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cdj3k_emu_panel::Model;
use cdj3k_emu_platform::window_socket::{Command, PendingReply, Reply, WindowSocket};
use cdj3k_emu_platform::{app_meta, desktop, menu_state};

use super::firmware_wizard::{FirmwareWizard, Installed};
use super::mods_view::{self, ModsAction, ModsBar, ModsPage, ModsScreen};
use super::picker::{
    draw_busy, draw_delete_confirm, draw_header, draw_picker, Header, PickerAction, PickerView,
};
use super::screenshot::ScreenshotDriver;
use super::script::ScriptDriver;
use super::theme;
use super::updater::{Updater, UpdaterEvent};
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

/// How long a quit left to Sparkle waits for it to end the process.
const QUIT_HAND_OFF_WAIT: Duration = Duration::from_secs(15);

/// The setup window's size on every step: the size of the picker it opens
/// over.
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
    /// When the slot's staging dir is next looked at for a finished install.
    next_install_check: Instant,
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
    /// slot - from another window, or from `--provision`. If no emulation is
    /// running, the install is applied at once. Otherwise the emulation uses it
    /// at its next start.
    fn check_for_install(&mut self) {
        let now = Instant::now();
        if now < self.next_install_check {
            return;
        }
        self.next_install_check = now + Duration::from_secs(1);
        // Skip the check while this window installs into the slot (this window
        // applies that install when it finishes), and while an emulation is
        // stopping, launching or running.
        if self.installing_here()
            || self.worker_stopping
            || self.pending_launch.is_some()
            || !cdj3k_emu_runtime::worker_is_finished()
        {
            return;
        }
        if cdj3k_emu_storage::pending_install(self.instance).is_some() {
            self.apply_staged();
        }
    }

    fn installing_here(&self) -> bool {
        self.wizard.target_instance() == self.instance && self.wizard.is_running()
    }

    /// Answer the commands that other windows sent on the slot's window
    /// socket.
    fn serve_window_socket(&mut self) {
        while let Some((command, responder)) =
            self.window_socket.as_ref().and_then(WindowSocket::next)
        {
            let reply = self.on_command(command);
            eprintln!(
                "cdj3k-emu: slot {}: another window sent {}, replied {}",
                self.instance,
                command.as_str(),
                reply.as_str()
            );
            responder.reply(reply);
        }
    }

    fn on_command(&mut self, command: Command) -> Reply {
        if self.shutdown_in_progress || self.close_window {
            return match command {
                Command::Quit => Reply::Ok,
                _ => Reply::Busy,
            };
        }
        match command {
            // The updater in another window needs this window closed. A
            // firmware install running here must finish first.
            Command::Quit if self.wizard.is_running() => Reply::Busy,
            Command::Quit => {
                self.close_window = true;
                Reply::Ok
            }
            // Emptying the slot resets the wizard, so an install running in
            // this window, into any slot, must finish first.
            Command::Delete if self.wizard.is_running() => Reply::Busy,
            Command::Delete => {
                if !matches!(self.setup, Some(SetupStep::StoppingToDelete)) {
                    self.view_slot(self.instance);
                    self.begin_delete();
                }
                Reply::Ok
            }
            Command::RestartMods => self.restart_for_mods(),
            Command::RestartInstall => self.restart_into_install(),
        }
    }

    /// Restart the emulation so that it boots with the slot's current mods.
    /// While a restart is already pending or under way, only increment
    /// `mods_restart_seq`; the runtime worker restarts QEMU once more if the
    /// request came after that restart read the mods.
    fn restart_for_mods(&mut self) -> Reply {
        if !matches!(self.screen, Screen::Panel) || cdj3k_emu_runtime::worker_is_finished() {
            return Reply::NotRunning;
        }
        if self.worker_stopping {
            return Reply::Busy;
        }
        let mut s = menu_state::lock();
        // If QEMU is not running and no restart is about to start it, QEMU
        // failed to start or was stopped. `qemu_running` turns true as soon as
        // QEMU is spawned.
        if !s.qemu_running && !s.qemu_respawning && !s.restart_requested {
            return Reply::NotRunning;
        }
        s.mods_restart_seq += 1;
        if !s.restart_requested && !s.shade_forced && !s.qemu_respawning {
            s.shade_forced = true;
            s.restart_requested = true;
        }
        Reply::Ok
    }

    /// Restart the running emulation into the finished install waiting for
    /// the slot. If no install is waiting, the emulation already runs the
    /// newest one, because an install is applied only while no emulation runs.
    fn restart_into_install(&mut self) -> Reply {
        if self.pending_launch.is_some() {
            // The pending launch applies the install.
            return Reply::Ok;
        }
        if self.installing_here() || self.worker_stopping {
            return Reply::Busy;
        }
        if cdj3k_emu_runtime::worker_is_finished() {
            return Reply::NotRunning;
        }
        match cdj3k_emu_storage::pending_install_retry(self.instance) {
            Some(record) => {
                self.reboot_into(record.model);
                Reply::Ok
            }
            None if menu_state::lock().qemu_running => Reply::Ok,
            // QEMU failed to start or was stopped.
            None => Reply::NotRunning,
        }
    }

    /// Send `command` to `slot`'s window. [`take_replies`](Self::take_replies)
    /// acts on the reply when it arrives.
    fn send_command(&mut self, slot: u32, command: Command) {
        let ctx = self.ctx.clone();
        let pending = PendingReply::send(slot, command, move || ctx.request_repaint());
        self.sent.push((slot, command, pending));
    }

    /// Act on the replies that arrived for commands sent to other windows.
    fn take_replies(&mut self) {
        for (slot, command, pending) in std::mem::take(&mut self.sent) {
            match pending.take() {
                Some(reply) => self.on_reply(slot, command, reply),
                None => self.sent.push((slot, command, pending)),
            }
        }
    }

    fn on_reply(&mut self, slot: u32, command: Command, reply: std::io::Result<Option<Reply>>) {
        if let Err(e) = &reply {
            eprintln!(
                "cdj3k-emu: sending {} to slot {slot}'s window failed: {e}",
                command.as_str()
            );
        }
        match command {
            Command::Delete => {
                let note = match reply {
                    Ok(Some(Reply::Ok | Reply::NotRunning)) => {
                        "its window is stopping the emulation to empty the slot"
                    }
                    Ok(Some(Reply::Busy)) => "its window is busy; try again in a moment",
                    Ok(None) => "its window is not listening; close it, then delete the slot",
                    Err(_) => "asking its window to empty the slot failed",
                };
                // Update the status line only if this slot's request is still
                // the latest one.
                if let Some((asked, at, _)) = self.delete_asked {
                    if asked == slot {
                        self.delete_asked = Some((slot, at, note));
                    }
                }
            }
            Command::RestartMods => {
                let note = match reply {
                    Ok(Some(Reply::Ok)) => None,
                    Ok(Some(Reply::Busy)) => {
                        Some(format!("Slot {slot} is busy; try again in a moment"))
                    }
                    Ok(Some(Reply::NotRunning) | None) => {
                        Some(format!("Slot {slot} is not running"))
                    }
                    Err(e) => Some(format!("Restarting slot {slot} failed: {e}")),
                };
                if self.mods.as_ref().is_some_and(|p| p.slot == slot) {
                    self.note_mods(note);
                }
            }
            // If the slot's window did not accept the restart, or no window
            // holds the slot, open or raise that window. The install is
            // applied when the slot's emulation next starts.
            Command::RestartInstall => {
                if !matches!(reply, Ok(Some(Reply::Ok))) {
                    cdj3k_emu_platform::menu::open_instance(slot);
                }
            }
            Command::Quit => {}
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
            if self.slot_is_open(slot) {
                // The other window has the eMMC open, so it must stop its
                // emulation and empty the slot itself.
                self.delete_asked =
                    Some((slot, Instant::now(), "asking its window to empty the slot"));
                self.send_command(slot, Command::Delete);
            } else {
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
            // picker.
            self.reveal_picker = true;
        } else {
            self.pending_delete = true;
            self.stop_emulation();
            self.setup = Some(SetupStep::StoppingToDelete);
        }
    }

    /// The slot this window owns goes back to empty, and the setup window
    /// closes: the main window falls back to the picker.
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
        self.probe_slot(slot).0
    }

    /// Whether another window's emulation is running `slot`, not merely
    /// holding it open.
    fn slot_runs_elsewhere(&self, slot: u32) -> bool {
        self.probe_slot(slot).1
    }

    fn probe_slot(&self, slot: u32) -> (bool, bool) {
        if slot == self.instance {
            return (false, false);
        }
        let now = Instant::now();
        if let Some((probed, at, busy, running)) = self.busy_probe.get() {
            if probed == slot && now.duration_since(at) < Duration::from_secs(1) {
                return (busy, running);
            }
        }
        let busy = cdj3k_emu_storage::slot_in_use(slot);
        let running = busy && cdj3k_emu_storage::slot_running(slot);
        self.busy_probe.set(Some((slot, now, busy, running)));
        (busy, running)
    }

    /// Whether no slot holds an installation. Only then is there nothing to
    /// choose between and nothing to go back to, so the picker asks for an
    /// install and offers no way out of it.
    fn every_slot_empty(&self) -> bool {
        self.host.installed_model().is_none()
            && (1..=menu_state::MAX_INSTANCES)
                .filter(|&slot| slot != self.instance)
                .all(|slot| cdj3k_emu_storage::slot_summary(slot).is_none())
    }

    /// How the picker should present the slot right now.
    fn picker_view<'a>(
        &'a self,
        status: Option<&'a str>,
        confirm_replace: Option<Model>,
        dismissable: bool,
    ) -> PickerView<'a> {
        let foreign = self.viewed_slot != self.instance;
        let installed = if foreign {
            cdj3k_emu_storage::slot_summary(self.viewed_slot).map(|(model, _)| model)
        } else {
            self.host.installed_model()
        };
        let asked = self
            .delete_asked
            .filter(|&(slot, at, _)| {
                slot == self.viewed_slot
                    && installed.is_some()
                    && at.elapsed() < Duration::from_secs(10)
            })
            .map(|(_, _, note)| note);
        PickerView {
            slot: self.viewed_slot,
            status: status.or(asked),
            installed,
            running: (!foreign && matches!(self.screen, Screen::Panel))
                .then_some(self.last_launched)
                .flatten(),
            all_launchable: !foreign && self.host.any_model_launchable(),
            confirm_replace,
            dismissable,
            foreign,
            busy: self.slot_is_open(self.viewed_slot),
            running_elsewhere: self.slot_runs_elsewhere(self.viewed_slot),
            every_slot_empty: self.every_slot_empty(),
            release: self.viewed_release.as_deref(),
            outdated: cdj3k_emu_storage::slot_outdated(self.viewed_slot),
            mods: self
                .mods_bar
                .as_ref()
                .filter(|(slot, _, _)| *slot == self.viewed_slot)
                .map(|(_, _, bar)| bar),
            replace_incompatible: confirm_replace
                .filter(|&m| installed != Some(m))
                .map(|m| mods_view::incompatible_with(self.viewed_slot, m))
                .unwrap_or_default(),
        }
    }

    /// The setup window: one window for the whole job of giving this slot an
    /// emulation, whichever step it is on.
    fn show_setup_window(&mut self, ctx: &egui::Context) -> Option<PickerAction> {
        if self.setup.is_none() {
            self.setup_pos = None;
            self.setup_shaped = false;
            return None;
        }
        let closed = Arc::new(AtomicBool::new(false));
        // Over the startup picker, a second window of the same size and chrome
        // is only used where the host can centre it on the first.
        let over_the_picker = matches!(self.screen, Screen::Picker);
        let action = if over_the_picker && !cdj3k_emu_platform::desktop::PLACES_WINDOWS {
            self.setup_in_main_window(ctx, &closed)
        } else {
            self.setup_in_own_window(ctx, &closed)
        };
        if closed.load(Relaxed) {
            self.close_setup();
        }
        action
    }

    /// The setup screen as a second OS window, centred on the first where the
    /// host allows it. Wayland ignores the position; the compositor places it.
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
        // Not built non-resizable: winit pins min and max to the size at
        // creation, before it knows the scale, and a compositor places the
        // window by that pin. The size is pinned from the first frame instead.
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
                // Min and max equal hold the size; restated whenever the
                // window measures wrong.
                let size = egui::Vec2::from(SETUP_WINDOW_SIZE);
                if first_frame || (root.content_rect().size() - size).length() > 1.0 {
                    root.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(size));
                    root.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(size));
                    root.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                }
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(theme::palette(root).paper))
                    .show(root, |ui| {
                        action = draw_setup(this, ui, &closed_inner);
                    });
            },
        );
        action
    }

    /// The setup screen inside the window it came from, over whatever it
    /// interrupted.
    fn setup_in_main_window(
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
                // It owns the window while it is up, so nothing behind it can
                // be clicked through. Registered before the setup's widgets:
                // the last one registered over a point takes its clicks.
                ui.interact(screen, ui.id().with("block"), egui::Sense::click_and_drag());
                ui.scope_builder(egui::UiBuilder::new().max_rect(screen), |ui| {
                    ui.set_clip_rect(screen);
                    action = draw_setup(self, ui, closed);
                });
            });
        action
    }

    /// Shut the setup window, unless a provisioning run would be orphaned.
    /// Closing returns to what is behind it: the running panel or the main
    /// window's startup picker.
    fn close_setup(&mut self) {
        if self.wizard.is_running() || matches!(self.setup, Some(SetupStep::StoppingToDelete)) {
            return;
        }
        self.setup = None;
        self.leave_mods();
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

/// How often the Mods view and the Mods bar read their slot again.
const MODS_READ_EVERY: Duration = Duration::from_secs(2);
/// The largest mod archive Add from URL takes.
const MOD_DOWNLOAD_LIMIT: u64 = 512 << 20;

impl CdjShell {
    /// Show `slot`'s mods, over Manage Emulation.
    fn open_mods(&mut self, slot: u32) {
        self.leave_mods();
        let mut page = ModsPage::new(slot);
        page.refresh(
            self.instance,
            self.emulation_running(),
            self.slot_runs_elsewhere(slot),
        );
        self.mods = Some(page);
        self.mods_read = Instant::now();
        self.setup = Some(SetupStep::Mods);
    }

    fn emulation_running(&self) -> bool {
        matches!(self.screen, Screen::Panel) && menu_state::lock().qemu_running
    }

    fn refresh_mods_bar(&mut self) {
        let stale = self.mods_bar.as_ref().is_none_or(|(slot, at, _)| {
            *slot != self.viewed_slot || at.elapsed() >= MODS_READ_EVERY
        });
        if stale && self.setup.is_some() {
            self.mods_bar = Some((
                self.viewed_slot,
                Instant::now(),
                mods_view::mods_bar(self.viewed_slot),
            ));
        }
    }

    /// Close the Mods view. A pending replace is discarded, and a file pick or
    /// download still in progress is dropped.
    fn leave_mods(&mut self) {
        self.discard_pending_mod();
        self.mods = None;
        self.mods_pick = None;
        self.mods_download = None;
    }

    /// Re-read the slot when due, and collect a finished file pick or
    /// download.
    fn poll_mods(&mut self) {
        if let Some(answer) = self.mods_pick.as_ref().and_then(|p| p.take()) {
            self.mods_pick = None;
            if let Some(path) = answer {
                self.add_mod_path(&path, None);
            }
        }
        let landed = self
            .mods_download
            .as_ref()
            .and_then(|(url, rx)| rx.try_recv().ok().map(|r| (url.clone(), r)));
        if let Some((url, result)) = landed {
            self.mods_download = None;
            match result {
                Ok(file) => {
                    // The add unpacks the archive, so the download can go.
                    self.add_mod_path(&file, Some(&url));
                    if let Some(dir) = file.parent() {
                        let _ = std::fs::remove_dir_all(dir);
                    }
                }
                Err(e) => self.note_mods(Some(e)),
            }
        }
        if self.mods_read.elapsed() >= MODS_READ_EVERY {
            self.reread_mods();
        }
    }

    fn reread_mods(&mut self) {
        let running = self.emulation_running();
        let elsewhere = self
            .mods
            .as_ref()
            .is_some_and(|page| self.slot_runs_elsewhere(page.slot));
        if let Some(page) = self.mods.as_mut() {
            page.refresh(self.instance, running, elsewhere);
        }
        self.mods_read = Instant::now();
        self.mods_bar = None;
    }

    /// Show `error` on the Mods view's toolbar, or clear what it showed.
    fn note_mods(&mut self, error: Option<String>) {
        if let Some(page) = self.mods.as_mut() {
            page.note = error.map(|e| (e, true));
        }
        self.reread_mods();
    }

    /// Add a picked, dropped or downloaded path: an archive is installed, a
    /// folder (or its `mod.toml`) is used in place. A mod whose name is in the
    /// list already waits for the Replace dialog.
    fn add_mod_path(&mut self, path: &std::path::Path, origin: Option<&str>) {
        let Some(slot) = self.mods.as_ref().map(|p| p.slot) else {
            return;
        };
        let mut list = cdj3k_emu_storage::mods::SlotMods::load(slot);
        let result = list
            .prepare(path, origin)
            .and_then(|add| match &add.replaces {
                Some(old) => {
                    if let Some(page) = self.mods.as_mut() {
                        page.replace = Some((
                            add.manifest.name.clone(),
                            old.clone(),
                            add.manifest.version.clone(),
                        ));
                    }
                    self.mods_pending = Some(add);
                    Ok(())
                }
                None => list.commit(add).map(|_| ()),
            });
        self.note_mods(result.err().map(|e| e.to_string()));
    }

    /// Drop a same-name add the Replace dialog was asking about.
    fn discard_pending_mod(&mut self) {
        self.mods_pending = None;
        if let Some(page) = self.mods.as_mut() {
            page.replace = None;
        }
    }

    fn apply_mods_action(&mut self, action: ModsAction) {
        let Some(slot) = self.mods.as_ref().map(|p| p.slot) else {
            return;
        };
        let list = || cdj3k_emu_storage::mods::SlotMods::load(slot);
        let result: std::io::Result<()> = match action {
            ModsAction::Back => {
                self.leave_mods();
                self.setup = Some(SetupStep::Pick);
                return;
            }
            ModsAction::Close => {
                self.close_setup();
                return;
            }
            ModsAction::Restart => {
                let slot = self.mods.as_ref().map_or(self.instance, |p| p.slot);
                if slot == self.instance {
                    let note = match self.restart_for_mods() {
                        Reply::Ok => None,
                        Reply::Busy => Some("The emulation is busy; try again in a moment"),
                        Reply::NotRunning => Some("The emulation is not running"),
                    };
                    self.note_mods(note.map(str::to_string));
                    return;
                }
                if let Some(page) = self.mods.as_mut() {
                    page.note = Some((format!("Asking slot {slot} to restart"), false));
                }
                self.send_command(slot, Command::RestartMods);
                return;
            }
            ModsAction::Start => {
                if let Some(page) = &self.mods {
                    cdj3k_emu_platform::menu::open_instance(page.slot);
                }
                return;
            }
            ModsAction::Toggle(name) => {
                let mut l = list();
                let on = l.get(&name).is_some_and(|m| m.enabled);
                l.set_enabled(&name, !on)
            }
            ModsAction::Reorder(from, to) => list().reorder(from, to),
            ModsAction::AskRemove(name) => {
                if let Some(page) = self.mods.as_mut() {
                    page.remove = Some(name);
                }
                return;
            }
            ModsAction::CancelRemove => {
                if let Some(page) = self.mods.as_mut() {
                    page.remove = None;
                }
                return;
            }
            ModsAction::Remove(name) | ModsAction::Eject(name) => {
                if let Some(page) = self.mods.as_mut() {
                    page.remove = None;
                }
                list().remove(&name)
            }
            ModsAction::Add => {
                if self.mods_pick.is_none() {
                    self.mods_pick = Some(desktop::PendingPick::open_mod(
                        "Choose a mod: a .tgz archive, or a mod folder",
                    ));
                }
                return;
            }
            ModsAction::Replace => {
                if let Some(page) = self.mods.as_mut() {
                    page.replace = None;
                }
                match self.mods_pending.take() {
                    Some(add) => list().commit(add).map(|_| ()),
                    None => Ok(()),
                }
            }
            ModsAction::KeepOld => {
                self.discard_pending_mod();
                return;
            }
            ModsAction::AskUrl => {
                if let Some(page) = self.mods.as_mut() {
                    page.url = Some(String::new());
                }
                return;
            }
            ModsAction::CancelUrl => {
                if let Some(page) = self.mods.as_mut() {
                    page.url = None;
                }
                return;
            }
            ModsAction::AddUrl(url) => {
                if self.mods_download.is_none() {
                    self.download_mod(slot, url);
                }
                return;
            }
            ModsAction::OpenLog(name) => {
                if let Some(page) = self.mods.as_mut() {
                    page.open_log(&name);
                }
                return;
            }
            ModsAction::CloseLog => {
                if let Some(page) = self.mods.as_mut() {
                    page.screen = ModsScreen::List;
                    page.log = None;
                }
                return;
            }
            ModsAction::Open(url) => {
                desktop::open_url(&url);
                return;
            }
            ModsAction::Dropped(paths) => {
                for path in paths {
                    self.add_mod_path(&path, None);
                }
                return;
            }
        };
        self.note_mods(result.err().map(|e| e.to_string()));
    }

    /// Fetch `url` on a thread of its own; [`Self::poll_mods`] installs it.
    fn download_mod(&mut self, slot: u32, url: String) {
        if let Some(page) = self.mods.as_mut() {
            page.url = None;
            page.note = Some((format!("Downloading {url}…"), false));
        }
        let name = url
            .rsplit('/')
            .next()
            .map(|n| n.split(['?', '#']).next().unwrap_or(n))
            .filter(|n| !n.is_empty() && !n.contains(['\\', ':']) && *n != "..")
            .unwrap_or("mod.tgz")
            .to_string();
        static DOWNLOADS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = DOWNLOADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = cdj3k_emu_storage::mods::mods_dir(slot)
            .join(format!(".download-{}-{n}", std::process::id()));
        let (tx, rx) = std::sync::mpsc::channel();
        let fetch_url = url.clone();
        let spawned = std::thread::Builder::new()
            .name("cdj3k-emu-mod-download".into())
            .spawn(move || {
                let result = std::fs::create_dir_all(&dir)
                    .map_err(|e| e.to_string())
                    .and_then(|()| {
                        let dest = dir.join(&name);
                        cdj3k_emu_update::fetch_file(&fetch_url, &dest, MOD_DOWNLOAD_LIMIT)
                            .map(|()| dest)
                            .map_err(|e| e.to_string())
                    });
                // Delete the download if it failed or nobody waits for it.
                let failed = result.is_err();
                if tx.send(result).is_err() || failed {
                    let _ = std::fs::remove_dir_all(&dir);
                }
            });
        match spawned {
            Ok(_) => self.mods_download = Some((url, rx)),
            Err(e) => self.note_mods(Some(e.to_string())),
        }
    }
}

impl eframe::App for CdjShell {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if matches!(self.screen, Screen::Panel) {
            self.app.tilt_input_hook(raw_input);
        }
    }

    /// eframe calls this before every [`ui`](Self::ui), and also while the
    /// window is minimized or covered, when it does not call `ui`. It closes
    /// the window, relaunches the emulation, sends the requested power-off
    /// frame, serves the window socket and applies finished installs, so these
    /// continue while the window is hidden.
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
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
                // A prepared update installs now. Sparkle ends the process
                // itself; past QUIT_HAND_OFF_WAIT the window closes anyway.
                if self.quit_handed_off.is_none() && self.updater.apply_on_quit() {
                    self.quit_handed_off = Some(Instant::now());
                }
                match self.quit_handed_off {
                    Some(at) if at.elapsed() < QUIT_HAND_OFF_WAIT => ctx.request_repaint(),
                    _ => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                }
            } else {
                ctx.request_repaint();
            }
        }

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
        if self.worker_stopping {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        self.app.send_requested_power_off();
        // Settle a finished install before the socket and the install check
        // look at the wizard.
        self.wizard.poll();
        if self.wizard.take_finished() {
            let installed = self.settle_install(self.wizard.target_instance());
            self.wizard.set_installed(installed);
        }
        self.serve_window_socket();
        self.take_replies();
        self.check_for_install();
        // Run `logic` again within a second so that `check_for_install` runs,
        // even while the window is hidden.
        ctx.request_repaint_after(Duration::from_secs(1));
        if std::mem::take(&mut self.close_window) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();
        MENU_SETUP.call_once(cdj3k_emu_platform::menu::setup_menu);
        // Hosts with no native menu bar draw one here, above the deck; a
        // no-op on macOS. The startup picker carries its own wordmark and slot
        // switcher, so it gets only the shortcuts.
        if matches!(self.screen, Screen::Panel) {
            cdj3k_emu_platform::menu::draw_in_window(ui);
        } else {
            cdj3k_emu_platform::menu::shortcuts_only(ctx);
        }
        let want_manage = std::mem::take(&mut menu_state::lock().manage_emulation_requested);
        // With no runtime worker to save it, Enable Mods is saved here.
        if cdj3k_emu_runtime::worker_is_finished() {
            let toggled = {
                let mut s = menu_state::lock();
                std::mem::take(&mut s.mods_toggle_requested).then_some(s.mods_enabled)
            };
            if let Some(on) = toggled {
                if let Err(e) = cdj3k_emu_storage::InstanceSettings::update(self.instance, |s| {
                    s.mods_enabled = on
                }) {
                    eprintln!("cdj3k-emu: saving Enable Mods failed: {e}");
                }
            }
        }

        // Not over the startup picker, which already shows the same cards. A
        // run under way keeps the window, as does a slot being emptied.
        if want_manage
            && matches!(self.screen, Screen::Panel)
            && !self.wizard.is_running()
            && !matches!(self.setup, Some(SetupStep::StoppingToDelete))
        {
            self.view_slot(self.instance);
            self.setup = Some(SetupStep::Pick);
        }
        // Any way out of the Mods view drops its pick, download and pending
        // replace, so none of them lands in another slot.
        if !matches!(self.setup, Some(SetupStep::Mods))
            && (self.mods.is_some()
                || self.mods_pick.is_some()
                || self.mods_download.is_some()
                || self.mods_pending.is_some())
        {
            self.leave_mods();
        }
        self.refresh_mods_bar();

        match self.screen {
            Screen::Picker => {
                desktop::apply_resize_constraints(ctx, frame, &mut self.resize, None);
                // The startup picker, for a slot with nothing to show. The
                // setup window handles every later choice.
                let view = self.picker_view(None, None, false);
                let mut action = None;
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(theme::palette(ctx).paper))
                    .show(ui, |ui| {
                        action = draw_picker(ui, &view);
                    });
                self.apply_picker_action(action);
            }
            Screen::Panel => self.app.update(ctx, ui, frame),
        }

        // One window for switching an emulation and installing its firmware.
        // Exempt from the shade gate: the install step must be reachable while
        // QEMU is not running.
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

/// One step of the setup screen, wherever it is being drawn.
fn draw_setup(
    this: &mut CdjShell,
    ui: &mut egui::Ui,
    closed: &Arc<AtomicBool>,
) -> Option<PickerAction> {
    let slot = this.viewed_slot;
    let going = this.model;
    let mut action = None;
    match this.setup {
        Some(SetupStep::Install) => {
            let model = this.wizard.model;
            let sub = format!(
                "Decrypt the {model} .UPD (or take its decrypted .iso), patch the \
                 kernel and initramfs, and provision an eMMC image."
            );
            let body = draw_header(
                ui,
                &Header {
                    slot: Some(slot),
                    model: Some(model),
                    release: None,
                    state: None,
                    title: "Install firmware",
                    sub: &sub,
                    switchable: false,
                    warn: false,
                },
            )
            .body;
            // The wizard lays out its own gutters and its own action bar, so
            // it takes the body whole.
            ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
                this.wizard.draw(ui, closed);
            });
        }
        Some(SetupStep::ConfirmDelete) => {
            let view = this.picker_view(None, None, true);
            action = draw_delete_confirm(ui, &view);
        }
        Some(SetupStep::Mods) => {
            this.poll_mods();
            if let Some(page) = this.mods.as_mut() {
                action = mods_view::draw_mods(ui, page).map(PickerAction::Mods);
            }
            // Repaint regularly so the list follows the slot's files and the
            // boot report, and a finished pick or download is collected.
            ui.ctx().request_repaint_after(Duration::from_millis(500));
        }
        Some(SetupStep::StoppingToDelete) => draw_busy(
            ui,
            &Header {
                slot: Some(slot),
                model: Some(going),
                release: this.viewed_release.as_deref(),
                state: Some("STOPPING"),
                title: "Emulation",
                sub: "The emulation has to stop before its installation may be touched.",
                switchable: false,
                warn: false,
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
    }
    action
}
