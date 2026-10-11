//! The lines cfgd sends, and what each one does to the client's state.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::{flush_pending, Latency, Shared};
use crate::mods_report::{ModsReport, Partial};

/// The maximum number of lines kept from a mod's journal (cfgd sends at
/// most 500).
const MOD_LOG_LINES: usize = 600;
/// The maximum length of a journal line, in bytes; a longer line is cut.
const MOD_LOG_LINE_MAX: usize = 1024;

/// Applies the rest of a guest message's line to the shared state.
type Handler = fn(&str, &mut Shared);

/// Each guest message's prefix and its handler. No prefix is the start of
/// another, so at most one entry matches a line.
const MESSAGES: [(&str, Handler); 9] = [
    ("usb_state ", on_usb_state),
    ("latency ", on_latency),
    ("param ", on_param),
    ("log_begin ", on_log_begin),
    ("log ", on_log),
    ("log_end ", on_log_end),
    ("mods ", on_mods),
    ("mod ", on_mod),
    ("pc_link ", on_pc_link),
];

/// Apply one line from cfgd. Any line, even an unknown one, proves cfgd is
/// reading, so it first releases the queued commands.
pub(super) fn handle_line(line: &str, shared: &Arc<Mutex<Shared>>) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    flush_pending(shared);

    let Some((rest, handler)) = MESSAGES
        .iter()
        .find_map(|(prefix, handler)| Some((line.strip_prefix(prefix)?, handler)))
    else {
        return;
    };
    if let Ok(mut s) = shared.lock() {
        handler(rest, &mut s);
    }
}

/// `usb_state <0|1>`.
fn on_usb_state(rest: &str, s: &mut Shared) {
    s.usb_state = Some(rest.trim() == "1");
}

/// `latency <g>,<h>,<t>`; a malformed triple is ignored.
fn on_latency(rest: &str, s: &mut Shared) {
    let mut parts = rest.split(',');
    let g = parts.next().and_then(|x| x.trim().parse().ok());
    let h = parts.next().and_then(|x| x.trim().parse().ok());
    let t = parts.next().and_then(|x| x.trim().parse().ok());
    if let (Some(g), Some(h), Some(t)) = (g, h, t) {
        s.latency = Some(Latency {
            guest_ms: g,
            host_ms: h,
            total_ms: t,
            at: Instant::now(),
        });
    }
}

/// `param <name> <value>`.
fn on_param(rest: &str, s: &mut Shared) {
    if let Some(space) = rest.find(' ') {
        let (name, value) = rest.split_at(space);
        s.params
            .insert(name.to_string(), value.trim_start().to_string());
    }
}

/// `pc_link on|off|on_failed|off_failed|?`. Anything else is a protocol
/// mismatch, which is logged and ignored.
fn on_pc_link(rest: &str, s: &mut Shared) {
    match rest.trim() {
        "on" => s.pc_link_state = Some(true),
        "off" => s.pc_link_state = Some(false),
        "on_failed" | "off_failed" | "?" => {
            eprintln!("cfg: the guest reported pc_link {}.", rest.trim());
            s.pc_link_failures = s.pc_link_failures.saturating_add(1);
        }
        other => eprintln!("cfg: the pc_link response '{other}' is not recognised."),
    }
}

/// `mods off|pending|begin|end`. An end without its begin is the tail of a
/// report sent before the host connected.
fn on_mods(rest: &str, s: &mut Shared) {
    match rest {
        "off" => s.mods = ModsReport::Off,
        "pending" => s.mods = ModsReport::Pending,
        "begin" => s.mods_partial = Some(Partial::default()),
        "end" => match s.mods_partial.take() {
            Some(partial) => s.mods = partial.done(),
            None => eprintln!("cfg: ignoring `mods end` without a `mods begin`"),
        },
        other => eprintln!("cfg: the mods line '{other}' is not recognised."),
    }
}

/// `mod <name> …`, one line of the report between `mods begin` and
/// `mods end`.
fn on_mod(rest: &str, s: &mut Shared) {
    if !s.mods_partial.as_mut().is_some_and(|p| p.push(rest)) {
        eprintln!("cfg: ignoring the mod line '{rest}', which is outside a report.");
    }
}

// Journal lines are kept only for a mod whose journal
// `CfgClient::request_mod_log` requested, and only the answer to the latest
// request is kept.

/// `log_begin <name>`: a new answer starts, replacing the lines of an earlier
/// one.
fn on_log_begin(name: &str, s: &mut Shared) {
    if let Some((_, lines)) = s.mod_logs_partial.get_mut(name) {
        lines.clear();
    }
}

/// `log <name> <line>`.
fn on_log(rest: &str, s: &mut Shared) {
    let (name, text) = rest.split_once(' ').unwrap_or((rest, ""));
    if let Some((_, lines)) = s.mod_logs_partial.get_mut(name) {
        if lines.len() < MOD_LOG_LINES {
            lines.push(clip(text, MOD_LOG_LINE_MAX).to_string());
        }
    }
}

/// `log_end <name>`: the journal is complete once the answer to the latest
/// request ends.
fn on_log_end(name: &str, s: &mut Shared) {
    if let Some((due, _)) = s.mod_logs_partial.get_mut(name) {
        *due -= 1;
        if *due == 0 {
            let (_, lines) = s.mod_logs_partial.remove(name).unwrap_or_default();
            s.mod_logs.insert(name.to_string(), lines);
        }
    }
}

/// `s` truncated to at most `max` bytes, at a character boundary.
fn clip(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_journal_asked_twice_keeps_the_last_answer_whole() {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let feed = |lines: &[&str]| {
            for l in lines {
                handle_line(l, &shared);
            }
        };
        let log = |name: &str| shared.lock().unwrap().mod_logs.get(name).cloned();
        shared
            .lock()
            .unwrap()
            .mod_logs_partial
            .insert("a".into(), (1, Vec::new()));
        feed(&["log_begin a", "log a one"]);
        // Requested again while the first answer is still arriving.
        shared
            .lock()
            .unwrap()
            .mod_logs_partial
            .get_mut("a")
            .unwrap()
            .0 += 1;
        feed(&["log a two", "log_end a", "log b stray", "log_end b"]);
        assert_eq!(log("a"), None);
        assert_eq!(log("b"), None, "never asked for");
        feed(&[
            "log_begin a",
            "log a one",
            "log a",
            "log a two",
            "log_end a",
        ]);
        assert_eq!(log("a").unwrap(), ["one", "", "two"]);
    }

    /// Each guest message lands in its own field; a malformed or unknown one
    /// changes nothing but still proves the guest is reading.
    #[test]
    fn each_guest_message_lands_in_its_field() {
        let shared = Arc::new(Mutex::new(Shared::default()));
        for line in [
            "  ",
            "usb_state 1",
            "latency 3, 4,7",
            "latency 1,x,2",
            "param period_frames  512",
            "param lonely",
            "pc_link on",
            "pc_link off_failed",
            "pc_link ?",
            "pc_link bogus",
            "unknown 1",
        ] {
            handle_line(line, &shared);
        }
        let s = shared.lock().unwrap();
        assert!(s.guest_ready);
        assert_eq!(s.usb_state, Some(true));
        let l = s.latency.unwrap();
        assert_eq!((l.guest_ms, l.host_ms, l.total_ms), (3, 4, 7));
        assert_eq!(s.params.len(), 1);
        assert_eq!(s.params["period_frames"], "512");
        assert_eq!(s.pc_link_state, Some(true));
        assert_eq!(s.pc_link_failures, 2);
        assert_eq!(s.mods, ModsReport::Unknown);
    }

    #[test]
    fn pc_link_off_and_usb_state_zero_clear_their_state() {
        let shared = Arc::new(Mutex::new(Shared::default()));
        for line in ["pc_link on", "usb_state 1", "pc_link off", "usb_state 0"] {
            handle_line(line, &shared);
        }
        let s = shared.lock().unwrap();
        assert_eq!(s.pc_link_state, Some(false));
        assert_eq!(s.usb_state, Some(false));
    }

    /// `mods off` and `mods pending` replace the report; a report between
    /// `mods begin` and `mods end` replaces it when it ends.
    #[test]
    fn mods_lines_build_the_report() {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let mods = || shared.lock().unwrap().mods.clone();
        let feed = |lines: &[&str]| {
            for l in lines {
                handle_line(l, &shared);
            }
        };
        feed(&["mod a script 0 libs 1", "mods end", "mods bogus"]);
        assert_eq!(mods(), ModsReport::Unknown, "a tail without its begin");
        feed(&["mods pending"]);
        assert_eq!(mods(), ModsReport::Pending);
        feed(&[
            "mods begin",
            "mod a script 0 libs 1",
            "mod b script 0 integrity ok libs 0",
            "mod ../x script 0 libs 0",
        ]);
        assert_eq!(mods(), ModsReport::Pending, "the report has not ended");
        feed(&["mods end"]);
        assert_eq!(
            mods(),
            ModsReport::from_lines([
                "mods begin",
                "mod a script 0 libs 1",
                "mod b script 0 integrity ok libs 0",
                "mods end",
            ])
        );
        feed(&["mods off", "mods end"]);
        assert_eq!(mods(), ModsReport::Off);
        assert!(shared.lock().unwrap().mods_partial.is_none());
    }

    #[test]
    fn a_journal_line_is_clipped_and_a_journal_capped() {
        let shared = Arc::new(Mutex::new(Shared::default()));
        shared
            .lock()
            .unwrap()
            .mod_logs_partial
            .insert("a".into(), (1, Vec::new()));
        handle_line("log_begin a", &shared);
        let long = format!("log a {}", "é".repeat(MOD_LOG_LINE_MAX));
        handle_line(&long, &shared);
        for _ in 0..MOD_LOG_LINES {
            handle_line("log a more", &shared);
        }
        handle_line("log_end a", &shared);
        let s = shared.lock().unwrap();
        let log = &s.mod_logs["a"];
        assert_eq!(log.len(), MOD_LOG_LINES);
        assert_eq!(log[0], "é".repeat(MOD_LOG_LINE_MAX / 2));
        assert!(s.mod_logs_partial.is_empty());
    }
}
