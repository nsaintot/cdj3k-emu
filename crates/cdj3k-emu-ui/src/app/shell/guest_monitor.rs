//! Samples the guest for the debug window's Guest tab while that window is
//! open: CPU, memory and load every [`SAMPLE_EVERY`], and the journal as it
//! grows. Both come through the guest's devd, so they need Developer Mode.

use std::time::{Duration, Instant};

use cdj3k_emu_runtime::dev::{DevClient, Job, JobEvent};

use crate::app::debug_view::{GuestStats, GuestView, Memory};

/// How often the CPU, memory and load are sampled.
const SAMPLE_EVERY: Duration = Duration::from_secs(2);
/// The number of journal lines the tab keeps.
const JOURNAL_LINES: usize = 1000;

const STATS_COMMAND: &str = "cat /proc/uptime /proc/loadavg; grep '^cpu' /proc/stat; \
    grep -E '^(MemTotal|MemAvailable|SwapTotal|SwapFree):' /proc/meminfo";
const JOURNAL_COMMAND: &str = "exec journalctl -f -n 200 --no-pager -o short-monotonic";

#[derive(Default)]
pub(super) struct GuestMonitor {
    next_sample: Option<Instant>,
    sample: Option<(Job, String)>,
    journal: Option<Job>,
    /// The CPU times of the last sample. The next sample measures CPU usage
    /// against them.
    prev_cpu: Vec<CpuTimes>,
}

impl GuestMonitor {
    /// Sample the guest while the window is `open` and devd is up, and update
    /// `view`. Closing the window stops the journal job.
    pub(super) fn tick(&mut self, open: bool, dev: Option<&DevClient>, view: &mut GuestView) {
        let dev = dev.filter(|d| d.is_up());
        view.devd = dev.is_some();
        let Some(dev) = dev.filter(|_| open) else {
            *self = Self::default();
            return;
        };
        self.follow_journal(dev, view);
        self.sample(dev, view);
    }

    fn follow_journal(&mut self, dev: &DevClient, view: &mut GuestView) {
        if self.journal.is_none() {
            view.journal.clear();
            self.journal = dev.start(JOURNAL_COMMAND).ok();
        }
        let Some(job) = self.journal.as_mut() else {
            return;
        };
        for event in job.take() {
            match event {
                JobEvent::Out(line) => {
                    if view.journal.len() == JOURNAL_LINES {
                        view.journal.pop_front();
                    }
                    view.journal.push_back(line);
                }
                // The journal job ended; start it again on the next tick.
                _ => self.journal = None,
            }
        }
    }

    fn sample(&mut self, dev: &DevClient, view: &mut GuestView) {
        let now = Instant::now();
        if self.sample.is_none() && self.next_sample.is_none_or(|t| now >= t) {
            self.next_sample = Some(now + SAMPLE_EVERY);
            self.sample = dev
                .start(STATS_COMMAND)
                .ok()
                .map(|job| (job, String::new()));
        }
        let Some((job, text)) = self.sample.as_mut() else {
            return;
        };
        let mut ended = false;
        for event in job.take() {
            match event {
                JobEvent::Out(line) => {
                    text.push_str(&line);
                    text.push('\n');
                }
                _ => ended = true,
            }
        }
        if ended {
            let (_, text) = self.sample.take().expect("checked above");
            if let Some(raw) = parse_stats(&text) {
                view.stats = Some(GuestStats {
                    uptime_s: raw.uptime_s,
                    load: raw.load,
                    cpu: cpu_usage(&self.prev_cpu, &raw.cpu),
                    memory: raw.memory,
                });
                self.prev_cpu = raw.cpu;
            }
        }
    }
}

/// The busy and idle jiffies on one `cpu` line of `/proc/stat`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CpuTimes {
    busy: u64,
    idle: u64,
}

#[derive(Debug, PartialEq)]
struct RawStats {
    uptime_s: f64,
    load: [f32; 3],
    /// The `cpu` total line first, then one per core.
    cpu: Vec<CpuTimes>,
    memory: Memory,
}

/// Parse the output of [`STATS_COMMAND`].
fn parse_stats(text: &str) -> Option<RawStats> {
    let mut lines = text.lines();
    let uptime_s = lines.next()?.split_whitespace().next()?.parse().ok()?;
    let mut load = [0.0; 3];
    for (slot, word) in load.iter_mut().zip(lines.next()?.split_whitespace()) {
        *slot = word.parse().ok()?;
    }
    let mut cpu = Vec::new();
    let mut memory = Memory::default();
    for line in lines {
        let mut words = line.split_whitespace();
        let key = words.next()?;
        if key.starts_with("cpu") {
            let n: Vec<u64> = words.filter_map(|w| w.parse().ok()).collect();
            // The fields are user, nice, system, idle, iowait, irq, softirq, steal.
            let idle = n.get(3).copied().unwrap_or(0) + n.get(4).copied().unwrap_or(0);
            let total: u64 = n.iter().take(8).sum();
            cpu.push(CpuTimes {
                busy: total.saturating_sub(idle),
                idle,
            });
            continue;
        }
        let kb: u64 = words.next()?.parse().ok()?;
        match key {
            "MemTotal:" => memory.total_kb = kb,
            "MemAvailable:" => memory.available_kb = kb,
            "SwapTotal:" => memory.swap_total_kb = kb,
            "SwapFree:" => memory.swap_free_kb = kb,
            _ => {}
        }
    }
    Some(RawStats {
        uptime_s,
        load,
        cpu,
        memory,
    })
}

/// The busy share of each CPU line between two samples, from 0 to 1. It is
/// empty without a previous sample.
fn cpu_usage(prev: &[CpuTimes], now: &[CpuTimes]) -> Vec<f32> {
    if prev.len() != now.len() {
        return Vec::new();
    }
    prev.iter()
        .zip(now)
        .map(|(a, b)| {
            let busy = b.busy.saturating_sub(a.busy) as f32;
            let idle = b.idle.saturating_sub(a.idle) as f32;
            if busy + idle > 0.0 {
                busy / (busy + idle)
            } else {
                0.0
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
12.50 40.10
0.24 0.05 0.02 1/123 456
cpu  100 0 50 800 50 0 0 0 0 0
cpu0 60 0 20 400 20 0 0 0 0 0
cpu1 40 0 30 400 30 0 0 0 0 0
MemTotal:        3000000 kB
MemAvailable:    2000000 kB
SwapTotal:             0 kB
SwapFree:              0 kB
";

    #[test]
    fn the_stats_parse_into_uptime_load_cpu_and_memory() {
        let raw = parse_stats(SAMPLE).unwrap();
        assert_eq!(raw.uptime_s, 12.5);
        assert_eq!(raw.load, [0.24, 0.05, 0.02]);
        assert_eq!(raw.cpu.len(), 3);
        assert_eq!(
            raw.cpu[0],
            CpuTimes {
                busy: 150,
                idle: 850
            }
        );
        assert_eq!(raw.memory.total_kb, 3_000_000);
        assert_eq!(raw.memory.available_kb, 2_000_000);
    }

    #[test]
    fn usage_is_the_busy_share_between_two_samples() {
        let prev = [CpuTimes {
            busy: 100,
            idle: 900,
        }];
        let now = [CpuTimes {
            busy: 150,
            idle: 950,
        }];
        assert_eq!(cpu_usage(&prev, &now), vec![0.5]);
        assert!(cpu_usage(&[], &now).is_empty());
    }

    #[test]
    fn truncated_output_does_not_parse() {
        assert_eq!(parse_stats(""), None);
        assert_eq!(parse_stats("12.5 1\n"), None);
    }
}
