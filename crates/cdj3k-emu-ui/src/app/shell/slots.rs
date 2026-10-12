//! Installs, deletes and what the shell knows of other slots.

use std::time::{Duration, Instant};

use cdj3k_emu_panel::Model;
use cdj3k_emu_platform::menu_state;
use cdj3k_emu_platform::window_socket::Command;

use super::{CdjShell, Screen, SetupStep};
use crate::app::firmware_wizard::Installed;
use crate::app::mods_view;
use crate::app::picker::PickerView;

impl CdjShell {
    /// Swap in a finished install waiting for this window's slot, if nothing
    /// runs it. Returns the model it holds.
    pub(super) fn apply_staged(&mut self) -> Option<Model> {
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

    /// Apply the wizard's finished install and report where it went. This
    /// window's slot applies it now unless its emulation runs. Another slot
    /// applies it now unless another window has the slot open, in which case
    /// that window applies it.
    pub(super) fn settle_install(&mut self, slot: u32) -> Installed {
        if slot == self.instance {
            if self.apply_staged().is_some() {
                return Installed::Applied;
            }
            return Installed::Staged;
        }
        self.busy_probe.set(None);
        match cdj3k_emu_storage::SlotClaim::take(slot) {
            Ok(Some(claim)) => match cdj3k_emu_storage::apply_staged(&claim) {
                Ok(Some(_)) => Installed::Applied,
                Ok(None) => Installed::Staged,
                Err(e) => {
                    eprintln!(
                        "cdj3k-emu: slot {slot}: swapping in the installed firmware failed: {e}"
                    );
                    Installed::Staged
                }
            },
            // Another window holds the slot; send it `install-ready` so that it
            // applies the install.
            Ok(None) => {
                self.send_command(slot, Command::InstallReady);
                Installed::Staged
            }
            Err(e) => {
                eprintln!("cdj3k-emu: slot {slot}: claiming the slot failed: {e}");
                Installed::Staged
            }
        }
    }

    /// When [`install_check_due`] is set, look for a finished install for
    /// this window's slot, staged by another window or by `--provision`. If
    /// the emulation is stopped, apply it now. Otherwise this window applies
    /// it when the emulation stops.
    ///
    /// [`install_check_due`]: Self::install_check_due
    pub(super) fn check_for_install(&mut self) {
        // Keep the check due while this window installs into the slot or
        // while the emulation stops or launches. This window applies its own
        // install when the install finishes.
        if !self.install_check_due
            || self.installing_here()
            || self.worker_stopping
            || self.pending_launch.is_some()
        {
            return;
        }
        self.install_check_due = false;
        if cdj3k_emu_runtime::worker_is_finished()
            && cdj3k_emu_storage::pending_install(self.instance).is_some()
        {
            self.apply_staged();
        }
    }

    pub(super) fn installing_here(&self) -> bool {
        self.wizard.target_instance() == self.instance && self.wizard.is_running()
    }

    /// Empty the slot. The emulation stops first, because QEMU holds the eMMC
    /// open. Nothing is installed in its place, so the window falls back to
    /// the picker with an empty slot.
    pub(super) fn begin_delete(&mut self) {
        if self.viewed_slot != self.instance {
            self.delete_other_slot(self.viewed_slot);
            return;
        }
        if cdj3k_emu_runtime::worker_is_finished() {
            self.empty_own_slot();
            // The main window behind shows a stopped panel, so it switches to
            // the picker.
            self.reveal_picker = true;
        } else {
            self.pending_delete = true;
            self.stop_emulation();
            self.setup = Some(SetupStep::StoppingToDelete);
        }
    }

    /// Empty another slot: directly when no window has it open, or else by
    /// asking its window to.
    fn delete_other_slot(&mut self, slot: u32) {
        if self.slot_is_open(slot) {
            // The other window has the eMMC open, so it must stop its
            // emulation and empty the slot itself.
            self.delete_asked = Some((slot, Instant::now(), "asking its window to empty the slot"));
            self.send_command(slot, Command::Delete);
        } else {
            if let Err(e) = cdj3k_emu_storage::FirmwarePaths::new(slot).remove() {
                eprintln!("cdj3k-emu: clearing slot {slot}'s installation failed: {e}");
            }
            self.viewed_release = None;
        }
        self.setup = Some(SetupStep::Pick);
    }

    /// Empty the slot this window owns and close the setup window, so the
    /// main window falls back to the picker.
    pub(super) fn empty_own_slot(&mut self) {
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
    pub(super) fn view_slot(&mut self, slot: u32) {
        self.viewed_slot = slot;
        self.viewed_release = cdj3k_emu_storage::slot_release(slot);
        self.busy_probe.set(None);
    }

    /// Whether another window has `slot` open, or an emulation runs it.
    /// Probed at most once a second.
    pub(super) fn slot_is_open(&self, slot: u32) -> bool {
        self.probe_slot(slot).0
    }

    /// Whether another window's emulation is running `slot`, not merely
    /// holding it open.
    pub(super) fn slot_runs_elsewhere(&self, slot: u32) -> bool {
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

    /// Whether no slot holds an installation. In that case the picker asks
    /// for an install and offers no way to close it.
    fn every_slot_empty(&self) -> bool {
        self.host.installed_model().is_none()
            && (1..=menu_state::MAX_INSTANCES)
                .filter(|&slot| slot != self.instance)
                .all(|slot| cdj3k_emu_storage::slot_summary(slot).is_none())
    }

    /// How the picker should present the slot right now.
    pub(super) fn picker_view<'a>(
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
            broken: cdj3k_emu_storage::slot_broken(self.viewed_slot).is_some(),
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
}
