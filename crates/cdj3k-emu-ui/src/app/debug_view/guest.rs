//! The Guest tab: whether the emulation runs, and the guest's CPU, memory and
//! journal, which the shell samples through devd while the window is open.

use std::collections::VecDeque;

use egui::RichText;

/// What the Guest tab shows. The shell fills it.
#[derive(Clone, Default)]
pub(crate) struct GuestView {
    pub running: bool,
    /// Whether the player app drives the panel on the current run.
    pub booted: bool,
    /// Whether the guest's cfgd has sent a message on this boot.
    pub cfgd: bool,
    /// Whether the guest's devd answers. It runs only in Developer Mode.
    pub devd: bool,
    pub stats: Option<GuestStats>,
    pub journal: VecDeque<String>,
}

#[derive(Clone, Default)]
pub(crate) struct GuestStats {
    pub uptime_s: f64,
    pub load: [f32; 3],
    /// The busy share of all CPUs first, then of each CPU, from 0 to 1. Empty
    /// until two samples have been taken.
    pub cpu: Vec<f32>,
    pub memory: Memory,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Memory {
    pub total_kb: u64,
    pub available_kb: u64,
    pub swap_total_kb: u64,
    pub swap_free_kb: u64,
}

fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

/// Draw the run state, the stats and the CPU bars side by side, then the
/// journal across the width.
pub(super) fn draw(ui: &mut egui::Ui, view: &GuestView, filter: &mut String) {
    ui.horizontal_top(|ui| {
        super::rows(
            ui,
            "guest_state",
            &[
                ("QEMU running", yes_no(view.running).into()),
                ("Player app up", yes_no(view.booted).into()),
                ("cfgd", yes_no(view.cfgd).into()),
                ("devd", yes_no(view.devd).into()),
            ],
        );
        if let Some(stats) = view.stats.as_ref().filter(|_| view.devd) {
            ui.add_space(super::BLOCK_GAP);
            draw_stats(ui, stats);
            ui.add_space(super::BLOCK_GAP);
            ui.vertical(|ui| draw_cpu(ui, stats));
        }
    });
    ui.separator();
    if !view.devd {
        ui.label(
            "CPU, memory and the journal come from the guest's devd, which runs in Developer \
             Mode.",
        );
        return;
    }
    draw_journal(ui, &view.journal, filter);
}

fn draw_stats(ui: &mut egui::Ui, stats: &GuestStats) {
    let m = stats.memory;
    let used_kb = m.total_kb.saturating_sub(m.available_kb);
    let mib = |kb: u64| kb / 1024;
    super::rows(
        ui,
        "guest_stats",
        &[
            ("Uptime", format!("{:.0} s", stats.uptime_s)),
            (
                "Load",
                format!(
                    "{:.2} {:.2} {:.2}",
                    stats.load[0], stats.load[1], stats.load[2]
                ),
            ),
            (
                "Memory used",
                format!("{} of {} MiB", mib(used_kb), mib(m.total_kb)),
            ),
            (
                "Swap used",
                format!(
                    "{} of {} MiB",
                    mib(m.swap_total_kb.saturating_sub(m.swap_free_kb)),
                    mib(m.swap_total_kb)
                ),
            ),
        ],
    );
}

/// Draw a bar for the busy share of all CPUs, then one bar per CPU.
fn draw_cpu(ui: &mut egui::Ui, stats: &GuestStats) {
    let Some((total, cores)) = stats.cpu.split_first() else {
        return;
    };
    ui.add(egui::ProgressBar::new(*total).text(format!("CPU {:.0}%", total * 100.0)));
    for (i, core) in cores.iter().enumerate() {
        ui.add(
            egui::ProgressBar::new(*core)
                .desired_height(8.0)
                .text(RichText::new(format!("cpu{i} {:.0}%", core * 100.0)).small()),
        );
    }
}

fn draw_journal(ui: &mut egui::Ui, journal: &VecDeque<String>, filter: &mut String) {
    ui.horizontal(|ui| {
        ui.label("Journal");
        ui.add(egui::TextEdit::singleline(filter).hint_text("filter"));
    });
    let needle = filter.to_lowercase();
    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for line in journal
                .iter()
                .filter(|l| needle.is_empty() || l.to_lowercase().contains(&needle))
            {
                ui.label(RichText::new(line).monospace().size(11.0));
            }
        });
}
