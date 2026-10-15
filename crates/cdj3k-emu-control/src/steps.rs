//! The commands that `ctl` runs and that scenario steps run: requests, screen
//! checks, guest commands and watches, over one session with a slot.

mod screen;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::client::Client;
use crate::image::Image;
use screen::MATCH_POLL;

/// Why a command failed. `ctl` exits with status 2 for a usage error and 1
/// for any other failure.
pub(crate) enum Failure {
    Usage(String),
    Failed(String),
}

impl From<String> for Failure {
    fn from(e: String) -> Self {
        Failure::Failed(e)
    }
}

/// A connection to one slot, and where the commands print their results.
pub(crate) struct Session {
    pub host: String,
    pub slot: u32,
    /// Opened on the first request and kept, so a hold lasts until the
    /// session ends.
    pub client: Option<Client>,
    /// Where replies and events are printed.
    pub out: Box<dyn Write>,
    /// Where a failed `match` saves its capture and the picture of the
    /// difference, when it has no `--out`.
    pub artifacts: Option<PathBuf>,
    /// When set, a `match` whose reference file does not exist saves the
    /// screen as that reference.
    pub record: bool,
}

impl Session {
    pub fn new(host: &str, slot: u32) -> Self {
        Self {
            host: host.into(),
            slot,
            client: None,
            out: Box::new(std::io::stdout()),
            artifacts: None,
            record: false,
        }
    }

    fn print(&mut self, line: impl std::fmt::Display) {
        let _ = writeln!(self.out, "{line}");
    }

    /// The session's connection, opened on first use.
    pub fn connect(&mut self) -> Result<&mut Client, String> {
        if self.client.is_none() {
            let c = Client::connect(&self.host, self.slot).map_err(|e| e.to_string())?;
            self.client = Some(c);
        }
        Ok(self.client.as_mut().expect("connected above"))
    }

    /// Send `line` and return the reply, or its error when `ok` is false.
    pub fn request(&mut self, line: &str) -> Result<Value, String> {
        let reply = self.connect()?.request(line).map_err(|e| e.to_string())?;
        if reply["ok"] == true {
            Ok(reply)
        } else {
            let why = reply["error"].as_str().unwrap_or("the request failed");
            Err(format!("{line}: {why}"))
        }
    }

    /// Capture a screen as [`Self::capture`] does, and retry until `deadline`
    /// while the capture fails, for example right after a boot.
    fn capture_by(&mut self, request: &str, deadline: Instant) -> Result<Image, String> {
        loop {
            match self.capture(request) {
                Err(_) if Instant::now() < deadline => std::thread::sleep(MATCH_POLL),
                result => return result,
            }
        }
    }

    /// Send a capture request, such as `lcd main`, and decode its image.
    pub fn capture(&mut self, request: &str) -> Result<Image, String> {
        let reply = self.request(request)?;
        let png = reply["png"]
            .as_str()
            .ok_or_else(|| format!("{request}: the reply has no image"))?;
        Image::from_base64_png(png)
    }
}

/// Run one command, given as the words after the global options.
pub(crate) fn exec(s: &mut Session, words: &[String]) -> Result<(), Failure> {
    let rest = &words[1..];
    match words[0].as_str() {
        "sleep" => sleep(rest)?,
        "lcd" | "shot" => save_step(s, &words[0], rest)?,
        "match" => screen::match_screen(s, rest)?,
        "changes" => screen::watch_region(s, rest, true)?,
        "still" => screen::watch_region(s, rest, false)?,
        "watch" => watch(s, words)?,
        "diff" => screen::diff_files(s, rest)?,
        "sh" => sh(s, words)?,
        _ => {
            let reply = s.request(&with_absolute_path(words).join(" "))?;
            s.print(reply);
        }
    }
    Ok(())
}

fn sleep(rest: &[String]) -> Result<(), Failure> {
    let [ms] = rest else {
        return Err(Failure::Usage("sleep needs milliseconds".into()));
    };
    let ms = ms
        .parse()
        .map_err(|_| Failure::Usage(format!("{ms:?} is not milliseconds")))?;
    std::thread::sleep(Duration::from_millis(ms));
    Ok(())
}

/// `lcd main|jog <out.png>` or `shot <out.png>`.
fn save_step(s: &mut Session, verb: &str, rest: &[String]) -> Result<(), Failure> {
    match (verb, rest) {
        ("lcd", [screen, out]) => save(s, &format!("lcd {screen}"), Path::new(out))?,
        ("shot", [out]) => save(s, "shot", Path::new(out))?,
        ("lcd", _) => {
            return Err(Failure::Usage(
                "lcd needs main or jog and an output file".into(),
            ))
        }
        _ => return Err(Failure::Usage("shot needs an output file".into())),
    }
    Ok(())
}

fn save(s: &mut Session, request: &str, out: &Path) -> Result<(), String> {
    let img = s.capture(request)?;
    img.write_png(out)?;
    s.print(json!({"ok": true, "width": img.width, "height": img.height, "path": out}));
    Ok(())
}

/// `words` with the path of `media attach` or `mods push` made absolute when
/// it names a file here, because the window reads it from its own working
/// directory.
fn with_absolute_path(words: &[String]) -> Vec<String> {
    let mut words = words.to_vec();
    let takes_path = matches!(
        words.iter().map(String::as_str).take(2).collect::<Vec<_>>()[..],
        ["media", "attach"] | ["mods", "push"]
    );
    if takes_path && words.len() > 2 {
        let path = PathBuf::from(words[2..].join(" "));
        if let Ok(abs) = std::path::absolute(&path) {
            if abs.exists() {
                words.truncate(2);
                words.push(abs.display().to_string());
            }
        }
    }
    words
}

/// Run a guest command and print its output unchanged. It fails when the
/// command fails.
fn sh(s: &mut Session, words: &[String]) -> Result<(), Failure> {
    let line = words.join(" ");
    let client = s.connect()?;
    let reply = client.request(&line).map_err(|e| e.to_string())?;
    let _ = write!(s.out, "{}", reply["output"].as_str().unwrap_or(""));
    if reply["truncated"] == true {
        eprintln!("cdj3k-emu ctl: the output was cut at 1 MiB");
    }
    if reply["ok"] == true {
        Ok(())
    } else {
        let why = reply["error"].as_str().unwrap_or("the command failed");
        Err(Failure::Failed(format!("{line}: {why}")))
    }
}

/// Send `watch ...` and print its events until `--for` passes, `--count`
/// events arrived or the connection closes.
fn watch(s: &mut Session, words: &[String]) -> Result<(), Failure> {
    let mut request = Vec::new();
    let (mut until, mut count) = (None, None);
    let mut it = words.iter();
    while let Some(w) = it.next() {
        let mut number = || -> Result<u64, Failure> {
            let v = it
                .next()
                .ok_or_else(|| Failure::Usage(format!("{w} needs a value")))?;
            v.parse()
                .map_err(|_| Failure::Usage(format!("{w} takes a whole number, not {v:?}")))
        };
        match w.as_str() {
            "--for" => until = Some(Instant::now() + Duration::from_millis(number()?)),
            "--count" => count = Some(number()?),
            _ => request.push(w.as_str()),
        }
    }
    let reply = s.request(&request.join(" "))?;
    s.print(reply);
    let client = s.client.as_mut().expect("connected by the request");
    let mut seen = 0;
    while count.is_none_or(|n| seen < n) {
        let wait = match until {
            Some(t) if Instant::now() >= t => break,
            Some(t) => t - Instant::now(),
            None => Duration::from_secs(3600),
        };
        let event = client.next_event(wait).map_err(|e| e.to_string())?;
        if let Some(event) = event {
            let _ = writeln!(s.out, "{event}");
            seen += 1;
        }
    }
    Ok(())
}

/// The words of a script line, without its comment.
pub(crate) fn script_words(line: &str) -> Vec<String> {
    line.split('#')
        .next()
        .unwrap_or("")
        .split_whitespace()
        .map(String::from)
        .collect()
}
