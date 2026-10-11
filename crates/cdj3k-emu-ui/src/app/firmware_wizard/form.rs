//! The install form: the firmware file, its key, and the developer options.

use std::path::Path;

use egui::{RichText, TextEdit, Vec2};

use cdj3k_emu_platform::desktop::PendingPick;

use super::provision::upd_name_objection;
use super::widgets::label_column;
use super::FirmwareWizard;
use crate::app::theme;

/// Horizontal padding inside a text field, counted against the row's width so
/// the Browse button beside it still fits.
const FIELD_MARGIN: f32 = 11.0;
const BROWSE_W: f32 = 90.0;

/// Whether the file is an ISO 9660 image, which has "CD001" at byte 1 of the
/// primary volume descriptor in sector 16.
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

impl FirmwareWizard {
    /// The form: one row per thing the install needs, each with its label in
    /// its own column so the fields line up down the step.
    pub(super) fn form(&mut self, ui: &mut egui::Ui, k: f32, pal: &theme::Palette) {
        // A slot holds one installation, so installing over another model
        // removes that model's firmware and its eMMC.
        if let Some(prev) = cdj3k_emu_storage::InstanceSettings::saved_model(self.target_instance) {
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

        ui.horizontal_top(|ui| {
            label_column(ui, k, pal.dim, "Firmware");
            ui.vertical(|ui| {
                self.path_row(ui, "wizard_upd", true, k);
                // Show the objection while the form is up, before Install is pressed.
                let (note, col) = match self.upd_objection() {
                    Some(objection) => (objection, pal.warn),
                    None => (
                        "A .UPD or its decrypted .iso. The file name must be one Pioneer publishes \
                         for this deck."
                            .to_owned(),
                        pal.faint,
                    ),
                };
                ui.label(RichText::new(note).size(theme::HINT_FONT * k).color(col));
            });
        });

        // Ask for the key only once a .UPD is chosen: the key decrypts a .UPD,
        // an .iso is already decrypted, and an empty field has nothing to
        // decrypt.
        if !self.upd_path.is_empty() && !self.firmware_is_iso() {
            ui.horizontal_top(|ui| {
                label_column(ui, k, pal.dim, "Decryption key");
                ui.vertical(|ui| self.path_row(ui, "wizard_key", false, k));
            });
        }

        ui.horizontal_top(|ui| {
            label_column(ui, k, pal.dim, "Developer");
            ui.vertical(|ui| {
                ui.checkbox(
                    &mut self.ssh,
                    RichText::new("Root SSH").size(theme::HINT_FONT * k).color(pal.dim),
                );
                if self.ssh {
                    ui.label(
                        RichText::new(
                            "Root access isn't protected with a password. Anyone who can reach this emulation can log in.",
                        )
                        .size(theme::HINT_FONT * k)
                        .color(pal.warn),
                    );
                }
            });
        });
    }

    /// Whether the chosen firmware file is an already-decrypted ISO image.
    pub(super) fn firmware_is_iso(&self) -> bool {
        if self.upd_path.is_empty() {
            return false;
        }
        let mut probe = self.iso_probe.borrow_mut();
        match &*probe {
            Some((path, is_iso)) if *path == self.upd_path => *is_iso,
            _ => {
                let is_iso = is_iso9660(Path::new(&self.upd_path));
                *probe = Some((self.upd_path.clone(), is_iso));
                is_iso
            }
        }
    }

    /// Why the chosen `.UPD` cannot go in this slot, if it cannot. An `.iso` is
    /// accepted without a check, and an empty field has nothing to check.
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
        // The field gets the width that the Browse button beside it leaves.
        // `desired_width` is the text area, so the field's own margin is taken
        // out of the width too, or the button runs off the edge.
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
        // Collect an answer from a dialog this field opened on an earlier
        // frame. Both fields run this every frame, so each leaves the other's
        // dialog alone.
        if let Some((for_upd, pick)) = &self.pending_pick {
            if *for_upd == is_upd {
                if let Some(answer) = pick.take() {
                    self.pending_pick = None;
                    new_pick = answer.map(|p| p.to_string_lossy().into_owned());
                }
            }
        }
        // A dialog is up: keep asking for frames so its answer is noticed.
        if self.pending_pick.is_some() {
            ui.ctx().request_repaint();
        }
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
                    .margin(egui::epaint::MarginF32::symmetric(FIELD_MARGIN * k, pad_y))
                    .hint_text(hint)
                    .font(egui::TextStyle::Monospace),
            );
            let pal = theme::palette(ui.ctx());
            if theme::secondary(ui, [BROWSE_W * k, theme::FIELD_H * k], "Browse…", pal).clicked()
            {
                // The dialog runs on its own thread. The check at the top of
                // this function collects its answer on a later frame, so the
                // frame loop keeps running while the dialog is up.
                self.pending_pick = Some((
                    is_upd,
                    if is_upd {
                        PendingPick::open(
                            "Select firmware (.UPD or decrypted .iso)",
                            &["UPD", "upd", "iso", "ISO"],
                        )
                    } else {
                        PendingPick::open("Select key file", &[])
                    },
                ));
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
}
