//! Scenario files: `ctl run` scripts with a header of `#@` directives.
//!
//! ```text
//! # Open the USB library from SOURCE.
//! #@ models cdj3k cdj3kx
//! #@ media ../fixtures/e2e-usb.img
//! #@ setting alc off
//! press SOURCE
//! match main refs/source.png --timeout 5000
//! ```
//!
//! `ctl run` treats a `#@` line as a comment, so it can run a scenario file
//! directly. It skips the steps of `#@ include` files.

use std::path::{Path, PathBuf};

use crate::steps::script_words;

#[derive(Debug, PartialEq)]
pub(crate) struct Scenario {
    pub path: PathBuf,
    pub name: String,
    /// The model slugs it runs on; empty for every model.
    pub models: Vec<String>,
    /// How many slots it drives. `slot <k>` switches between them.
    pub slots: usize,
    /// The settings each of its slots gets before the reset.
    pub settings: Vec<(String, String)>,
    /// A USB image that each slot gets a copy of after the reset.
    pub media: Option<PathBuf>,
    /// Whether the slots are reset before the steps: their eMMC writes are
    /// discarded and the emulation restarts.
    pub reset: bool,
    /// Whether the scenario needs a hardware accelerator. It is skipped on a
    /// slot that runs under TCG.
    pub needs_accel: bool,
    /// The steps with their line numbers, from 1.
    pub steps: Vec<(usize, Vec<String>)>,
}

impl Scenario {
    pub fn read(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(path, &text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn parse(path: &Path, text: &str) -> Result<Self, String> {
        let mut s = Scenario {
            path: path.to_path_buf(),
            name: path
                .file_stem()
                .map_or("scenario".into(), |n| n.to_string_lossy().into_owned()),
            models: Vec::new(),
            slots: 1,
            settings: Vec::new(),
            media: None,
            reset: true,
            needs_accel: false,
            steps: Vec::new(),
        };
        for (n, line) in text.lines().enumerate() {
            if let Some(directive) = line.trim_start().strip_prefix("#@") {
                let result = match directive.trim().strip_prefix("include ") {
                    Some(file) => s.include(file.trim(), n + 1),
                    None => s.directive(directive),
                };
                result.map_err(|e| format!("line {}: {e}", n + 1))?;
                continue;
            }
            let words = script_words(line);
            if let Some(first) = words.first() {
                if first == "slot" {
                    s.check_slot(&words)
                        .map_err(|e| format!("line {}: {e}", n + 1))?;
                }
                s.steps.push((n + 1, words));
            }
        }
        Ok(s)
    }

    fn directive(&mut self, line: &str) -> Result<(), String> {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words[..] {
            ["name", ..] => self.name = words[1..].join(" "),
            ["models", ..] => self.models = words[1..].iter().map(|m| m.to_string()).collect(),
            ["slots", n] => {
                self.slots = n
                    .parse()
                    .ok()
                    .filter(|n| (1..=4).contains(n))
                    .ok_or_else(|| format!("slots takes 1 to 4, not {n:?}"))?;
            }
            ["setting", name, value] => self.settings.push((name.into(), value.into())),
            ["media", ..] if words.len() > 1 => {
                self.media = Some(PathBuf::from(words[1..].join(" ")));
            }
            ["reset", "on"] => self.reset = true,
            ["reset", "off"] => self.reset = false,
            ["needs", "hw-accel"] => self.needs_accel = true,
            _ => {
                return Err(format!(
                "{:?} is not a directive: name, models, slots, setting, media, reset, needs hw-accel or include",
                line.trim()
            ))
            }
        }
        Ok(())
    }

    /// Add the steps of `file`, relative to the scenario's folder. They take
    /// the line number `at` of the `include`.
    fn include(&mut self, file: &str, at: usize) -> Result<(), String> {
        let path = self.path.parent().unwrap_or(Path::new("")).join(file);
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        for line in text.lines() {
            if line.trim_start().starts_with("#@") {
                return Err(format!(
                    "{file} has a directive; an included file holds only steps"
                ));
            }
            let words = script_words(line);
            if words.first().is_some_and(|w| w == "slot") {
                self.check_slot(&words)?;
            }
            if !words.is_empty() {
                self.steps.push((at, words));
            }
        }
        Ok(())
    }

    /// Check that a `slot <k>` step names one of the scenario's slots.
    fn check_slot(&self, words: &[String]) -> Result<(), String> {
        match words {
            [_, k]
                if k.parse::<usize>()
                    .is_ok_and(|k| (1..=self.slots).contains(&k)) =>
            {
                Ok(())
            }
            _ => Err(format!(
                "slot takes a number from 1 to {}; set `#@ slots` first",
                self.slots
            )),
        }
    }
}

/// The scenario files that `paths` name: each file, and the `.ctl` files
/// under each folder, in name order.
pub(crate) fn collect(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            let mut found = Vec::new();
            walk(path, &mut found)?;
            found.sort();
            files.extend(found);
        } else if path.is_file() {
            files.push(path.clone());
        } else {
            return Err(format!("{} is not a file or a folder", path.display()));
        }
    }
    Ok(files)
}

fn walk(dir: &Path, found: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, found)?;
        } else if path.extension().is_some_and(|e| e == "ctl") {
            found.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Scenario, String> {
        Scenario::parse(Path::new("suite/browse.ctl"), text)
    }

    #[test]
    fn a_header_sets_up_the_scenario() {
        let s = parse(
            "# Open the library.\n\
             #@ models cdj3k cdj3kx\n\
             #@ slots 2\n\
             #@ setting alc off\n\
             #@ media ../fixtures/My USB.img\n\
             #@ reset off\n\
             #@ needs hw-accel\n\
             \n\
             press SOURCE  # the source screen\n\
             slot 2\n\
             wait boot\n",
        )
        .unwrap();
        assert_eq!(s.name, "browse");
        assert_eq!(s.models, ["cdj3k", "cdj3kx"]);
        assert_eq!(s.slots, 2);
        assert_eq!(s.settings, [("alc".to_string(), "off".to_string())]);
        assert_eq!(s.media, Some(PathBuf::from("../fixtures/My USB.img")));
        assert!(!s.reset);
        assert!(s.needs_accel);
        let steps: Vec<_> = s.steps.iter().map(|(n, w)| (*n, w.join(" "))).collect();
        assert_eq!(
            steps,
            [
                (9, "press SOURCE".to_string()),
                (10, "slot 2".to_string()),
                (11, "wait boot".to_string())
            ]
        );
    }

    #[test]
    fn an_include_adds_its_steps_at_its_line() {
        let dir = std::env::temp_dir().join(format!("cdj3k-e2e-include-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(
            dir.join("lib/open.inc"),
            "# Open it.\npress SOURCE\ntouch 0.3 0.19\n",
        )
        .unwrap();
        std::fs::write(dir.join("lib/bad.inc"), "#@ slots 2\n").unwrap();
        let path = dir.join("browse.ctl");
        let s = Scenario::parse(&path, "wait boot\n#@ include lib/open.inc\npress PLAY\n").unwrap();
        let steps: Vec<_> = s.steps.iter().map(|(n, w)| (*n, w.join(" "))).collect();
        assert_eq!(
            steps,
            [
                (1, "wait boot".to_string()),
                (2, "press SOURCE".to_string()),
                (2, "touch 0.3 0.19".to_string()),
                (3, "press PLAY".to_string())
            ]
        );
        assert!(Scenario::parse(&path, "#@ include lib/bad.inc").is_err());
        assert!(Scenario::parse(&path, "#@ include lib/none.inc").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_plain_script_is_a_scenario() {
        let s = parse("press PLAY\n").unwrap();
        assert!(s.reset);
        assert_eq!(s.slots, 1);
        assert!(s.models.is_empty() && s.settings.is_empty() && s.media.is_none());
    }

    #[test]
    fn bad_headers_and_slots_name_their_line() {
        assert_eq!(
            parse("#@ slots 9").unwrap_err(),
            "line 1: slots takes 1 to 4, not \"9\""
        );
        assert!(parse("\n#@ bogus").unwrap_err().starts_with("line 2: "));
        assert!(parse("#@ setting alc").is_err());
        assert!(parse("slot 2")
            .unwrap_err()
            .starts_with("line 1: slot takes"));
        assert!(parse("#@ slots 2\nslot 2").is_ok());
    }
}
