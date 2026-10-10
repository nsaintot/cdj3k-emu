//! `mod.toml`, the file that makes a folder a mod.
//!
//! ```toml
//! [mod]
//! name = "cdj3k-mods"                    # required, [a-z0-9._-]
//! version = "0.1.1"                      # required
//! author = "…"                           # optional
//! description = "…"                      # optional
//! url = "https://github.com/…"           # optional, the project's web page
//! preload = ["ep122_shim.so"]            # optional, relative to the mod
//! env = { EP122_MOD_LOGLEVEL = "info" }  # optional
//!
//! [compat]                               # optional, see [`super::compat`]
//! cdj3k = ">=3.13, <=3.22"
//! ```
//!
//! `preload` and `env` reach the player app as if `loader.sh` had called
//! `mod-preload` and `mod-env`. `loader.sh` itself is optional.

use std::path::Path;

use cdj3k_emu_panel::Model;

use super::compat::{Compat, Declared};

pub const FILE: &str = "mod.toml";
/// The script the guest runs, when the mod has one.
pub const LOADER: &str = "loader.sh";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub author: Option<String>,
    pub description: Option<String>,
    /// The project's web page, exactly as mod.toml gives it.
    pub url: Option<String>,
    /// Libraries for the app, relative to the mod's folder.
    pub preload: Vec<String>,
    /// Variables for the app, in file order.
    pub env: Vec<(String, String)>,
    pub compat: Declared,
}

impl Manifest {
    pub fn check(&self, model: Model, release: Option<&str>) -> Compat {
        self.compat.check(model, release)
    }
}

/// Read the `mod.toml` of the mod in `dir`, and check that its `preload`
/// files exist in the folder.
pub fn read(dir: &Path) -> Result<Manifest, String> {
    let text = std::fs::read_to_string(dir.join(FILE)).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("{FILE} is missing"),
        _ => format!("{FILE}: {e}"),
    })?;
    let manifest = parse(&text)?;
    for lib in &manifest.preload {
        if !dir.join(lib).is_file() {
            return Err(format!("{FILE}: preload {lib}: no such file in the mod"));
        }
    }
    Ok(manifest)
}

/// [`Manifest::check`] for the mod in `dir`. A mod whose `mod.toml` is
/// missing or invalid never runs.
pub fn check_dir(dir: &Path, model: Model, release: Option<&str>) -> Compat {
    match read(dir) {
        Ok(m) => m.check(model, release),
        Err(e) => Compat::Invalid(e),
    }
}

/// Parse a `mod.toml`. Each error starts with `mod.toml: `.
pub fn parse(text: &str) -> Result<Manifest, String> {
    parse_tables(text).map_err(|m| format!("{FILE}: {m}"))
}

fn parse_tables(text: &str) -> Result<Manifest, String> {
    let table: toml::Table = text.parse().map_err(|e| format!("{e}"))?;
    let m = table
        .get("mod")
        .and_then(|v| v.as_table())
        .ok_or_else(|| "the [mod] table is missing".to_string())?;
    let name = string(m, "name")?.ok_or_else(|| "mod.name is missing".to_string())?;
    if !valid_name(&name) {
        return Err(format!(
            "mod.name {name:?}: a name may contain only lowercase letters, digits, '.', '_' \
             and '-', and must start with a letter or digit"
        ));
    }
    let version = string(m, "version")?.ok_or_else(|| "mod.version is missing".to_string())?;
    if version.is_empty() || version.contains(char::is_whitespace) {
        return Err(format!("mod.version {version:?}: must be one word"));
    }
    let author = string(m, "author")?.filter(|a| !a.trim().is_empty());
    let description = string(m, "description")?;
    let url = string(m, "url")?
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());
    let preload = m.get("preload").map_or(Ok(Vec::new()), parse_preload)?;
    let env = m.get("env").map_or(Ok(Vec::new()), parse_env)?;
    let compat = match table.get("compat") {
        None => Declared::default(),
        Some(v) => Declared::parse(v)?,
    };
    Ok(Manifest {
        name,
        version,
        author,
        description,
        url,
        preload,
        env,
        compat,
    })
}

/// `mod.<key>`, which must be a string when present.
fn string(m: &toml::Table, key: &str) -> Result<Option<String>, String> {
    match m.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_str()
            .map(|s| Some(s.to_string()))
            .ok_or_else(|| format!("mod.{key} is not a string")),
    }
}

/// `mod.preload`: a list of paths inside the mod that `LD_PRELOAD` can carry.
fn parse_preload(v: &toml::Value) -> Result<Vec<String>, String> {
    let list = v
        .as_array()
        .ok_or_else(|| "mod.preload is not a list".to_string())?;
    let mut preload = Vec::new();
    for item in list {
        let lib = item
            .as_str()
            .ok_or_else(|| "mod.preload contains a non-string".to_string())?;
        let inside =
            !lib.is_empty() && !lib.starts_with('/') && !lib.split('/').any(|part| part == "..");
        if !inside {
            return Err(format!(
                "mod.preload {lib:?}: must be a path inside the mod"
            ));
        }
        if !valid_preload(lib) {
            return Err(format!(
                "mod.preload {lib:?}: must not contain ':', spaces, quotes or backslashes"
            ));
        }
        preload.push(lib.to_string());
    }
    Ok(preload)
}

/// `mod.env`: a table of variables whose values reach the app unchanged.
fn parse_env(v: &toml::Value) -> Result<Vec<(String, String)>, String> {
    let vars = v
        .as_table()
        .ok_or_else(|| "mod.env is not a table".to_string())?;
    let mut env = Vec::new();
    for (key, value) in vars {
        let value = value
            .as_str()
            .ok_or_else(|| format!("mod.env.{key} is not a string"))?;
        if !valid_env_key(key) {
            return Err(format!(
                "mod.env.{key}: the key is not a valid variable name"
            ));
        }
        if key == "LD_PRELOAD" {
            return Err("mod.env.LD_PRELOAD: use mod.preload".into());
        }
        if !valid_env_value(value) {
            return Err(format!(
                "mod.env.{key}: the value contains a quote, a backslash or a control \
                 character, or starts or ends with a space"
            ));
        }
        env.push((key.clone(), value.to_string()));
    }
    Ok(env)
}

/// Whether `name` is a valid mod name: `[a-z0-9._-]`, starting with a letter
/// or digit, at most 48 bytes, and a valid file name on every host. The guest
/// names the mod's systemd unit after it, and the host its folder.
pub fn valid_name(name: &str) -> bool {
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

/// Whether `value` reaches the app unchanged through the guest's
/// `EnvironmentFile=`, which interprets quotes and backslashes and trims
/// spaces around a value.
fn valid_env_value(value: &str) -> bool {
    !value
        .chars()
        .any(|c| matches!(c, '\\' | '"' | '\'') || c.is_control())
        && value.trim() == value
}

/// Whether `lib` can go into `LD_PRELOAD` unchanged: that list is separated
/// by ':' and spaces, and `EnvironmentFile=` interprets quotes and
/// backslashes.
fn valid_preload(lib: &str) -> bool {
    !lib.chars()
        .any(|c| matches!(c, ':' | '\\' | '"' | '\'') || c.is_whitespace() || c.is_control())
}

fn valid_env_key(key: &str) -> bool {
    key.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_manifest_parses() {
        let m = parse(
            "[mod]\nname = \"cdj3k-mods\"\nversion = \"0.1.1\"\ndescription = \"d\"\n\
             url = \" https://github.com/nsaintot/cdj3k-mods \"\n\
             preload = [\"lib/a.so\"]\nenv = { A = \"1\", B_2 = \"x y\" }\n\
             [compat]\ncdj3k = \"*\"\n",
        )
        .unwrap();
        assert_eq!(m.name, "cdj3k-mods");
        assert_eq!(m.version, "0.1.1");
        assert_eq!(
            m.url.as_deref(),
            Some("https://github.com/nsaintot/cdj3k-mods")
        );
        assert_eq!(m.preload, ["lib/a.so"]);
        assert_eq!(
            m.env,
            [("A".into(), "1".into()), ("B_2".into(), "x y".into())]
        );
        assert!(m.check(Model::Cdj3k, Some("3.22")).runs());
        assert!(!m.check(Model::Cdj3kx, Some("1.40")).runs());
    }

    #[test]
    fn name_and_version_are_required() {
        assert!(parse("").is_err());
        assert!(parse("[mod]\nversion = \"1\"\n").is_err());
        assert!(parse("[mod]\nname = \"a\"\n").is_err());
        assert!(parse("[mod]\nname = \"a\"\nversion = \"1\"\n").is_ok());
    }

    #[test]
    fn bad_fields_are_refused() {
        let with = |extra: &str| parse(&format!("[mod]\nname = \"a\"\nversion = \"1\"\n{extra}"));
        assert!(parse("[mod]\nname = \"Stemd Client\"\nversion = \"1\"\n").is_err());
        for name in ["con", "nul.x", "com1", "foo."] {
            let text = format!("[mod]\nname = \"{name}\"\nversion = \"1\"\n");
            assert!(parse(&text).is_err(), "{name}");
        }
        assert!(parse("[mod]\nname = \"console\"\nversion = \"1\"\n").is_ok());
        assert!(with("preload = [\"/abs.so\"]\n").is_err());
        assert!(with("env = { A = 'x\\' }\n").is_err());
        assert!(with("env = { A = 'say \"hi\"' }\n").is_err());
        assert!(with("env = { A = \"tab\\there\" }\n").is_err());
        assert!(with("env = { A = 'a=b c' }\n").is_ok());
        assert!(with("env = { A = ' a' }\n").is_err());
        assert!(with("preload = [\"a\\\\b.so\"]\n").is_err());
        assert!(with("preload = [\"../up.so\"]\n").is_err());
        assert!(with("preload = [\"a:b.so\"]\n").is_err());
        assert!(with("env = { LD_PRELOAD = \"x\" }\n").is_err());
        assert!(with("env = { \"1A\" = \"x\" }\n").is_err());
        assert!(with("[compat]\ncdj2000 = \"*\"\n").is_err());
        assert!(with("url = 1\n").is_err());
    }

    const BASE: &str = "[mod]\nname = \"a\"\nversion = \"1\"\n";

    fn parse_err(text: &str) -> String {
        parse(text).unwrap_err()
    }

    /// The error for a valid name and version followed by `extra`.
    fn with(extra: &str) -> String {
        parse_err(&format!("{BASE}{extra}"))
    }

    #[test]
    fn mod_table_refusals_have_their_messages() {
        let syntax = "[mod".parse::<toml::Table>().unwrap_err();
        assert_eq!(parse_err("[mod"), format!("mod.toml: {syntax}"));
        assert_eq!(parse_err(""), "mod.toml: the [mod] table is missing");
        assert_eq!(
            parse_err("mod = 1\n"),
            "mod.toml: the [mod] table is missing"
        );
        assert_eq!(
            parse_err("[mod]\nname = 1\n"),
            "mod.toml: mod.name is not a string"
        );
        assert_eq!(
            parse_err("[mod]\nversion = \"1\"\n"),
            "mod.toml: mod.name is missing"
        );
        assert_eq!(
            parse_err("[mod]\nname = \"Stemd Client\"\nversion = 1\n"),
            "mod.toml: mod.name \"Stemd Client\": a name may contain only lowercase letters, \
             digits, '.', '_' and '-', and must start with a letter or digit"
        );
        assert_eq!(
            parse_err("[mod]\nname = \"a\"\n"),
            "mod.toml: mod.version is missing"
        );
        assert_eq!(
            parse_err("[mod]\nname = \"a\"\nversion = 1\nauthor = 1\n"),
            "mod.toml: mod.version is not a string"
        );
        assert_eq!(
            parse_err("[mod]\nname = \"a\"\nversion = \"\"\n"),
            "mod.toml: mod.version \"\": must be one word"
        );
        assert_eq!(
            parse_err("[mod]\nname = \"a\"\nversion = \"1 2\"\n"),
            "mod.toml: mod.version \"1 2\": must be one word"
        );
        for key in ["author", "description", "url"] {
            assert_eq!(
                with(&format!("{key} = 1\npreload = 1\n")),
                format!("mod.toml: mod.{key} is not a string")
            );
        }
    }

    #[test]
    fn preload_refusals_have_their_messages() {
        assert_eq!(
            with("preload = \"a.so\"\nenv = 1\n"),
            "mod.toml: mod.preload is not a list"
        );
        assert_eq!(
            with("preload = [1]\n"),
            "mod.toml: mod.preload contains a non-string"
        );
        for lib in ["", "/abs.so", "a/../b.so", ".."] {
            assert_eq!(
                with(&format!("preload = ['{lib}']\n")),
                format!("mod.toml: mod.preload {lib:?}: must be a path inside the mod")
            );
        }
        for lib in ["a b.so", "a:b.so", "a\"b.so", "a\\b.so"] {
            assert_eq!(
                with(&format!("preload = ['{lib}', '/abs.so']\n")),
                format!(
                    "mod.toml: mod.preload {lib:?}: must not contain ':', spaces, quotes or \
                     backslashes"
                )
            );
        }
    }

    #[test]
    fn env_and_compat_refusals_have_their_messages() {
        assert_eq!(
            with("env = 1\n[compat]\ncdj2000 = \"*\"\n"),
            "mod.toml: mod.env is not a table"
        );
        assert_eq!(
            with("env = { A = 1 }\n"),
            "mod.toml: mod.env.A is not a string"
        );
        assert_eq!(
            with("env = { \"1A\" = 1 }\n"),
            "mod.toml: mod.env.1A is not a string"
        );
        assert_eq!(
            with("env = { \"1A\" = \"x\" }\n"),
            "mod.toml: mod.env.1A: the key is not a valid variable name"
        );
        assert_eq!(
            with("env = { LD_PRELOAD = \"'\" }\n"),
            "mod.toml: mod.env.LD_PRELOAD: use mod.preload"
        );
        assert_eq!(
            with("env = { A = \" a\" }\n"),
            "mod.toml: mod.env.A: the value contains a quote, a backslash or a control \
             character, or starts or ends with a space"
        );
        assert_eq!(
            parse_err(&format!("compat = 1\n{BASE}")),
            "mod.toml: `compat` is not a table"
        );
        assert_eq!(
            with("[compat]\ncdj3k = \"x\"\n"),
            "mod.toml: compat.cdj3k: \"x\" is not a range"
        );
    }

    #[test]
    fn optional_fields_are_normalised() {
        let m = parse(
            "[mod]\nname = \"a\"\nversion = \"1\"\nauthor = \" x \"\ndescription = \"\"\n\
             url = \"  \"\n",
        )
        .unwrap();
        assert_eq!(
            m,
            Manifest {
                name: "a".into(),
                version: "1".into(),
                author: Some(" x ".into()),
                description: Some(String::new()),
                url: None,
                preload: Vec::new(),
                env: Vec::new(),
                compat: Declared::default(),
            }
        );
        let m = parse("[mod]\nname = \"a\"\nversion = \"1\"\nauthor = \" \"\n").unwrap();
        assert_eq!((m.author, m.description, m.url), (None, None, None));
        let m = parse(
            "[mod]\nname = \"a\"\nversion = \"1\"\npreload = [\"x.so\", \"lib/y.so\"]\n\
             env = { Z = \"\" }\n[compat]\ncdj3kx = \">=1.0\"\ncdj3k = \"*\"\n",
        )
        .unwrap();
        assert_eq!(m.preload, ["x.so", "lib/y.so"]);
        assert_eq!(m.env, [("Z".into(), String::new())]);
        assert_eq!(
            m.compat.ranges,
            [
                ("cdj3k".into(), "*".into()),
                ("cdj3kx".into(), ">=1.0".into())
            ]
        );
    }

    #[test]
    fn read_reports_a_missing_or_unreadable_manifest() {
        let dir = std::env::temp_dir().join(format!("cdj3k-manifest-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read(&dir).unwrap_err(), "mod.toml is missing");
        assert_eq!(
            check_dir(&dir, Model::Cdj3k, None),
            Compat::Invalid("mod.toml is missing".into())
        );
        std::fs::create_dir_all(dir.join(FILE)).unwrap();
        let err = read(&dir).unwrap_err();
        assert!(err.starts_with("mod.toml: "), "{err}");
        std::fs::remove_dir_all(dir.join(FILE)).unwrap();
        std::fs::write(
            dir.join(FILE),
            "[mod]\nname = \"a\"\nversion = \"1\"\npreload = [\"lib/a.so\"]\n",
        )
        .unwrap();
        assert_eq!(
            read(&dir).unwrap_err(),
            "mod.toml: preload lib/a.so: no such file in the mod"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_preload_must_be_in_the_folder() {
        let dir = std::env::temp_dir().join(format!("cdj3k-manifest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(FILE),
            "[mod]\nname = \"a\"\nversion = \"1\"\npreload = [\"a.so\"]\n",
        )
        .unwrap();
        assert!(read(&dir).is_err());
        std::fs::write(dir.join("a.so"), b"").unwrap();
        assert!(read(&dir).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
