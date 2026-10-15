//! The outcome of an `e2e` run, as `report.json` and as JUnit XML.

use std::fmt::Write as _;
use std::path::Path;

use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Status {
    Passed,
    /// The step at `line` failed, or the setup failed when `line` is `None`.
    Failed {
        line: Option<usize>,
        error: String,
    },
    /// The scenario did not run, for the given reason.
    Skipped(String),
}

#[derive(Clone, Debug)]
pub(crate) struct Outcome {
    pub name: String,
    pub file: String,
    pub seconds: f64,
    pub status: Status,
}

impl Outcome {
    /// One line for the terminal.
    pub fn summary(&self) -> String {
        match &self.status {
            Status::Passed => format!("PASS {} ({:.1} s)", self.name, self.seconds),
            Status::Failed { line, error } => format!(
                "FAIL {} ({:.1} s): {}{error}",
                self.name,
                self.seconds,
                line.map_or(String::new(), |n| format!("line {n}: "))
            ),
            Status::Skipped(why) => format!("SKIP {}: {why}", self.name),
        }
    }

    fn to_json(&self) -> Value {
        let mut v = json!({
            "name": self.name,
            "file": self.file,
            "seconds": self.seconds,
        });
        match &self.status {
            Status::Passed => v["status"] = "passed".into(),
            Status::Failed { line, error } => {
                v["status"] = "failed".into();
                v["line"] = json!(line);
                v["error"] = error.as_str().into();
            }
            Status::Skipped(why) => {
                v["status"] = "skipped".into();
                v["reason"] = why.as_str().into();
            }
        }
        v
    }
}

/// The number of failed scenarios.
pub(crate) fn failed(outcomes: &[Outcome]) -> usize {
    count(outcomes, |s| matches!(s, Status::Failed { .. }))
}

fn count(outcomes: &[Outcome], f: impl Fn(&Status) -> bool) -> usize {
    outcomes.iter().filter(|o| f(&o.status)).count()
}

/// The run as JSON: each scenario, the slots it used, and the totals.
pub(crate) fn json(outcomes: &[Outcome], slots: Value) -> Value {
    json!({
        "scenarios": outcomes.iter().map(Outcome::to_json).collect::<Vec<_>>(),
        "slots": slots,
        "passed": count(outcomes, |s| *s == Status::Passed),
        "failed": failed(outcomes),
        "skipped": count(outcomes, |s| matches!(s, Status::Skipped(_))),
    })
}

/// The run as a JUnit test suite, one test case per scenario.
pub(crate) fn junit(outcomes: &[Outcome]) -> String {
    let total: f64 = outcomes.iter().map(|o| o.seconds).sum();
    let mut x = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        x,
        "<testsuite name=\"cdj3k-emu e2e\" tests=\"{}\" failures=\"{}\" skipped=\"{}\" time=\"{total:.3}\">",
        outcomes.len(),
        failed(outcomes),
        count(outcomes, |s| matches!(s, Status::Skipped(_))),
    );
    for o in outcomes {
        let _ = write!(
            x,
            "  <testcase classname=\"{}\" name=\"{}\" time=\"{:.3}\"",
            escape(&o.file),
            escape(&o.name),
            o.seconds
        );
        match &o.status {
            Status::Passed => x.push_str("/>\n"),
            Status::Failed { line, error } => {
                let message = match line {
                    Some(n) => format!("line {n}: {error}"),
                    None => error.clone(),
                };
                let _ = writeln!(
                    x,
                    ">\n    <failure message=\"{}\"/>\n  </testcase>",
                    escape(&message)
                );
            }
            Status::Skipped(why) => {
                let _ = writeln!(
                    x,
                    ">\n    <skipped message=\"{}\"/>\n  </testcase>",
                    escape(why)
                );
            }
        }
    }
    x.push_str("</testsuite>\n");
    x
}

/// Escape `s` for an XML attribute, and drop other control characters.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Write `report.json` and `junit.xml` into `dir`.
pub(crate) fn write(dir: &Path, outcomes: &[Outcome], slots: Value) -> Result<(), String> {
    let json = serde_json::to_string_pretty(&json(outcomes, slots)).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("report.json"), json + "\n").map_err(|e| e.to_string())?;
    std::fs::write(dir.join("junit.xml"), junit(outcomes)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcomes() -> Vec<Outcome> {
        let o = |name: &str, status| Outcome {
            name: name.into(),
            file: format!("suite/{name}.ctl"),
            seconds: 1.5,
            status,
        };
        vec![
            o("boot", Status::Passed),
            o(
                "browse",
                Status::Failed {
                    line: Some(7),
                    error: "match: 12 of 100 pixels differ & more".into(),
                },
            ),
            o("link", Status::Skipped("needs 2 slots".into())),
        ]
    }

    #[test]
    fn the_json_counts_each_status() {
        let v = json(&outcomes(), json!([]));
        assert_eq!(
            (
                v["passed"].clone(),
                v["failed"].clone(),
                v["skipped"].clone()
            ),
            (json!(1), json!(1), json!(1))
        );
        assert_eq!(v["scenarios"][1]["line"], 7);
        assert_eq!(v["scenarios"][2]["reason"], "needs 2 slots");
    }

    #[test]
    fn junit_escapes_and_counts() {
        let x = junit(&outcomes());
        assert!(
            x.contains("tests=\"3\" failures=\"1\" skipped=\"1\""),
            "{x}"
        );
        assert!(
            x.contains("<failure message=\"line 7: match: 12 of 100 pixels differ &amp; more\"/>"),
            "{x}"
        );
        assert!(
            x.contains("<testcase classname=\"suite/boot.ctl\" name=\"boot\" time=\"1.500\"/>"),
            "{x}"
        );
    }

    #[test]
    fn the_summary_names_the_failing_line() {
        let o = &outcomes()[1];
        assert_eq!(
            o.summary(),
            "FAIL browse (1.5 s): line 7: match: 12 of 100 pixels differ & more"
        );
    }
}
