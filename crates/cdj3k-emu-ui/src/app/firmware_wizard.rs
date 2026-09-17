//! Firmware install wizard: decrypt UPD → extract kernel + initramfs → provision eMMC.
//!
//! The firmware input is either a Pioneer `.UPD` plus its LUKS key file, or an
//! already-decrypted ISO 9660 image (the `.UPD` payload), which skips the
//! decrypt step. Output goes to the slot's [`FirmwarePaths`]; a slot holds one
//! installation, so installing replaces whatever model was there.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};

use egui::{
    Align, Color32, ComboBox, Context, FontId, Frame, Margin, Pos2, Rect, RichText, Rounding,
    Sense, Stroke, TextEdit, Vec2,
};

use cdj3k_emu_panel::Model;
use cdj3k_emu_platform::{desktop::open_file_picker, menu_state};
use cdj3k_emu_storage::FirmwarePaths;

use super::picker;
use super::theme;

/// Horizontal padding inside a text field, counted against the row's width so
/// the Browse button beside it still fits.
const FIELD_MARGIN: f32 = 11.0;
/// The log frame's own margin and border, top and bottom.
const LOG_FRAME_PAD: f32 = 18.0;
/// Between the foot of the log and the action bar under it.
const LOG_BAR_GAP: f32 = 16.0;
/// The label column every form row hangs its field off.
const LABEL_W: f32 = 116.0;
/// Between form rows.
const ROW_GAP: f32 = 16.0;
const BROWSE_W: f32 = 90.0;
const SLOT_W: f32 = 132.0;
/// Kept clear at the left of the action bar, where the build is drawn.
const VERSION_ROOM: f32 = 46.0;

/// The stage meter: the height of a segment, and the gap to the next.
const METER_SEG_H: f32 = 3.0;
const METER_GAP: f32 = 6.0;
/// The name of the stage under way, over the meter.
const METER_TITLE_FONT: f32 = 15.0;
/// The square of state before a banner's line.
const MARK: f32 = 7.0;

/// ISO 9660: "CD001" at byte 1 of the primary volume descriptor (sector 16).
fn is_iso9660(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 5];
    f.seek(SeekFrom::Start(16 * 2048 + 1)).is_ok()
        && f.read_exact(&mut magic).is_ok()
        && &magic == b"CD001"
}

// ── Provision step ────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub enum ProvisionStep {
    Decrypting,
    ExtractingKernel,
    PatchingInitramfs,
    CreatingEmmc,
    Done,
    Error(String),
}

/// The four stages a run goes through, in the order the meter draws them.
const STAGES: [&str; 4] = [
    "Decrypt",
    "Extract kernel",
    "Patch initramfs",
    "Create eMMC image",
];

impl ProvisionStep {
    fn label(&self) -> &str {
        match self {
            Self::Decrypting => "Decrypting firmware…",
            Self::ExtractingKernel => "Extracting kernel…",
            Self::PatchingInitramfs => "Patching initramfs…",
            Self::CreatingEmmc => "Creating eMMC image…",
            Self::Done => "Done",
            Self::Error(_) => "Error",
        }
    }

    /// Which of [`STAGES`] the run is on, if it is still on one.
    fn stage(&self) -> Option<usize> {
        match self {
            Self::Decrypting => Some(0),
            Self::ExtractingKernel => Some(1),
            Self::PatchingInitramfs => Some(2),
            Self::CreatingEmmc => Some(3),
            Self::Done | Self::Error(_) => None,
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Error(_))
    }
}

// ── Wizard ────────────────────────────────────────────────────────────────────

/// The install step of the setup window: the form, the progress and the log.
/// It does not own a window - the shell hosts it, so switching an emulation
/// and installing its firmware happen in one place.
pub struct FirmwareWizard {
    /// The model being provisioned (chosen on the picker / the running one).
    pub model: Model,
    upd_path: String,
    key_path: String,
    /// Target slot for provisioning (1..=MAX_INSTANCES). Defaults to the
    /// current window's instance so the obvious thing happens by default.
    target_instance: u32,
    /// `(slot, model)` the user asked to (re)boot from the Done banner;
    /// taken by the shell.
    boot_request: Option<(u32, Model)>,
    /// Shared log buffer; provision thread appends lines, UI reads every frame.
    log: Arc<Mutex<String>>,
    /// Set while provisioning is running; None = form or finished.
    provision_status: Option<Arc<Mutex<ProvisionStep>>>,
    /// Cached terminal state after thread completes.
    terminal: Option<ProvisionStep>,
}

impl FirmwareWizard {
    pub fn new() -> Self {
        Self {
            model: Model::default(),
            upd_path: String::new(),
            key_path: String::new(),
            target_instance: menu_state::lock().current_instance_id.max(1),
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

    /// Whether a provisioning run is under way, so the shell can refuse to
    /// close the window out from under it.
    pub fn is_running(&self) -> bool {
        self.provision_status.is_some()
    }

    /// The Done banner's boot request (`(slot, model)`), once.
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
            // Success is reported, not acted on: the deck starts when the user
            // says so, not behind a window they are still reading.
            self.terminal = Some(step);
            self.provision_status = None;
        }
    }

    pub(super) fn draw(&mut self, ui: &mut egui::Ui, close: &Arc<std::sync::atomic::AtomicBool>) {
        let area = ui.max_rect();
        let k = picker::k_of(area);
        let pal = theme::palette(ui.ctx());
        let running = self.provision_status.is_some();
        let done = matches!(self.terminal, Some(ProvisionStep::Done));
        let (space, bar) = picker::footer_split(area, k);

        let body = Rect::from_min_max(
            Pos2::new(space.left() + theme::GUTTER * k, space.top()),
            Pos2::new(space.right() - theme::GUTTER * k, space.bottom()),
        );
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(body), |ui| {
            theme::apply_setup_style(ui, pal);
            ui.spacing_mut().item_spacing.y = ROW_GAP * k;

            // ── Where the run stands ──────────────────────────────────────
            match (&self.provision_status, &self.terminal) {
                (Some(status), _) => {
                    let step = status.lock().unwrap().clone();
                    meter(ui, k, pal, &step);
                }
                (None, Some(ProvisionStep::Done)) => {
                    banner(ui, k, pal, pal.ok, "Firmware installed", pal.ok);
                }
                (None, Some(ProvisionStep::Error(msg))) => {
                    let msg = msg.clone();
                    banner(ui, k, pal, pal.danger, &msg, pal.danger);
                }
                _ => {}
            }

            if !running && !done {
                self.form(ui, k, pal);
            }

            // ── Log ───────────────────────────────────────────────────────
            // Whatever is left above the action bar, which the split already
            // took out of the height.
            micro_label(ui, k, pal.dim, "Log");
            let snapshot = self.log.lock().unwrap().clone();
            // The frame's own margins and the gap to the action bar come out
            // of the budget first, or the log grows over the bar.
            let height = (ui.available_height() - (LOG_FRAME_PAD + LOG_BAR_GAP) * k).max(48.0 * k);
            Frame::default()
                .fill(pal.field)
                .stroke(Stroke::new(theme::HAIRLINE * k, pal.line))
                .rounding(Rounding::same(theme::ROUND * k))
                .inner_margin(Margin::symmetric(11.0 * k, 9.0 * k))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("provision_log")
                        .max_height(height)
                        .stick_to_bottom(true)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if snapshot.is_empty() {
                                ui.label(
                                    RichText::new(
                                        "Nothing yet. The install writes here as it runs.",
                                    )
                                    .size(theme::HINT_FONT * k)
                                    .monospace()
                                    .color(pal.faint),
                                );
                            } else {
                                ui.add(
                                    TextEdit::multiline(&mut snapshot.as_str())
                                        .desired_width(ui.available_width())
                                        .frame(false)
                                        .font(egui::TextStyle::Monospace)
                                        .text_color(pal.muted),
                                );
                            }
                        });
                });
        });

        // ── Action bar ────────────────────────────────────────────────────
        let inner = picker::draw_footer(ui, bar, pal, k);
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
            theme::apply_setup_style(ui, pal);
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                match (&self.provision_status, &self.terminal) {
                    // Nothing is offered while a run is under way: the window
                    // refuses to close, and says so at the bar's quiet end.
                    (Some(_), _) => {
                        ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                            ui.add_space(VERSION_ROOM * k);
                        });
                    }

                    (None, Some(ProvisionStep::Done)) => {
                        if theme::primary(ui, picker::button_size(k), "Start the deck", pal)
                            .clicked()
                        {
                            self.boot_request = Some((self.target_instance, self.model));
                            close.store(true, Relaxed);
                        }
                    }

                    (None, Some(ProvisionStep::Error(_))) => {
                        if theme::secondary(ui, picker::button_size(k), "Close", pal).clicked() {
                            close.store(true, Relaxed);
                        }
                        if theme::secondary(ui, picker::button_size(k), "Back", pal).clicked() {
                            self.terminal = None;
                        }
                    }

                    _ => {
                        let can_install = !self.upd_path.is_empty()
                            && (self.firmware_is_iso() || !self.key_path.is_empty());
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
                }
            });
        });
    }

    /// The form: one row per thing the install needs, each with its label in
    /// its own column so the fields line up down the step.
    fn form(&mut self, ui: &mut egui::Ui, k: f32, pal: &theme::Palette) {
        ui.horizontal_top(|ui| {
            label_column(ui, k, pal.dim, "Install to slot");
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let slots = ComboBox::from_id_salt("wizard_slot")
                        .width(SLOT_W * k)
                        .selected_text(format!("Slot {}", self.target_instance))
                        .show_ui(ui, |ui| {
                            // The popup opens in an Area of its own, which is
                            // built from the context's style, not this step's.
                            theme::apply_setup_style(ui, pal);
                            ui.spacing_mut().item_spacing.y = 2.0 * k;
                            for n in 1..=menu_state::MAX_INSTANCES {
                                theme::pointer(ui.selectable_value(
                                    &mut self.target_instance,
                                    n,
                                    format!("Slot {n}"),
                                ));
                            }
                        });
                    theme::pointer(slots.response);
                    let cur = menu_state::lock().current_instance_id;
                    ui.label(
                        RichText::new(if self.target_instance == cur {
                            "current window"
                        } else {
                            "open from the Instances menu after install"
                        })
                        .size(theme::HINT_FONT * k)
                        .color(pal.dim),
                    );
                });
                // A slot holds one installation, so installing over another
                // model takes its firmware and its eMMC with it.
                if let Some(prev) =
                    cdj3k_emu_storage::InstanceSettings::saved_model(self.target_instance)
                {
                    if prev != self.model {
                        ui.label(
                            RichText::new(format!(
                                "Slot {} holds the {}. Installing replaces it, erasing its eMMC.",
                                self.target_instance,
                                prev.title()
                            ))
                            .size(theme::HINT_FONT * k)
                            .color(pal.warn),
                        );
                    }
                }
            });
        });

        ui.horizontal_top(|ui| {
            label_column(ui, k, pal.dim, "Firmware");
            ui.vertical(|ui| {
                self.path_row(ui, "wizard_upd", true, k);
                // Say so while the form is still up, rather than after Install.
                let (note, col) = match self.upd_objection() {
                    Some(objection) => (objection, pal.warn),
                    None => (
                        "A .UPD or its decrypted .iso. The name has to be one Pioneer publishes \
                         this deck under."
                            .to_owned(),
                        pal.faint,
                    ),
                };
                ui.label(RichText::new(note).size(theme::HINT_FONT * k).color(col));
            });
        });

        // The key decrypts a .UPD; an .iso has been decrypted already and a
        // file has yet to be chosen. Only the one case has anything to ask for.
        if !self.upd_path.is_empty() && !self.firmware_is_iso() {
            ui.horizontal_top(|ui| {
                label_column(ui, k, pal.dim, "Decryption key");
                ui.vertical(|ui| self.path_row(ui, "wizard_key", false, k));
            });
        }
    }

    /// Whether the chosen firmware file is an already-decrypted ISO image.
    fn firmware_is_iso(&self) -> bool {
        !self.upd_path.is_empty() && is_iso9660(Path::new(&self.upd_path))
    }

    /// Why the chosen `.UPD` cannot go in this slot, if it cannot. An `.iso`
    /// is taken on trust, and an empty field has nothing to object to.
    fn upd_objection(&self) -> Option<String> {
        if self.upd_path.is_empty() || self.firmware_is_iso() {
            return None;
        }
        upd_name_objection(&self.upd_path, self.model, self.target_instance)
    }

    fn path_row(&mut self, ui: &mut egui::Ui, salt: &str, is_upd: bool, k: f32) {
        let hint = if is_upd {
            "/path/to/.UPD or .ISO"
        } else {
            "/path/to/aes256.key"
        };
        // The field takes what the Browse button beside it leaves. `desired_width`
        // is the text area, so the field's own margin comes out of the budget too
        // or the button runs off the edge.
        let field_w = ui.available_width()
            - BROWSE_W * k
            - ui.spacing().item_spacing.x
            - FIELD_MARGIN * 2.0 * k;
        // A TextEdit lays its galley out against the top margin, so the margin
        // is what centres the line in a field of a fixed height.
        let row = ui.text_style_height(&egui::TextStyle::Monospace);
        let pad_y = ((theme::FIELD_H * k - row) * 0.5).max(0.0);
        // Accumulate the file-picker result outside the closure to avoid
        // conflicting borrows with the TextEdit's &mut path reference.
        let mut new_pick: Option<String> = None;
        ui.horizontal(|ui| {
            let path = if is_upd {
                &mut self.upd_path
            } else {
                &mut self.key_path
            };
            ui.add(
                TextEdit::singleline(path)
                    .id_salt(salt)
                    .desired_width(field_w)
                    .min_size(Vec2::new(0.0, theme::FIELD_H * k))
                    .margin(Margin::symmetric(FIELD_MARGIN * k, pad_y))
                    .hint_text(hint)
                    .font(egui::TextStyle::Monospace),
            );
            let pal = theme::palette(ui.ctx());
            if theme::secondary(ui, [BROWSE_W * k, theme::FIELD_H * k], "Browse…", pal).clicked()
            {
                let picked = if is_upd {
                    open_file_picker(
                        "Select firmware (.UPD or decrypted .iso)",
                        &["UPD", "upd", "iso", "ISO"],
                    )
                } else {
                    open_file_picker("Select key file", &[])
                };
                if let Some(p) = picked {
                    new_pick = Some(p.to_string_lossy().into_owned());
                }
            }
        });
        if let Some(val) = new_pick {
            if is_upd {
                self.upd_path = val;
            } else {
                self.key_path = val;
            }
        }
    }

    fn start_provision(&mut self, ctx: Context) {
        // A decrypted ISO needs no key; a .UPD does.
        let key = if self.firmware_is_iso() {
            None
        } else {
            match cdj3k_emu_firmware::LuksKey::from_file(Path::new(&self.key_path)) {
                Ok(k) => Some(k),
                Err(e) => {
                    *self.log.lock().unwrap() = format!("[key] could not read key file: {e}\n");
                    self.terminal = Some(ProvisionStep::Error(format!(
                        "Could not read key file: {e}"
                    )));
                    return;
                }
            }
        };

        let upd = PathBuf::from(&self.upd_path);
        let target = self.target_instance;
        let model = self.model;
        let status = Arc::new(Mutex::new(ProvisionStep::Decrypting));
        self.provision_status = Some(status.clone());

        // Clear + recycle the same log Arc.
        self.log.lock().unwrap().clear();
        let log = self.log.clone();

        std::thread::Builder::new()
            .name("cdj3k-emu-provision".into())
            .spawn(move || provision(upd, key, target, model, status, log, ctx))
            .expect("failed to spawn provision thread");
    }

    pub(super) fn reset(&mut self) {
        // An install that went through has consumed its firmware file, so the
        // next one starts on an empty form. One that was called off or failed
        // keeps its paths, the next attempt being the same one again.
        if matches!(self.terminal, Some(ProvisionStep::Done)) {
            self.upd_path.clear();
            self.key_path.clear();
        }
        self.provision_status = None;
        self.terminal = None;
        self.log.lock().unwrap().clear();
    }
}

/// A field label: capitals at the window's micro size, tracked by the painter
/// because egui has no letter-spacing.
fn micro_label(ui: &mut egui::Ui, k: f32, col: Color32, text: &str) {
    let text = text.to_uppercase();
    let font = FontId::proportional(theme::MICRO_FONT * k);
    let w = picker::text_width(ui.painter(), &text, &font, theme::MICRO_TRACK * k);
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(w, theme::MICRO_FONT * k * 1.6), Sense::hover());
    picker::tracked(
        ui.painter(),
        Pos2::new(rect.left(), rect.center().y),
        &text,
        &font,
        col,
        theme::MICRO_TRACK * k,
    );
}

/// The same label, in the fixed column a form row hangs its field off, sitting
/// level with the middle of that field.
fn label_column(ui: &mut egui::Ui, k: f32, col: Color32, text: &str) {
    let text = text.to_uppercase();
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(LABEL_W * k, theme::FIELD_H * k), Sense::hover());
    picker::tracked(
        ui.painter(),
        Pos2::new(rect.left(), rect.center().y),
        &text,
        &FontId::proportional(theme::MICRO_FONT * k),
        col,
        theme::MICRO_TRACK * k,
    );
}

/// A plate across the step saying how the run ended: a square of the state's
/// colour, then the line itself.
fn banner(
    ui: &mut egui::Ui,
    k: f32,
    pal: &theme::Palette,
    mark: Color32,
    text: &str,
    col: Color32,
) {
    Frame::default()
        .fill(pal.plate)
        .stroke(Stroke::new(theme::HAIRLINE * k, pal.line))
        .rounding(Rounding::same(theme::ROUND * k))
        .inner_margin(Margin::symmetric(14.0 * k, 11.0 * k))
        .show(ui, |ui| {
            // Full width, so it reads as a bar across the step rather than a
            // tag pinned to its own text.
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(MARK * k), Sense::hover());
                ui.painter().rect_filled(rect, theme::HAIRLINE * k, mark);
                ui.label(RichText::new(text).size(theme::BODY_FONT * k).color(col));
            });
        });
}

/// The stage meter: the stage under way, how far along the run is, and the
/// four stages named under their own segments.
fn meter(ui: &mut egui::Ui, k: f32, pal: &theme::Palette, step: &ProvisionStep) {
    let Some(at) = step.stage() else {
        return;
    };
    Frame::default()
        .fill(pal.plate)
        .stroke(Stroke::new(theme::HAIRLINE * k, pal.line))
        .rounding(Rounding::same(theme::ROUND * k))
        .inner_margin(Margin::symmetric(16.0 * k, 14.0 * k))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 9.0 * k;
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(step.label())
                        .size(METER_TITLE_FONT * k)
                        .color(pal.ink),
                );
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("STEP {} / {}", at + 1, STAGES.len()))
                            .size(theme::HINT_FONT * k)
                            .monospace()
                            .color(pal.dim),
                    );
                });
            });

            let n = STAGES.len() as f32;
            let gap = METER_GAP * k;
            let (bar, _) = ui.allocate_exact_size(
                Vec2::new(ui.available_width(), METER_SEG_H * k),
                Sense::hover(),
            );
            let seg = (bar.width() - gap * (n - 1.0)) / n;
            for i in 0..STAGES.len() {
                let x = bar.left() + i as f32 * (seg + gap);
                ui.painter().rect_filled(
                    Rect::from_min_size(Pos2::new(x, bar.top()), Vec2::new(seg, bar.height())),
                    0.0,
                    if i <= at { pal.ink } else { pal.off_line },
                );
            }

            let (names, _) = ui.allocate_exact_size(
                Vec2::new(ui.available_width(), theme::HINT_FONT * k * 1.5),
                Sense::hover(),
            );
            for (i, name) in STAGES.iter().enumerate() {
                let (col, font) = match i {
                    _ if i < at => (pal.dim, FontId::proportional(theme::HINT_FONT * k)),
                    _ if i == at => (pal.ink, FontId::proportional(theme::HINT_FONT * k)),
                    _ => (pal.faint, FontId::proportional(theme::HINT_FONT * k)),
                };
                ui.painter().text(
                    Pos2::new(names.left() + i as f32 * (seg + gap), names.center().y),
                    egui::Align2::LEFT_CENTER,
                    name,
                    font,
                    col,
                );
            }
        });
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
                .map_err(|e| format!("could not read key file {}: {e}", p.display()))?,
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
        other => Err(format!("stopped at: {}", other.label())),
    }
}

/// Why an update file cannot go in a slot being set up as `model`, if it
/// cannot.
///
/// A `.UPD` is a bare LUKS container whose header names no product, so its
/// file name is the whole of the evidence: it has to be one Pioneer publishes
/// this deck under. A name belonging to another deck, or to none of ours, is
/// refused - a CDJ-3000 slot takes a `CDJ3K…`, a CDJ-3000X slot a `CDJ3000X…`, and
/// nothing else.
fn upd_name_objection(path: &str, model: Model, slot: u32) -> Option<String> {
    match Model::from_firmware_file_name(path) {
        Some(named) if named == model => None,
        Some(named) => Some(format!(
            "This is a {named} update. Slot {slot} is being set up as a {model}."
        )),
        None => Some(format!(
            "This is not a {model} update. Its name should be {}, as Pioneer publishes it.",
            name_list(model)
        )),
    }
}

/// "CDJ3K… or CDJ3000…", the names a deck's update file goes by.
fn name_list(model: Model) -> String {
    let names: Vec<String> = model
        .firmware_file_names()
        .iter()
        .map(|n| format!("{n}…"))
        .collect();
    match names.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => String::new(),
    }
}

// ── Background provisioning ───────────────────────────────────────────────────

fn bundled_resources() -> std::path::PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let macos = exe.parent().unwrap_or(std::path::Path::new("."));
        let resources = macos.parent().unwrap_or(macos).join("Resources");
        if resources.exists() {
            return resources;
        }
        let dev = macos.join("resources");
        if dev.exists() {
            return dev;
        }
    }
    std::path::PathBuf::from("resources")
}

fn provision(
    upd_path: PathBuf,
    key: Option<cdj3k_emu_firmware::LuksKey>,
    target_instance: u32,
    model: Model,
    status: Arc<Mutex<ProvisionStep>>,
    log: Arc<Mutex<String>>,
    ctx: Context,
) {
    macro_rules! set {
        ($step:expr) => {{
            *status.lock().unwrap() = $step;
            ctx.request_repaint();
        }};
    }
    macro_rules! log {
        ($($arg:tt)*) => {{
            let line = format!($($arg)*);
            eprintln!("cdj3k-emu-provision: {line}");
            {
                let mut g = log.lock().unwrap();
                g.push_str(&line);
                g.push('\n');
            }
            ctx.request_repaint();
        }};
    }
    macro_rules! try_step {
        ($step:expr, $expr:expr) => {
            match $expr {
                Ok(v) => v,
                Err(e) => {
                    log!("[error] {}", e.to_string());
                    set!(ProvisionStep::Error(
                        "Error while processing: see the console for details".to_string()
                    ));
                    return;
                }
            }
        };
    }

    let paths = FirmwarePaths::new(target_instance);
    log!("[slot] target = instance-{} ({})", target_instance, model);
    log!("[slot] firmware dir = {}", paths.dir().display());

    // A `.UPD` is a bare LUKS container whose header names no product, so its
    // file name is all there is to go on. An `.iso` has been unwrapped by
    // hand already and is taken on trust. Refusing happens here, before
    // anything is read or written, so it costs the slot nothing.
    if key.is_some() {
        let name = upd_path.to_string_lossy();
        match upd_name_objection(&name, model, target_instance) {
            Some(objection) => {
                log!("[deck] {objection}");
                set!(ProvisionStep::Error(objection));
                return;
            }
            None => log!("[deck] named as a {model} update"),
        }
    }

    // 1. Decrypt UPD → tmp ISO (or use the given ISO as is)
    set!(ProvisionStep::Decrypting);
    let (tmp_iso, tmp_iso_owned) = match &key {
        Some(key) => {
            log!("[decrypt] opening {}", upd_path.display());
            let tmp_iso = std::env::temp_dir()
                .join(format!("cdj3k-emu-firmware-{}.iso.tmp", target_instance));
            log!("[decrypt] output → {}", tmp_iso.display());
            try_step!(
                ProvisionStep::Decrypting,
                cdj3k_emu_firmware::decrypt_upd(&upd_path, key, &tmp_iso)
            );
            log!("[decrypt] OK - ISO written");
            (tmp_iso, true)
        }
        None => {
            log!(
                "[decrypt] skipped - {} is already an ISO 9660 image",
                upd_path.display()
            );
            (upd_path.clone(), false)
        }
    };

    let firmware_info = match cdj3k_emu_firmware::read_firmware_info(&tmp_iso) {
        Ok(info) => {
            let unknown = "(unknown)";
            log!(
                "[firmware] release={} rev_apl={} rev_kernel={} miniloader={}",
                info.release.as_deref().unwrap_or(unknown),
                info.rev_apl.as_deref().unwrap_or(unknown),
                info.rev_kernel.as_deref().unwrap_or(unknown),
                info.miniloader.as_deref().unwrap_or(unknown),
            );
            info
        }
        Err(e) => {
            log!("[firmware] could not read version info ({e}), defaulting to all-unknown");
            cdj3k_emu_storage::FirmwareInfo::default()
        }
    };

    // 2. A slot holds one installation. Clear whatever was there - including
    //    the eMMC and everything the guest wrote to it - and record the model
    //    the slot now is.
    if let Some(prev) = cdj3k_emu_storage::InstanceSettings::saved_model(target_instance) {
        if prev != model {
            log!("[slot] replacing the {prev} installation");
        }
    }
    try_step!(ProvisionStep::ExtractingKernel, paths.remove());
    let release = firmware_info.release.clone();
    if let Err(e) = cdj3k_emu_storage::InstanceSettings::update(target_instance, move |s| {
        s.model = Some(model);
        s.firmware_release = release;
    }) {
        log!("[slot] recording the slot model failed: {e}");
    }

    // 3. Install vanilla kernel + extract Pioneer initramfs
    set!(ProvisionStep::ExtractingKernel);
    let out_dir = paths.dir().to_path_buf();
    try_step!(
        ProvisionStep::ExtractingKernel,
        std::fs::create_dir_all(&out_dir)
    );

    // Vanilla kernel ships in the app bundle - no Pioneer extraction or SMC patching.
    let resources_dir = bundled_resources();
    let kernel_src = resources_dir.join("Image");
    let kernel_out = paths.kernel.clone();
    log!(
        "[kernel] installing kernel {} → {}",
        kernel_src.display(),
        kernel_out.display()
    );
    try_step!(
        ProvisionStep::ExtractingKernel,
        std::fs::copy(&kernel_src, &kernel_out)
            .map(|_| ())
            .map_err(|e| std::io::Error::other(format!("copy Image: {e}")))
    );
    log!("[kernel] OK");

    // 4. Patch initramfs - still uses Pioneer firmware as the rootfs base
    //    (EP122 binary and Pioneer libraries live there).
    //    The Pioneer kernel is extracted to a temp file solely to unpack the
    //    embedded initramfs; it is not used as the final kernel.
    set!(ProvisionStep::PatchingInitramfs);
    log!("[patch] resources dir: {}", resources_dir.display());
    let pioneer_kernel_tmp = out_dir.join("Image.pioneer.tmp");
    let initramfs_patched = paths.initramfs.clone();
    let initramfs_orig = out_dir.join("initramfs-orig.cpio");

    log!(
        "[initramfs] extracting Pioneer kernel (initramfs source) → {}",
        pioneer_kernel_tmp.display()
    );
    try_step!(
        ProvisionStep::PatchingInitramfs,
        cdj3k_emu_firmware::extract_kernel(&tmp_iso, &pioneer_kernel_tmp, |msg| log!("{}", msg))
            .map_err(|e| std::io::Error::other(format!("{e}")))
    );
    log!(
        "[initramfs] extracting from Pioneer kernel → {}",
        initramfs_orig.display()
    );
    try_step!(
        ProvisionStep::PatchingInitramfs,
        cdj3k_emu_firmware::extract_initramfs(&pioneer_kernel_tmp, &initramfs_orig)
            .map_err(|e| std::io::Error::other(format!("{e:?}")))
    );
    let _ = std::fs::remove_file(&pioneer_kernel_tmp);
    log!("[initramfs] OK");

    try_step!(
        ProvisionStep::PatchingInitramfs,
        cdj3k_emu_firmware::patch_initramfs(&initramfs_orig, &resources_dir, &initramfs_patched)
            .map_err(|e| std::io::Error::other(e.to_string()))
    );
    log!("[patch] OK → {}", initramfs_patched.display());
    let _ = std::fs::remove_file(&initramfs_orig);

    // 5. Create the eMMC qcow2 (the slot's old one went with `paths.remove`)
    set!(ProvisionStep::CreatingEmmc);

    // The eMMC is new, so the slot needs the serial `genkey_pr` hashes into
    // the cabinet passphrase - which the cabinet is keyed for just below, so
    // it has to be settled first.  CDJ3K_SOC_SERIAL pins a particular deck's,
    // which is what a cabinet lifted off that deck already opens with.
    let serial = match std::env::var("CDJ3K_SOC_SERIAL") {
        Ok(pinned) => cdj3k_emu_storage::InstanceSettings::set_soc_serial(target_instance, &pinned),
        Err(_) => cdj3k_emu_storage::InstanceSettings::regenerate_soc_serial(target_instance),
    };
    let serial = match serial {
        Ok(serial) => {
            log!("[serial] SoC serial = {}", serial);
            serial
        }
        Err(e) => {
            // Not fatal: the slot boots on the serial it already has, and the
            // cabinet below is keyed for that one.
            let kept =
                cdj3k_emu_storage::InstanceSettings::load_or_init(target_instance).soc_serial;
            log!("[serial] could not persist a new SoC serial ({e}) - keeping {kept}");
            kept
        }
    };

    // cabinet.img carries the Widevine keys and the Device Library Plus key
    // file.  The deck's updater copies it onto the settings partition, where
    // apl_start.sh opens it on every boot (`genkey_pr | initoptenv`); here it
    // is staged in the recovery partition for guest patch 14.
    // The firmware's own image is keyed for the factory passphrase genkey_pb
    // computes (model + images.tar.gz); a keyslot for this slot's own serial
    // is added to it below.
    // CDJ3K_CABINET_IMG stages a cabinet taken off a real deck instead.
    let cabinet = match std::env::var_os("CDJ3K_CABINET_IMG") {
        Some(path) => {
            let path = std::path::PathBuf::from(path);
            match std::fs::read(&path) {
                Ok(bytes) if bytes.starts_with(b"LUKS\xba\xbe") => {
                    log!(
                        "[cabinet] {} ({} MiB)",
                        path.display(),
                        bytes.len() / 1_048_576
                    );
                    Some(bytes)
                }
                Ok(_) => {
                    log!(
                        "[cabinet] {} is not a LUKS container - skipped",
                        path.display()
                    );
                    None
                }
                Err(e) => {
                    log!("[cabinet] {}: {e} - skipped", path.display());
                    None
                }
            }
        }
        None => cdj3k_emu_firmware::read_cabinet_image(&tmp_iso),
    };

    // Give the container a keyslot this slot can open.  The firmware ships it
    // keyed for the factory passphrase (genkey_pb, from the model and this
    // ISO's images.tar.gz); opening that slot recovers the master key every
    // CDJ-3000X cabinet shares, and a slot keyed for this unit's serial is
    // added from it.  A container no factory slot opens - a foreign cabinet,
    // or one from another firmware version - is refused and stages unchanged.
    let mut cabinet = cabinet;
    if let Some(bytes) = cabinet.as_mut() {
        let model_env = model.spec().model_env;
        let vendor = cdj3k_emu_firmware::read_images_targz(&tmp_iso)
            .map(|tgz| cdj3k_emu_firmware::vendor_passphrase(model_env, &tgz));
        match (
            vendor,
            cdj3k_emu_firmware::cabinet_passphrase(model_env, &serial),
        ) {
            (Some(vendor), Ok(unit)) => {
                match cdj3k_emu_firmware::add_keyslot(bytes, &vendor, &unit) {
                    Ok(slot) => log!("[cabinet] keyed for {serial} in slot {slot}"),
                    Err(e) => log!("[cabinet] not keyed for this slot ({e}) - staging it as it is"),
                }
            }
            (None, _) => log!("[cabinet] no images.tar.gz to key from - staging it as it is"),
            (_, Err(e)) => {
                log!("[cabinet] no passphrase for this slot ({e}) - staging it as it is")
            }
        }
    }
    match &cabinet {
        Some(b) => log!("[cabinet] staging {} MiB", b.len() / 1_048_576),
        None => log!("[cabinet] nothing to stage"),
    }

    let emmc_path = paths.emmc.clone();
    log!("[emmc] provisioning → {}", emmc_path.display());
    let emmc_cfg = cdj3k_emu_storage::EmmcConfig {
        path: emmc_path,
        instance_id: target_instance,
        model,
        firmware: firmware_info,
        cabinet,
    };
    try_step!(
        ProvisionStep::CreatingEmmc,
        cdj3k_emu_storage::provision_emmc(&emmc_cfg)
    );
    log!("[emmc] OK");

    if tmp_iso_owned {
        let _ = std::fs::remove_file(&tmp_iso);
    }

    log!("[done] all steps completed");
    set!(ProvisionStep::Done);
}
