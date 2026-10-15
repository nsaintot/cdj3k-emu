//! The screen commands: `match`, `diff`, `changes` and `still`, and the
//! record mode of `match`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::json;

use super::{Failure, Session};
use crate::command::DEFAULT_WAIT;
use crate::diff::{self, Options, Region};
use crate::image::Image;

/// How often `match` captures the screen again.
pub(super) const MATCH_POLL: Duration = Duration::from_millis(250);
/// How long the screen must stay unchanged before `match` records it as a
/// missing reference.
const RECORD_SETTLE: Duration = Duration::from_secs(3);

/// The diff options, and the words that are not options.
struct DiffArgs {
    positional: Vec<String>,
    opts: Options,
    max: u64,
    out: Option<PathBuf>,
    timeout: Duration,
    /// How long the screen must stay unchanged before record mode saves it.
    settle: Duration,
}

fn diff_args(words: &[String]) -> Result<DiffArgs, Failure> {
    let mut a = DiffArgs {
        positional: Vec::new(),
        opts: Options::default(),
        max: 0,
        out: None,
        timeout: DEFAULT_WAIT,
        settle: RECORD_SETTLE,
    };
    let mut it = words.iter();
    while let Some(w) = it.next() {
        if !w.starts_with("--") {
            a.positional.push(w.clone());
            continue;
        }
        let value = it
            .next()
            .ok_or_else(|| Failure::Usage(format!("{w} needs a value")))?;
        let number = |v: &str| {
            v.parse::<u64>()
                .map_err(|_| Failure::Usage(format!("{w} takes a whole number, not {v:?}")))
        };
        match w.as_str() {
            "--threshold" => a.opts.threshold = number(value)?.min(255) as u8,
            "--max" => a.max = number(value)?,
            "--timeout" => a.timeout = Duration::from_millis(number(value)?),
            "--settle" => a.settle = Duration::from_millis(number(value)?),
            "--mask" => a
                .opts
                .masks
                .push(Region::parse(value).map_err(Failure::Usage)?),
            "--region" => a.opts.region = Some(Region::parse(value).map_err(Failure::Usage)?),
            "--out" => a.out = Some(PathBuf::from(value)),
            _ => return Err(Failure::Usage(format!("{w} is not a diff option"))),
        }
    }
    Ok(a)
}

/// Compare two files and print the report. It fails when more than `--max`
/// pixels differ.
pub(super) fn diff_files(s: &mut Session, words: &[String]) -> Result<(), Failure> {
    let mut a = diff_args(words)?;
    let [reference, actual] = &a.positional[..] else {
        return Err(Failure::Usage(
            "diff needs a reference and an actual image".into(),
        ));
    };
    let (reference, actual) = (PathBuf::from(reference), PathBuf::from(actual));
    a.opts.masks.extend(diff::sidecar_masks(&reference)?);
    let r_img = Image::read_png(&reference)?;
    let a_img = Image::read_png(&actual)?;
    verdict(s, &r_img, &a_img, &a)
}

/// Capture a screen until it matches the reference or the timeout passes.
pub(super) fn match_screen(s: &mut Session, words: &[String]) -> Result<(), Failure> {
    let mut a = diff_args(words)?;
    let [screen, reference] = &a.positional[..] else {
        return Err(Failure::Usage(
            "match needs main or jog and a reference image".into(),
        ));
    };
    let request = format!("lcd {screen}");
    let reference = PathBuf::from(reference);
    if s.record && !reference.exists() {
        return record(s, &request, &reference, &a).map_err(Failure::Failed);
    }
    a.opts.masks.extend(diff::sidecar_masks(&reference)?);
    let r_img = Image::read_png(&reference)?;
    let deadline = Instant::now() + a.timeout;
    loop {
        let shot = s.capture_by(&request, deadline)?;
        let report = diff::compare(&r_img, &shot, &a.opts)?;
        if report.changed <= a.max {
            return verdict(s, &r_img, &shot, &a);
        }
        if Instant::now() >= deadline {
            keep_failed_match(s, &reference, &shot, &mut a)?;
            return verdict(s, &r_img, &shot, &a);
        }
        std::thread::sleep(MATCH_POLL);
    }
}

/// `changes` waits until a region of the screen differs from its first
/// capture, and fails at the timeout. `still` fails when the region changes
/// within `--for`.
pub(super) fn watch_region(
    s: &mut Session,
    words: &[String],
    want_change: bool,
) -> Result<(), Failure> {
    let (verb, flag) = if want_change {
        ("changes", "--timeout")
    } else {
        ("still", "--for")
    };
    let usage = || {
        Failure::Usage(format!(
            "{verb} needs main or jog, a region x0,y0,x1,y1 and {flag} MS"
        ))
    };
    let (screen, region, ms) = match words {
        [screen, region] => (screen, region, DEFAULT_WAIT.as_millis() as u64),
        [screen, region, f, ms] if f == flag => (screen, region, ms.parse().map_err(|_| usage())?),
        _ => return Err(usage()),
    };
    let opts = Options {
        region: Some(Region::parse(region).map_err(Failure::Usage)?),
        ..Options::default()
    };
    let request = format!("lcd {screen}");
    let deadline = Instant::now() + Duration::from_millis(ms);
    let first = s.capture_by(&request, deadline)?;
    loop {
        std::thread::sleep(MATCH_POLL);
        let shot = s.capture(&request)?;
        let report = diff::compare(&first, &shot, &opts)?;
        if report.changed > 0 {
            let mut line = report.to_json();
            line["ok"] = want_change.into();
            s.print(line);
            return if want_change {
                Ok(())
            } else {
                Err(Failure::Failed(format!(
                    "{} pixels of the region changed",
                    report.changed
                )))
            };
        }
        if Instant::now() >= deadline {
            s.print(json!({"ok": !want_change, "changed": 0}));
            return if want_change {
                Err(Failure::Failed(format!(
                    "the region did not change in {ms} ms"
                )))
            } else {
                Ok(())
            };
        }
    }
}

/// Save the screen as the reference once the compared part of it has not
/// changed for `--settle`, or when `--timeout` has passed.
fn record(s: &mut Session, request: &str, reference: &Path, a: &DiffArgs) -> Result<(), String> {
    let deadline = Instant::now() + a.timeout;
    let mut shot = s.capture_by(request, deadline)?;
    let mut since = Instant::now();
    while since.elapsed() < a.settle && Instant::now() < deadline {
        std::thread::sleep(MATCH_POLL);
        let next = s.capture(request)?;
        if diff::compare(&shot, &next, &a.opts)?.changed > 0 {
            since = Instant::now();
        }
        shot = next;
    }
    if let Some(dir) = reference.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    shot.write_png(reference)?;
    s.print(json!({"ok": true, "recorded": reference}));
    Ok(())
}

/// Save the capture of a failed `match` in the session's artifacts, and the
/// picture of the difference unless `--out` names a file for it.
fn keep_failed_match(
    s: &Session,
    reference: &Path,
    shot: &Image,
    a: &mut DiffArgs,
) -> Result<(), String> {
    let Some(dir) = &s.artifacts else {
        return Ok(());
    };
    let stem = reference
        .file_stem()
        .map_or("match".into(), |n| n.to_string_lossy());
    let path = |kind: &str| {
        let mut n = 1;
        loop {
            let p = dir.join(format!("{stem}-{kind}-{n}.png"));
            if !p.exists() {
                return p;
            }
            n += 1;
        }
    };
    shot.write_png(&path("actual"))?;
    if a.out.is_none() {
        a.out = Some(path("diff"));
    }
    Ok(())
}

/// Print the comparison and write the picture of the difference. It fails
/// when more than `max` pixels differ.
fn verdict(
    s: &mut Session,
    reference: &Image,
    actual: &Image,
    a: &DiffArgs,
) -> Result<(), Failure> {
    let report = diff::compare(reference, actual, &a.opts)?;
    if let Some(out) = &a.out {
        diff::diff_image(reference, actual, &a.opts)?.write_png(out)?;
    }
    let matched = report.changed <= a.max;
    let mut line = report.to_json();
    line["ok"] = matched.into();
    s.print(line);
    if matched {
        Ok(())
    } else {
        Err(Failure::Failed(format!(
            "{} of {} pixels differ, more than the {} allowed",
            report.changed, report.compared, a.max
        )))
    }
}
