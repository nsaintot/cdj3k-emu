//! The firmware install wizard. It decrypts the UPD, extracts the kernel and
//! the initramfs, and provisions the eMMC.
//!
//! The firmware input is either a Pioneer `.UPD` plus its LUKS key file, or an
//! already-decrypted ISO 9660 image (the `.UPD` payload), which skips the
//! decrypt step. The run writes into the slot's
//! [`StagedFirmware`](cdj3k_emu_storage::StagedFirmware) and leaves
//! the slot's live installation alone, running emulation included; the shell
//! swaps it in once the run has finished and the slot's emulation is not
//! running. A slot holds one installation, so the swap replaces whatever
//! model was there.

mod form;
mod provision;
mod widgets;

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};

use egui::{Align, Context, CornerRadius, Frame, Pos2, Rect, RichText, Stroke, TextEdit};

use cdj3k_emu_panel::Model;
use cdj3k_emu_platform::{desktop::PendingPick, menu_state};

use super::picker;
use super::theme;
use provision::{provision, ProvisionStep};
use widgets::{banner, meter, micro_label};

/// The log frame's own margin and border, top and bottom.
const LOG_FRAME_PAD: f32 = 18.0;
/// The gap between the bottom of the log and the action bar.
const LOG_BAR_GAP: f32 = 16.0;
/// The gap between form rows.
const ROW_GAP: f32 = 16.0;
/// The space kept clear at the left of the action bar, where the build version
/// is drawn.
const VERSION_ROOM: f32 = 46.0;

/// The outcome of a finished install, which the shell decides. Only the window
/// that owns the slot swaps the install in, and only while the slot's emulation
/// is not running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installed {
    /// The install is swapped in, and the slot's deck boots from it.
    Applied,
    /// The install waits while the slot's emulation runs; the deck boots from
    /// it after a restart.
    Staged,
}

/// The install step of the setup window: the form, the progress and the log.
/// It does not own a window; the shell hosts it.
pub struct FirmwareWizard {
    /// The model being provisioned: the one chosen on the picker, or the
    /// running one.
    pub model: Model,
    /// A file dialog that is open right now, and which field it fills. The
    /// dialog runs on its own thread so the window keeps drawing while it is
    /// up; `poll` collects the answer.
    pending_pick: Option<(bool, PendingPick)>,
    upd_path: String,
    /// The path last checked for an ISO 9660 image and the result. It is
    /// cached because the form asks on every frame.
    iso_probe: std::cell::RefCell<Option<(String, bool)>>,
    key_path: String,
    /// Provision passwordless root SSH into the guest.
    ssh: bool,
    /// The slot being provisioned, set by [`Self::open_for`].
    target_instance: u32,
    /// The slot this window owns.
    home_instance: u32,
    /// A run has finished since the shell last asked.
    finished: bool,
    /// Where the shell put the finished run: applied or staged.
    installed: Option<Installed>,
    /// The `(slot, model)` that the user asked to boot or open from the Done
    /// banner. The shell takes it.
    boot_request: Option<(u32, Model)>,
    /// The shared log buffer. The provision thread appends lines and the UI
    /// reads it every frame.
    log: Arc<Mutex<String>>,
    /// Set while provisioning runs; `None` while the form shows or after the
    /// run has finished.
    provision_status: Option<Arc<Mutex<ProvisionStep>>>,
    /// The terminal state, kept after the provision thread completes.
    terminal: Option<ProvisionStep>,
}

impl FirmwareWizard {
    pub fn new() -> Self {
        let instance = menu_state::lock().current_instance_id.max(1);
        Self {
            pending_pick: None,
            model: Model::default(),
            upd_path: String::new(),
            iso_probe: std::cell::RefCell::new(None),
            key_path: String::new(),
            ssh: ssh_from_env(),
            target_instance: instance,
            home_instance: instance,
            finished: false,
            installed: None,
            boot_request: None,
            log: Arc::new(Mutex::new(String::new())),
            provision_status: None,
            terminal: None,
        }
    }

    /// Aim the wizard at `model` in slot `instance`. The shell decides when to
    /// show it.
    pub fn open_for(&mut self, model: Model, instance: u32) {
        self.model = model;
        self.target_instance = instance.max(1);
    }

    /// The slot the wizard installs into.
    pub fn target_instance(&self) -> u32 {
        self.target_instance
    }

    /// Whether a provisioning run is under way, so the shell can refuse to
    /// close the window, or quit, out from under it.
    pub fn is_running(&self) -> bool {
        self.provision_status.is_some()
    }

    /// A run has finished successfully since the last call, and waits for
    /// [`Self::set_installed`].
    pub fn take_finished(&mut self) -> bool {
        std::mem::take(&mut self.finished)
    }

    /// Record where the shell put the finished run. It decides what the Done
    /// banner offers.
    pub fn set_installed(&mut self, installed: Installed) {
        self.installed = Some(installed);
    }

    /// Takes the Done banner's boot request, `(slot, model)`. Later calls return
    /// `None` until the user asks again.
    pub fn take_boot_request(&mut self) -> Option<(u32, Model)> {
        self.boot_request.take()
    }

    /// Pick up the provision thread's terminal state. Called once a frame by
    /// the shell, whether or not the step is on screen, so a run that finishes
    /// behind a closed window is still recorded.
    pub fn poll(&mut self) {
        let Some(status) = &self.provision_status else {
            return;
        };
        let step = status.lock().unwrap().clone();
        if step.is_terminal() {
            // Report success without starting the deck; the user starts it
            // from the Done banner.
            self.finished = matches!(step, ProvisionStep::Done);
            self.terminal = Some(step);
            self.provision_status = None;
        }
    }

    pub(super) fn draw(&mut self, ui: &mut egui::Ui, close: &Arc<std::sync::atomic::AtomicBool>) {
        let area = ui.max_rect();
        let k = picker::k_of(area);
        let pal = theme::palette(ui.ctx());
        let running = self.is_running();
        let done = matches!(self.terminal, Some(ProvisionStep::Done));
        let (space, bar) = picker::footer_split(area, k);

        let body = Rect::from_min_max(
            Pos2::new(space.left() + theme::GUTTER * k, space.top()),
            Pos2::new(space.right() - theme::GUTTER * k, space.bottom()),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            theme::apply_setup_style(ui, pal);
            ui.spacing_mut().item_spacing.y = ROW_GAP * k;
            self.draw_status(ui, k, pal);
            if !running && !done {
                self.form(ui, k, pal);
            }
            self.draw_log(ui, k, pal);
        });

        let inner = picker::draw_footer(ui, bar, pal, k);
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            theme::apply_setup_style(ui, pal);
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                self.draw_actions(ui, running, close, k, pal);
            });
        });
    }

    /// Draws the run's state: the meter while it runs and a banner once it
    /// ends.
    fn draw_status(&self, ui: &mut egui::Ui, k: f32, pal: &theme::Palette) {
        match (&self.provision_status, &self.terminal) {
            (Some(status), _) => {
                let step = status.lock().unwrap().clone();
                meter(ui, k, pal, &step);
            }
            (None, Some(ProvisionStep::Done)) => {
                let line = self.done_line();
                banner(ui, k, pal, pal.ok, &line, pal.ok);
            }
            (None, Some(ProvisionStep::Error(msg))) => {
                let msg = msg.clone();
                banner(ui, k, pal, pal.danger, &msg, pal.danger);
            }
            _ => {}
        }
    }

    /// Draws the log in the space left above the action bar. The split has
    /// already taken the bar out of the height.
    fn draw_log(&self, ui: &mut egui::Ui, k: f32, pal: &theme::Palette) {
        micro_label(ui, k, pal.dim, "Log");
        let snapshot = self.log.lock().unwrap().clone();
        // The frame's own margins and the gap to the action bar come out
        // of the budget first, or the log grows over the bar.
        let height = (ui.available_height() - (LOG_FRAME_PAD + LOG_BAR_GAP) * k).max(48.0 * k);
        Frame::default()
            .fill(pal.field)
            .stroke(Stroke::new(theme::HAIRLINE * k, pal.line))
            .corner_radius(CornerRadius::from(theme::ROUND * k))
            .inner_margin(egui::epaint::MarginF32::symmetric(11.0 * k, 9.0 * k))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("provision_log")
                    .max_height(height)
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if snapshot.is_empty() {
                            ui.label(
                                RichText::new("No log entries")
                                    .size(theme::HINT_FONT * k)
                                    .monospace()
                                    .color(pal.faint),
                            );
                        } else {
                            ui.add(
                                TextEdit::multiline(&mut snapshot.as_str())
                                    .desired_width(ui.available_width())
                                    .frame(egui::Frame::NONE)
                                    .font(egui::TextStyle::Monospace)
                                    .text_color(pal.muted),
                            );
                        }
                    });
            });
    }

    /// Draws the action bar's buttons, from right to left, for the run's
    /// current state.
    fn draw_actions(
        &mut self,
        ui: &mut egui::Ui,
        running: bool,
        close: &Arc<std::sync::atomic::AtomicBool>,
        k: f32,
        pal: &theme::Palette,
    ) {
        match (&self.provision_status, &self.terminal) {
            // Offer no action while a run is under way, because the window
            // refuses to close. Keep the left end of the bar clear for the
            // build version.
            _ if running => {
                ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                    ui.add_space(VERSION_ROOM * k);
                });
            }

            (None, Some(ProvisionStep::Done)) => self.done_actions(ui, close, k, pal),

            (None, Some(ProvisionStep::Error(_))) => {
                if theme::secondary(ui, picker::button_size(k), "Close", pal).clicked() {
                    close.store(true, Relaxed);
                }
                if theme::secondary(ui, picker::button_size(k), "Back", pal).clicked() {
                    self.terminal = None;
                }
            }

            _ => self.form_actions(ui, close, k, pal),
        }
    }

    /// Draws the Done banner's actions: one goes to the deck and one leaves it
    /// for later.
    fn done_actions(
        &mut self,
        ui: &mut egui::Ui,
        close: &Arc<std::sync::atomic::AtomicBool>,
        k: f32,
        pal: &theme::Palette,
    ) {
        let (go, dismiss) = self.done_offers();
        if let Some(label) = go {
            if theme::primary(ui, picker::button_size(k), &label, pal).clicked() {
                self.boot_request = Some((self.target_instance, self.model));
                close.store(true, Relaxed);
            }
        }
        if let Some(label) = dismiss {
            if theme::secondary(ui, picker::button_size(k), label, pal).clicked() {
                close.store(true, Relaxed);
            }
        }
    }

    /// The labels of the Done banner's go and dismiss buttons, which depend on
    /// where the install went.
    fn done_offers(&self) -> (Option<String>, Option<&'static str>) {
        let own = self.target_instance == self.home_instance;
        match (self.installed, own) {
            (Some(Installed::Applied), true) => (Some("Start the deck".to_owned()), None),
            (Some(Installed::Applied), false) => (
                Some(format!("Open slot {}", self.target_instance)),
                Some("Close"),
            ),
            (Some(Installed::Staged), true) => (Some("Restart".to_owned()), Some("Later")),
            (Some(Installed::Staged), false) => (Some("Restart".to_owned()), Some("Close")),
            (None, _) => (None, None),
        }
    }

    /// Draws Install, enabled once the form is complete, and Cancel.
    fn form_actions(
        &mut self,
        ui: &mut egui::Ui,
        close: &Arc<std::sync::atomic::AtomicBool>,
        k: f32,
        pal: &theme::Palette,
    ) {
        let can_install =
            !self.upd_path.is_empty() && (self.firmware_is_iso() || !self.key_path.is_empty());
        ui.add_enabled_ui(can_install, |ui| {
            let size = picker::button_size(k);
            let hit = if can_install {
                theme::primary(ui, size, "Install", pal)
            } else {
                theme::primary_off(ui, size, "Install", pal)
            };
            if hit.clicked() {
                self.start_provision(ui.ctx().clone());
            }
        });
        if theme::secondary(ui, picker::button_size(k), "Cancel", pal).clicked() {
            close.store(true, Relaxed);
        }
    }

    /// The Done banner's text, which says where the install went.
    fn done_line(&self) -> String {
        let own = self.target_instance == self.home_instance;
        match (self.installed, own) {
            (Some(Installed::Staged), true) => {
                "Firmware installed. It takes effect after a restart.".to_owned()
            }
            (Some(Installed::Staged), false) => format!(
                "Firmware installed. It takes effect in slot {} after a restart.",
                self.target_instance
            ),
            _ => "Firmware installed.".to_owned(),
        }
    }

    /// Start the run. The slot's emulation, if it runs, is left running.
    fn start_provision(&mut self, ctx: Context) {
        // The file may have changed since the form last looked at it.
        *self.iso_probe.borrow_mut() = None;
        // A decrypted ISO needs no key; a .UPD does.
        let key = if self.firmware_is_iso() {
            None
        } else {
            match cdj3k_emu_firmware::LuksKey::from_file(Path::new(&self.key_path)) {
                Ok(k) => Some(k),
                Err(e) => {
                    *self.log.lock().unwrap() = format!("[key] Could not read the key file: {e}\n");
                    self.terminal = Some(ProvisionStep::Error(format!(
                        "Could not read the key file: {e}"
                    )));
                    return;
                }
            }
        };

        let upd = PathBuf::from(&self.upd_path);
        let target = self.target_instance;
        let model = self.model;
        let ssh = self.ssh;
        let status = Arc::new(Mutex::new(ProvisionStep::Decrypting));
        self.provision_status = Some(status.clone());

        // Clear the log and reuse the same Arc.
        self.log.lock().unwrap().clear();
        let log = self.log.clone();

        std::thread::Builder::new()
            .name("cdj3k-emu-provision".into())
            .spawn(move || provision(upd, key, target, model, ssh, status, log, ctx))
            .expect("could not start the provision thread");
    }

    pub(super) fn reset(&mut self) {
        // A successful install has used up its firmware file, so the next
        // install starts with an empty form. An install that was called off or
        // failed keeps its paths, because the next attempt usually retries the
        // same files.
        if matches!(self.terminal, Some(ProvisionStep::Done)) {
            self.upd_path.clear();
            self.key_path.clear();
        }
        self.provision_status = None;
        self.terminal = None;
        self.finished = false;
        self.installed = None;
        self.log.lock().unwrap().clear();
    }
}

/// `ENABLE_SSH=1` in the app's environment ticks the SSH box, and asks for it
/// on a `--provision` run.
fn ssh_from_env() -> bool {
    std::env::var("ENABLE_SSH").is_ok_and(|v| v == "1")
}

/// The wizard's pipeline without the window, for the `--provision` flag:
/// provision `(instance, model)` from `upd_path` (a `.UPD` with `key_path`,
/// or a decrypted ISO) on a worker thread, echo its log to stderr as it runs
/// and return once it has finished.
pub fn provision_blocking(
    upd_path: PathBuf,
    key_path: Option<&Path>,
    instance: u32,
    model: Model,
) -> Result<(), String> {
    let key = match key_path {
        Some(p) => Some(
            cdj3k_emu_firmware::LuksKey::from_file(p)
                .map_err(|e| format!("could not read the key file {}: {e}", p.display()))?,
        ),
        None => None,
    };
    let status = Arc::new(Mutex::new(ProvisionStep::Decrypting));
    let log = Arc::new(Mutex::new(String::new()));
    let worker = {
        let (status, log) = (status.clone(), log.clone());
        std::thread::Builder::new()
            .name("cdj3k-emu-provision".into())
            .spawn(move || {
                provision(
                    upd_path,
                    key,
                    instance,
                    model,
                    ssh_from_env(),
                    status,
                    log,
                    Context::default(),
                )
            })
            .map_err(|e| e.to_string())?
    };
    let mut printed = 0;
    loop {
        let finished = worker.is_finished();
        {
            let l = log.lock().unwrap();
            if l.len() > printed {
                eprint!("{}", &l[printed..]);
                printed = l.len();
            }
        }
        if finished {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let _ = worker.join();
    let status = status.lock().unwrap();
    match &*status {
        ProvisionStep::Done => Ok(()),
        ProvisionStep::Error(e) => Err(e.clone()),
        other => Err(format!(
            "the run stopped before it finished ({})",
            other.label()
        )),
    }
}
