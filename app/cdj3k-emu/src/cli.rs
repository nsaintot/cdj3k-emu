//! Command-line options, defaulted from their environment variables.

use std::path::PathBuf;

use cdj3k_emu_panel::Model;

pub struct Cli {
    /// Slot 1 is the default for a fresh launch. Slots 2..=4 are reachable
    /// via `--instance N` (the "Instances" menu launches us with `open -n`).
    pub instance: u32,
    /// `--model <slug>` / `CDJ3K_MODEL`: skip the picker and boot this
    /// model right away (headless captures, dev loops, launch scripts).
    pub initial_model: Option<Model>,
    pub kernel: Option<PathBuf>,
    pub initramfs: Option<PathBuf>,
    pub no_emmc: bool,
    /// Default off: serial log grows unbounded and is only useful for debugging
    /// boot/kernel panics. Enabled via `--serial-log` or `CDJ3K_SERIAL_LOG=1`.
    pub serial_log: bool,
    /// Default off: enables puffin scopes + binds a localhost TCP listener
    /// for puffin_viewer.  Off in shipping builds; enable with `--profile`
    /// or `CDJ3K_PROFILE=1` for performance investigations.
    pub profile: bool,
    /// `--service-mode` boots straight into the sub-CPU test mode (the
    /// "Service Mode" menu item, which otherwise needs a restart).
    pub service_mode: bool,
    /// `--provision <.UPD|.iso> [--key <file>]`: run the Install Firmware
    /// pipeline for `--instance` / `--model` without a window, then exit.
    pub provision_path: Option<PathBuf>,
    pub key_path: Option<PathBuf>,
    /// `--no-spawn` / `CDJ3K_NO_SPAWN=1`: chassis only - no runtime worker, no
    /// QEMU, no boot shade. For layout work and `CDJ3K_SCREENSHOT` captures.
    pub no_spawn: bool,
    /// `--after-update`: started by the updater while the build it replaced is
    /// still exiting, so the slot is waited for rather than found taken.
    pub after_update: bool,
    /// `--mod <folder>` (repeatable) adds a mod folder for this launch only.
    pub extra_mods: Vec<PathBuf>,
    /// `--no-mods` boots without the slot's mods.
    pub no_mods: bool,
}

type FlagSetter = fn(&mut Cli);
type ValueSetter = fn(&mut Cli, &str);

const FLAGS: &[(&str, FlagSetter)] = &[
    ("--no-emmc", |c| c.no_emmc = true),
    ("--serial-log", |c| c.serial_log = true),
    ("--profile", |c| c.profile = true),
    ("--service-mode", |c| c.service_mode = true),
    ("--no-spawn", |c| c.no_spawn = true),
    ("--after-update", |c| c.after_update = true),
    ("--no-mods", |c| c.no_mods = true),
];

const VALUE_OPTIONS: &[(&str, ValueSetter)] = &[
    ("--instance", |c, v| c.instance = v.parse().unwrap_or(0)),
    ("--model", Cli::set_model),
    ("--kernel", |c, v| c.kernel = Some(PathBuf::from(v))),
    ("--initramfs", |c, v| c.initramfs = Some(PathBuf::from(v))),
    ("--provision", |c, v| {
        c.provision_path = Some(PathBuf::from(v))
    }),
    ("--key", |c, v| c.key_path = Some(PathBuf::from(v))),
    ("--mod", |c, v| c.extra_mods.push(PathBuf::from(v))),
];

impl Cli {
    /// Parse `args` (with the program name first). Unknown arguments, and an
    /// option missing its value, are ignored.
    pub fn parse(args: &[String]) -> Self {
        let mut cli = Self::from_env();
        let mut args = args.iter().skip(1);
        while let Some(arg) = args.next() {
            if let Some((_, set)) = FLAGS.iter().find(|(name, _)| name == arg) {
                set(&mut cli);
            } else if let Some((_, set)) = VALUE_OPTIONS.iter().find(|(name, _)| name == arg) {
                if let Some(value) = args.next() {
                    set(&mut cli, value);
                }
            }
        }
        cli
    }

    fn from_env() -> Self {
        Self {
            instance: 1,
            initial_model: std::env::var("CDJ3K_MODEL")
                .ok()
                .and_then(|v| Model::parse(&v)),
            kernel: None,
            initramfs: None,
            no_emmc: false,
            serial_log: env_flag("CDJ3K_SERIAL_LOG"),
            profile: env_flag("CDJ3K_PROFILE"),
            service_mode: false,
            provision_path: None,
            key_path: None,
            no_spawn: env_flag("CDJ3K_NO_SPAWN"),
            after_update: false,
            extra_mods: Vec::new(),
            no_mods: false,
        }
    }

    fn set_model(&mut self, slug: &str) {
        match Model::parse(slug) {
            Some(m) => self.initial_model = Some(m),
            None => eprintln!(
                "cdj3k-emu: unknown --model {:?} ({}) - showing the picker",
                slug,
                Model::ALL.map(|m| m.spec().slug).join(" | ")
            ),
        }
    }
}

/// Set unless empty or `0`.
fn env_flag(name: &str) -> bool {
    std::env::var_os(name)
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        let args: Vec<String> = std::iter::once("cdj3k-emu")
            .chain(args.iter().copied())
            .map(String::from)
            .collect();
        Cli::parse(&args)
    }

    #[test]
    fn options_take_the_next_argument() {
        let cli = parse(&["--instance", "3", "--kernel", "k", "--key", "--no-emmc"]);
        assert_eq!(cli.instance, 3);
        assert_eq!(cli.kernel, Some(PathBuf::from("k")));
        assert_eq!(cli.key_path, Some(PathBuf::from("--no-emmc")));
        assert!(!cli.no_emmc);
    }

    #[test]
    fn flags_unknown_arguments_and_a_trailing_option() {
        let cli = parse(&["--bogus", "--service-mode", "--after-update", "--provision"]);
        assert!(cli.service_mode);
        assert!(cli.after_update);
        assert_eq!(cli.provision_path, None);
    }

    #[test]
    fn mod_folders_accumulate() {
        let cli = parse(&["--mod", "a", "--no-mods", "--mod", "b"]);
        assert_eq!(cli.extra_mods, [PathBuf::from("a"), PathBuf::from("b")]);
        assert!(cli.no_mods);
    }

    #[test]
    fn an_unparsable_instance_is_zero() {
        assert_eq!(parse(&["--instance", "x"]).instance, 0);
    }
}
