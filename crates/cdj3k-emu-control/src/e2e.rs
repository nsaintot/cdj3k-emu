//! `cdj3k-emu ctl e2e`: run scenario files against slots and write a report.
//!
//! ```text
//! cdj3k-emu ctl e2e [--slot N]... [--launch] [--record] [--report DIR] [--boot-wait MS]
//!                   <file|dir>...
//! ```
//!
//! The run drives the ephemeral session of each slot it names (slot N+4).
//! Each scenario gets its settings, a reset and its USB image, then runs its
//! steps as `ctl run` does. A failure saves the screens and the guest's
//! journal next to the steps' output. At the end the sessions that `--launch`
//! started close, which deletes them. `docs/e2e.md` describes the scenario
//! format.

mod report;
mod scenario;

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use crate::steps::{self, Failure, Session};
use report::{Outcome, Status};
use scenario::Scenario;

/// How long a slot that `--launch` started may take to open its port.
const LAUNCH_WAIT: Duration = Duration::from_secs(60);
/// How long a slot that `--launch` started may take to close after `quit`.
const QUIT_WAIT: Duration = Duration::from_secs(30);
/// The default for `--boot-wait`.
const BOOT_WAIT: Duration = Duration::from_secs(600);
/// The journal lines a failure saves.
const JOURNAL_LINES: u32 = 500;

/// The options of `ctl e2e`.
struct Options {
    slots: Vec<u32>,
    launch: bool,
    record: bool,
    report: PathBuf,
    boot_wait: Duration,
    paths: Vec<PathBuf>,
}

fn options(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        slots: Vec::new(),
        launch: false,
        record: false,
        report: PathBuf::from("e2e-report"),
        boot_wait: BOOT_WAIT,
        paths: Vec::new(),
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || {
            it.next()
                .ok_or_else(|| format!("{arg} needs a value"))
                .cloned()
        };
        match arg.as_str() {
            "--slot" => {
                let v = value()?;
                o.slots.push(
                    v.parse()
                        .map_err(|_| format!("{v:?} is not a slot number"))?,
                );
            }
            "--launch" => o.launch = true,
            "--record" => o.record = true,
            "--report" => o.report = PathBuf::from(value()?),
            "--boot-wait" => {
                let v = value()?;
                let ms = v
                    .parse()
                    .map_err(|_| format!("{v:?} is not milliseconds"))?;
                o.boot_wait = Duration::from_millis(ms);
            }
            flag if flag.starts_with("--") => return Err(format!("{flag} is not an e2e option")),
            path => o.paths.push(PathBuf::from(path)),
        }
    }
    if o.slots.is_empty() {
        o.slots.push(1);
    }
    if o.paths.is_empty() {
        return Err("e2e needs scenario files or folders".into());
    }
    Ok(o)
}

/// Run `ctl e2e` with the arguments after `e2e`. It returns the exit status,
/// or an error for a usage error, such as a bad option or scenario.
pub(crate) fn run(host: &str, args: &[String]) -> Result<i32, String> {
    let o = options(args)?;
    let scenarios = load(&o.paths)?;
    Ok(
        match Run::start(host, &o).and_then(|mut run| run.all(&scenarios, &o)) {
            Ok(0) => 0,
            Ok(_) => 1,
            Err(e) => {
                eprintln!("cdj3k-emu ctl e2e: {e}");
                1
            }
        },
    )
}

fn load(paths: &[PathBuf]) -> Result<Vec<Scenario>, String> {
    let files = scenario::collect(paths)?;
    if files.is_empty() {
        return Err("no .ctl scenario was found".into());
    }
    files.iter().map(|f| Scenario::read(f)).collect()
}

/// A slot the run drives, and what it had before the run.
struct Slot {
    /// The slot the run names; its session runs as the ephemeral slot
    /// `base + EPHEMERAL_SLOT_OFFSET`, which `session` talks to.
    base: u32,
    session: Session,
    model: String,
    /// The firmware release, such as `3.22`, or empty when it is unknown.
    release: String,
    /// Whether the slot's QEMU runs on a hardware accelerator.
    accel: bool,
    /// The process `--launch` started, which the run closes at the end.
    child: Option<Launched>,
    launched: bool,
    /// The settings as the run found them, and as they are now.
    original: Map<String, Value>,
    current: Map<String, Value>,
    /// The current scenario's copy of its USB image.
    media_copy: Option<PathBuf>,
}

impl Slot {
    fn open(host: &str, base: u32, launch: bool, report: &Path) -> Result<Self, String> {
        let mut session = Session::new(host, base + crate::EPHEMERAL_SLOT_OFFSET);
        session.out = Box::new(std::io::sink());
        let child = match session.request("hello") {
            Ok(_) => None,
            Err(_) if launch => Some(Launched(launch_slot(&mut session, base, report)?)),
            Err(e) => {
                return Err(format!(
                    "slot {base}'s ephemeral session: {e}; start it with \
                     `cdj3k-emu --instance {base} --dev --ephemeral`, or pass --launch"
                ))
            }
        };
        let hello = session.request("hello")?;
        let settings = session.request("setting")?;
        let mut original = settings.as_object().cloned().unwrap_or_default();
        original.remove("ok");
        Ok(Self {
            base,
            model: hello["model"].as_str().unwrap_or("").into(),
            release: hello["release"].as_str().unwrap_or("").into(),
            accel: hello["accel"].is_string(),
            launched: child.is_some(),
            child,
            current: original.clone(),
            original,
            media_copy: None,
            session,
        })
    }

    /// Attach a copy of `image` as the slot's USB device, so that the
    /// scenario's writes do not reach the original. `k` numbers the copy. With
    /// no image, eject whatever the slot has attached.
    fn attach_media(&mut self, image: Option<&Path>, dir: &Path, k: usize) -> Result<(), String> {
        self.media_copy = None;
        let Some(image) = image else {
            if self.session.request("media")?["attached"] != "none" {
                self.session.request("media eject")?;
            }
            return Ok(());
        };
        let copy = dir.join(format!("usb-{k}.img"));
        std::fs::copy(image, &copy).map_err(|e| format!("{}: {e}", image.display()))?;
        self.session
            .request(&format!("media attach {}", copy.display()))?;
        self.media_copy = Some(copy);
        Ok(())
    }

    /// `words` with `{model}`, `{release}`, `{slot}` and `{media}` replaced by
    /// this slot's own, as in `match main refs/{model}-{release}/browse.png`.
    /// `{media}` is the slot's copy of the scenario's USB image.
    fn substitute(&self, words: &[String]) -> Vec<String> {
        words
            .iter()
            .map(|w| {
                w.replace("{model}", &self.model)
                    .replace("{release}", &self.release)
                    .replace("{slot}", &self.base.to_string())
                    .replace(
                        "{media}",
                        &self
                            .media_copy
                            .as_ref()
                            .map_or(String::new(), |p| p.display().to_string()),
                    )
            })
            .collect()
    }

    /// Give the slot `settings` over the run's original ones. It returns
    /// whether a change restarts the emulation.
    fn apply_settings(&mut self, settings: &[(String, String)]) -> Result<bool, String> {
        let mut wanted: Vec<(String, String)> = self
            .original
            .iter()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, v)| (k.clone(), setting_word(v)))
            .collect();
        for (name, value) in settings {
            match wanted.iter_mut().find(|(k, _)| k == name) {
                Some(w) => w.1 = value.clone(),
                None => return Err(format!("{name:?} is not a setting")),
            }
        }
        let mut restarts = false;
        for (name, value) in wanted {
            if self.current.get(&name).map(setting_word).as_deref() == Some(value.as_str()) {
                continue;
            }
            let reply = self.session.request(&format!("setting {name} {value}"))?;
            restarts |= reply["restarts"] == true;
            self.current.insert(name, value.into());
        }
        Ok(restarts)
    }

    /// Let go of what the run holds, and close the slot when the run started
    /// it. The ephemeral slot goes with its session, so nothing else needs
    /// putting back.
    fn finish(&mut self) -> Result<(), String> {
        let released = self.session.request("release all").map(drop);
        let closed = match self.child.take() {
            Some(child) => child.close(&mut self.session),
            None => Ok(()),
        };
        released
            .and(closed)
            .map_err(|e| format!("slot {}: {e}", self.base))
    }

    fn to_json(&self) -> Value {
        json!({
            "slot": self.base,
            "runs_as": self.session.slot,
            "model": self.model,
            "accel": self.accel,
            "launched": self.launched,
        })
    }
}

/// A slot process that `--launch` started. Dropping it kills the process, so
/// a run that stops early leaves no hidden slot behind.
struct Launched(Child);

impl Launched {
    /// Ask the slot to quit, and kill it when it has not closed in time.
    fn close(mut self, session: &mut Session) -> Result<(), String> {
        let _ = session.request("quit");
        let deadline = Instant::now() + QUIT_WAIT;
        while self.0.try_wait().ok().flatten().is_none() {
            if Instant::now() >= deadline {
                return Err("the slot did not close; it was killed".into());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Ok(())
    }
}

impl Drop for Launched {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

/// A setting's value as `setting` takes it.
fn setting_word(v: &Value) -> String {
    match v {
        Value::Bool(true) => "on".into(),
        Value::Bool(false) => "off".into(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Start an ephemeral session of slot `base`, headless and in Developer Mode,
/// and wait for its port.
fn launch_slot(session: &mut Session, base: u32, report: &Path) -> Result<Child, String> {
    let slot = base;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let log = File::create(report.join(format!("slot-{slot}.log"))).map_err(|e| e.to_string())?;
    let mut child = Command::new(exe)
        .args([
            "--instance",
            &slot.to_string(),
            "--dev",
            "--headless",
            "--ephemeral",
        ])
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log)
        .spawn()
        .map_err(|e| format!("slot {slot} did not start: {e}"))?;
    let deadline = Instant::now() + LAUNCH_WAIT;
    loop {
        session.client = None;
        if session.request("hello").is_ok() {
            return Ok(child);
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "slot {slot} exited with {status}; see slot-{slot}.log"
            ));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return Err(format!("slot {slot} did not open its port in time"));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// The slots of an `e2e` run and the folder of its report.
struct Run {
    slots: Vec<Slot>,
    report: PathBuf,
}

impl Run {
    fn start(host: &str, o: &Options) -> Result<Self, String> {
        std::fs::create_dir_all(&o.report).map_err(|e| format!("{}: {e}", o.report.display()))?;
        let report = std::fs::canonicalize(&o.report).map_err(|e| e.to_string())?;
        let mut run = Run {
            slots: Vec::new(),
            report,
        };
        for &n in &o.slots {
            match Slot::open(host, n, o.launch, &run.report) {
                Ok(mut slot) => {
                    slot.session.record = o.record;
                    run.slots.push(slot);
                }
                Err(e) => {
                    for slot in &mut run.slots {
                        let _ = slot.finish();
                    }
                    return Err(e);
                }
            }
        }
        Ok(run)
    }

    /// Run every scenario, finish the slots and write the report. It
    /// returns the number of failed scenarios.
    fn all(&mut self, scenarios: &[Scenario], o: &Options) -> Result<usize, String> {
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        let mut outcomes = Vec::new();
        for (i, s) in scenarios.iter().enumerate() {
            // `prepare` changes into each scenario's folder. Go back first,
            // because the scenario paths are relative to the starting folder.
            std::env::set_current_dir(&cwd).map_err(|e| e.to_string())?;
            let outcome = self.scenario(i + 1, s, o.boot_wait);
            println!("{}", outcome.summary());
            outcomes.push(outcome);
        }
        let _ = std::env::set_current_dir(&cwd);
        let mut finished = Ok(());
        for slot in &mut self.slots {
            finished = finished.and(slot.finish());
        }
        let slots = self.slots.iter().map(Slot::to_json).collect();
        report::write(&self.report, &outcomes, slots)?;
        println!(
            "{} scenarios: {} failed; report in {}",
            outcomes.len(),
            report::failed(&outcomes),
            self.report.display()
        );
        finished.map(|()| report::failed(&outcomes))
    }

    fn scenario(&mut self, index: usize, s: &Scenario, boot_wait: Duration) -> Outcome {
        let started = Instant::now();
        let status = match self.skip_reason(s) {
            Some(why) => Status::Skipped(why),
            None => self.run_scenario(index, s, boot_wait),
        };
        Outcome {
            name: s.name.clone(),
            file: s.path.display().to_string(),
            seconds: started.elapsed().as_secs_f64(),
            status,
        }
    }

    fn skip_reason(&self, s: &Scenario) -> Option<String> {
        if s.slots > self.slots.len() {
            return Some(format!(
                "needs {} slots; the run has {}",
                s.slots,
                self.slots.len()
            ));
        }
        if s.needs_accel && self.slots[..s.slots].iter().any(|slot| !slot.accel) {
            return Some("needs hardware acceleration; the slot runs under TCG".into());
        }
        let model = self.slots[..s.slots]
            .iter()
            .map(|slot| &slot.model)
            .find(|m| !s.models.is_empty() && !s.models.contains(m))?;
        Some(format!("runs on {}, not {model}", s.models.join(", ")))
    }

    fn run_scenario(&mut self, index: usize, s: &Scenario, boot_wait: Duration) -> Status {
        let dir = self
            .report
            .join(format!("{index:02}-{}", file_name(&s.name)));
        let status = match self.prepare(s, &dir, boot_wait) {
            Err(error) => Status::Failed { line: None, error },
            Ok(()) => self.run_steps(s),
        };
        let slots = &mut self.slots[..s.slots];
        if matches!(status, Status::Failed { .. }) {
            for (k, slot) in slots.iter_mut().enumerate() {
                save_evidence(&mut slot.session, &dir, k + 1);
            }
        }
        for slot in slots.iter_mut() {
            let _ = slot.session.request("release all");
            let _ = slot.session.request("unwatch all");
            if s.media.is_some() {
                let _ = slot.session.request("media eject");
            }
            slot.session.out = Box::new(std::io::sink());
            slot.session.artifacts = None;
        }
        remove_media_copies(&dir);
        status
    }

    /// Create the scenario's folder, set its slots up and reset them.
    fn prepare(&mut self, s: &Scenario, dir: &Path, boot_wait: Duration) -> Result<(), String> {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let log = File::create(dir.join("steps.log")).map_err(|e| e.to_string())?;
        if let Some(parent) = s.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::env::set_current_dir(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let mut restarting = Vec::new();
        for slot in &mut self.slots[..s.slots] {
            slot.session.out = Box::new(log.try_clone().map_err(|e| e.to_string())?);
            slot.session.artifacts = Some(dir.to_path_buf());
            let restarted = slot.apply_settings(&s.settings)?;
            if s.reset {
                slot.session.request("reset")?;
            }
            restarting.push(s.reset || restarted);
        }
        let wait_boot = format!("wait boot {}", boot_wait.as_millis());
        for (k, slot) in self.slots[..s.slots].iter_mut().enumerate() {
            if restarting[k] {
                slot.session.request(&wait_boot)?;
            }
            slot.attach_media(s.media.as_deref(), dir, k + 1)?;
        }
        Ok(())
    }

    fn run_steps(&mut self, s: &Scenario) -> Status {
        let mut current = 0;
        for (line, words) in &s.steps {
            if words[0] == "slot" {
                current = words[1].parse::<usize>().map_or(0, |k| k - 1);
                continue;
            }
            let slot = &mut self.slots[current];
            let words = slot.substitute(words);
            let session = &mut slot.session;
            let _ = writeln!(session.out, "## line {line}: {}", words.join(" "));
            if let Err(Failure::Usage(e) | Failure::Failed(e)) = steps::exec(session, &words) {
                let _ = writeln!(session.out, "## failed: {e}");
                return Status::Failed {
                    line: Some(*line),
                    error: e,
                };
            }
        }
        Status::Passed
    }
}

fn remove_media_copies(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("usb-") && name.ends_with(".img") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Save the screens and the guest's journal of slot `k` after a failure. A
/// capture that fails is left out.
fn save_evidence(session: &mut Session, dir: &Path, k: usize) {
    for screen in ["main", "jog"] {
        if let Ok(img) = session.capture(&format!("lcd {screen}")) {
            let _ = img.write_png(&dir.join(format!("slot{k}-{screen}.png")));
        }
    }
    let journal = format!("sh journalctl -b --no-pager -n {JOURNAL_LINES}");
    if let Ok(client) = session.connect() {
        if let Ok(reply) = client.request(&journal) {
            let text = reply["output"].as_str().unwrap_or("");
            let _ = std::fs::write(dir.join(format!("slot{k}-journal.txt")), text);
        }
    }
}

/// `name` with every character other than a letter, a digit, `-` or `_`
/// replaced by `-`.
fn file_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn options_take_slots_and_paths() {
        let o = options(&args(
            "--slot 1 --slot 2 --launch --report out --boot-wait 5000 a.ctl suite",
        ))
        .unwrap();
        assert_eq!(o.slots, [1, 2]);
        assert!(o.launch);
        assert_eq!(o.report, PathBuf::from("out"));
        assert_eq!(o.boot_wait, Duration::from_millis(5000));
        assert_eq!(o.paths, [PathBuf::from("a.ctl"), PathBuf::from("suite")]);
        assert_eq!(options(&args("a.ctl")).unwrap().slots, [1]);
        assert!(options(&args("--slot")).is_err());
        assert!(options(&args("--bogus a.ctl")).is_err());
        assert!(options(&args("--slot 1")).is_err());
    }

    #[test]
    fn settings_are_words_setting_takes() {
        assert_eq!(setting_word(&json!(true)), "on");
        assert_eq!(setting_word(&json!(false)), "off");
        assert_eq!(setting_word(&json!("en0")), "en0");
    }

    #[test]
    fn names_become_file_names() {
        assert_eq!(file_name("browse: USB/1"), "browse--USB-1");
    }
}
