//! The guest's report on a boot's mods, as `cdj3k-cfgd` sends it on the cfg
//! channel (see [`crate::cfg`]).

/// What the guest reported about this boot's mods.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ModsReport {
    /// Nothing received yet for this boot.
    #[default]
    Unknown,
    /// The boot has no mods.
    Off,
    /// The mods runner has not finished.
    Pending,
    /// One outcome per mod, in boot order, and the `mod` lines that could
    /// not be parsed.
    Done {
        outcomes: Vec<ModOutcome>,
        unread: Vec<Unread>,
    },
}

/// A `mod` line that names a valid mod but cannot be parsed as an outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unread {
    pub name: String,
    /// The line, without the leading `mod `.
    pub line: String,
}

/// The maximum number of `mod` lines in a report.
const MAX: usize = 256;
/// The maximum length of a `mod` line, in bytes; a longer line is dropped.
const LINE_MAX: usize = 1024;

/// The `mod` lines received between `mods begin` and `mods end`.
#[derive(Clone, Debug, Default)]
pub struct Partial {
    outcomes: Vec<ModOutcome>,
    unread: Vec<Unread>,
}

impl Partial {
    /// Add `rest`, a `mod` line without its leading `mod `. Returns `false`
    /// when the line names no valid mod, is too long, or the report is full.
    pub fn push(&mut self, rest: &str) -> bool {
        if self.outcomes.len() + self.unread.len() >= MAX || rest.len() > LINE_MAX {
            return false;
        }
        if let Some(outcome) = ModOutcome::parse(rest) {
            self.outcomes.push(outcome);
            return true;
        }
        match rest.split_whitespace().next().filter(|n| valid_name(n)) {
            Some(name) => {
                self.unread.push(Unread {
                    name: name.to_string(),
                    line: rest.trim().to_string(),
                });
                true
            }
            None => false,
        }
    }

    pub fn done(self) -> ModsReport {
        ModsReport::Done {
            outcomes: self.outcomes,
            unread: self.unread,
        }
    }
}

impl ModsReport {
    /// The report as the lines cfgd sends.
    pub fn lines(&self) -> Vec<String> {
        match self {
            ModsReport::Unknown => Vec::new(),
            ModsReport::Off => vec!["mods off".into()],
            ModsReport::Pending => vec!["mods pending".into()],
            ModsReport::Done { outcomes, unread } => std::iter::once("mods begin".to_string())
                .chain(outcomes.iter().map(ModOutcome::line))
                .chain(unread.iter().map(|u| format!("mod {}", u.line)))
                .chain(std::iter::once("mods end".to_string()))
                .collect(),
        }
    }

    /// Parse lines written by [`Self::lines`].
    pub fn from_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> Self {
        let mut report = ModsReport::Unknown;
        let mut partial: Option<Partial> = None;
        for line in lines {
            match line.trim() {
                "mods off" => report = ModsReport::Off,
                "mods pending" => report = ModsReport::Pending,
                "mods begin" => partial = Some(Partial::default()),
                "mods end" => {
                    if let Some(p) = partial.take() {
                        report = p.done();
                    }
                }
                other => {
                    if let (Some(rest), Some(p)) = (other.strip_prefix("mod "), partial.as_mut()) {
                        p.push(rest);
                    }
                }
            }
        }
        report
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModOutcome {
    pub name: String,
    pub script: Script,
    /// Libraries the mod passed to the app.
    pub libs: u32,
}

/// How the mod's `loader.sh` ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Script {
    Exit(i32),
    /// Stopped at the 60 s cap.
    Timeout,
    /// The mod has no `loader.sh`; its `mod.toml` declares everything it does.
    Missing,
}

impl ModOutcome {
    /// `mod <name> script <rc|timeout|none> libs <n>`, without the leading
    /// `mod `.
    pub fn parse(rest: &str) -> Option<Self> {
        let tokens: Vec<&str> = rest.split_whitespace().collect();
        let [name, "script", script, "libs", libs] = tokens.as_slice() else {
            return None;
        };
        if !valid_name(name) {
            return None;
        }
        let script = match *script {
            "timeout" => Script::Timeout,
            "none" => Script::Missing,
            rc => Script::Exit(rc.parse().ok()?),
        };
        Some(Self {
            name: name.to_string(),
            script,
            libs: libs.parse().ok()?,
        })
    }

    /// The `mod …` line for this outcome.
    pub fn line(&self) -> String {
        let script = match &self.script {
            Script::Exit(rc) => rc.to_string(),
            Script::Timeout => "timeout".into(),
            Script::Missing => "none".into(),
        };
        format!("mod {} script {script} libs {}", self.name, self.libs)
    }

    pub fn failed(&self) -> bool {
        !matches!(self.script, Script::Exit(0))
    }
}

/// Whether `name` is a valid mod name, by the rule for `mod.toml` names
/// (`cdj3k_emu_storage::mods::manifest`). The host names files after mods.
fn valid_name(name: &str) -> bool {
    // Windows treats these names as devices whatever the extension, and
    // drops a trailing dot from file names.
    const DEVICES: [&str; 22] = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    let stem = name.split('.').next().unwrap_or(name);
    name.len() <= 48
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
        && !name.ends_with('.')
        && !DEVICES.contains(&stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_parse() {
        assert_eq!(
            ModOutcome::parse("good script 0 libs 1"),
            Some(ModOutcome {
                name: "good".into(),
                script: Script::Exit(0),
                libs: 1,
            })
        );
        let slow = ModOutcome::parse("slow script timeout libs 0").unwrap();
        assert_eq!(slow.script, Script::Timeout);
        assert!(slow.failed());
        assert_eq!(ModOutcome::parse("x script 0 libs"), None);
        assert_eq!(ModOutcome::parse("x script zero libs 0"), None);
        assert_eq!(ModOutcome::parse("x script 0 integrity ok libs 0"), None);
        assert_eq!(ModOutcome::parse("../../x script 0 libs 0"), None);
        assert_eq!(ModOutcome::parse("X script 0 libs 0"), None);
    }

    #[test]
    fn an_end_without_its_begin_is_ignored() {
        let tail = ModsReport::from_lines(["mod a script 0 libs 0", "mods end"]);
        assert_eq!(tail, ModsReport::Unknown);
        let off_then_tail = ModsReport::from_lines(["mods off", "mods end"]);
        assert_eq!(off_then_tail, ModsReport::Off);
    }

    #[test]
    fn a_report_round_trips_through_its_lines() {
        let mut partial = Partial::default();
        assert!(partial.push("a script 2 libs 0"));
        assert!(partial.push("b script timeout libs 3"));
        assert!(
            partial.push("c script 0 integrity ok libs 1"),
            "names a mod"
        );
        assert!(!partial.push("../x script 0"), "names none");
        let report = partial.done();
        let ModsReport::Done { outcomes, unread } = &report else {
            unreachable!()
        };
        assert_eq!(outcomes.len(), 2);
        assert_eq!(unread[0].name, "c");
        let lines = report.lines();
        assert_eq!(
            ModsReport::from_lines(lines.iter().map(String::as_str)),
            report
        );
        assert_eq!(ModsReport::from_lines(["mods off"]), ModsReport::Off);
    }
}
