//! Command-line options, defaulted from their environment variables.

use std::path::PathBuf;

use cdj3k_emu_panel::Model;

pub struct Cli {
    /// `--instance N` selects slot N, from 1 to 4. The default is slot 1. The
    /// Instances menu launches the app again with this option.
    pub instance: u32,
    /// `--model <slug>` or `CDJ3K_MODEL` skips the picker and boots this model
    /// at once. Headless captures, development loops and launch scripts use it.
    pub initial_model: Option<Model>,
    pub kernel: Option<PathBuf>,
    pub initramfs: Option<PathBuf>,
    pub no_emmc: bool,
    /// The serial log is off by default, because it grows without limit and
    /// only helps to debug boot failures and kernel panics. `--serial-log` or
    /// `CDJ3K_SERIAL_LOG=1` turns it on.
    pub serial_log: bool,
    /// Profiling is off by default. `--profile` or `CDJ3K_PROFILE=1` enables
    /// the puffin scopes and binds a localhost TCP listener for puffin_viewer.
    pub profile: bool,
    /// `--service-mode` boots straight into the sub-CPU test mode. The
    /// Service Mode menu item does the same through a restart.
    pub service_mode: bool,
    /// `--provision <file>` runs the Install Firmware pipeline on a .UPD or
    /// .iso file for `--instance` and `--model` without a window, then exits.
    /// `--key <file>` gives it the key file.
    pub provision_path: Option<PathBuf>,
    pub key_path: Option<PathBuf>,
    /// `--no-spawn` or `CDJ3K_NO_SPAWN=1` shows the chassis without the
    /// runtime worker, QEMU or the boot shade. It is meant for layout work and
    /// `CDJ3K_SCREENSHOT` captures.
    pub no_spawn: bool,
    /// The updater passes `--after-update` while the replaced build is still
    /// exiting, so the app waits for the slot to be released.
    pub after_update: bool,
    /// Each `--mod <folder>` adds a mod folder for this launch only.
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
                "cdj3k-emu: the model {:?} is unknown, so the app shows the picker. \
                 The known models are {}.",
                slug,
                Model::ALL.map(|m| m.spec().slug).join(", ")
            ),
        }
    }
}

/// Whether the variable is set to a value other than an empty string or `0`.
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
