//! Deck mods: the QEMU restart that a change to the slot's mods requests, and
//! the guest's mods report and journals for each boot.

use cdj3k_emu_platform::menu_state;
use cdj3k_emu_runtime::mods_report::ModsReport;

use super::Worker;

impl Worker {
    /// If a mods restart was requested after the current boot read the
    /// slot's mods, restart QEMU. Wait until the guest's cfgd answers: a
    /// guest that is still booting cannot power off and is killed after 8 s.
    pub(super) fn request_mods_restart(&self) {
        let mut s = menu_state::lock();
        if self.instance.is_some()
            && s.qemu_running
            && s.mods_restart_seq != s.mods_restart_seq_booted
            && self.cfg_client.guest_heard()
            && !s.restart_requested
        {
            s.restart_requested = true;
            s.shade_forced = true;
        }
    }

    /// Record each new mods report, request the journal of every mod it
    /// names, and save the journals that have arrived.
    pub(super) fn track_mods_report(&mut self) {
        let report = self.cfg_client.mods_report();
        if report != self.last_mods_report {
            self.on_new_mods_report(&report);
            self.last_mods_report = report;
        }
        self.save_arrived_mod_logs();
    }

    fn on_new_mods_report(&mut self, report: &ModsReport) {
        if matches!(report, ModsReport::Done { .. } | ModsReport::Off) {
            record_mods_report(self.config.instance_id, report);
        }
        self.mod_logs_due.clear();
        if let ModsReport::Done { outcomes, unread } = report {
            let names = outcomes.iter().map(|o| &o.name);
            for name in names.chain(unread.iter().map(|u| &u.name)) {
                if self.cfg_client.request_mod_log(name).is_ok() {
                    self.mod_logs_due.push(name.clone());
                }
            }
        }
    }

    fn save_arrived_mod_logs(&mut self) {
        let instance_id = self.config.instance_id;
        let cfg_client = &self.cfg_client;
        self.mod_logs_due
            .retain(|name| match cfg_client.mod_log(name) {
                Some(lines) => {
                    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
                    if let Err(e) =
                        cdj3k_emu_storage::mods::save_last_boot_log(instance_id, name, &text)
                    {
                        eprintln!("cdj3k-emu: saving the journal of mod {name} failed: {e}");
                    }
                    false
                }
                None => true,
            });
    }
}

/// Log the guest's mods report for this boot and save it to the slot's
/// `mods/last-boot.txt`, which the Mods view reads when no emulation runs.
fn record_mods_report(instance_id: u32, report: &ModsReport) {
    if let ModsReport::Done { outcomes, unread } = report {
        for o in outcomes {
            eprintln!("cdj3k-emu: mod {}", o.line().trim_start_matches("mod "));
        }
        for u in unread {
            eprintln!(
                "cdj3k-emu: mod {}: the report line {} could not be parsed.",
                u.name, u.line
            );
        }
    }
    let text: String = report.lines().iter().map(|l| format!("{l}\n")).collect();
    let ids = cdj3k_emu_storage::mods::current_boot().map_or_else(Vec::new, |b| b.slot_run);
    if let Err(e) = cdj3k_emu_storage::mods::save_last_boot_report(instance_id, &text, &ids) {
        eprintln!("cdj3k-emu: saving the mods report failed: {e}");
    }
}
