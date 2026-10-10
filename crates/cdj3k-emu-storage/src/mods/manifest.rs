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
        std::io::ErrorKind::NotFound => format!("no {FILE}"),
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

pub fn parse(text: &str) -> Result<Manifest, String> {
    let err = |m: String| format!("{FILE}: {m}");
    let table: toml::Table = text.parse().map_err(|e| err(format!("{e}")))?;
    let m = table
        .get("mod")
        .and_then(|v| v.as_table())
        .ok_or_else(|| err("no [mod] table".into()))?;
    let string = |key: &str| -> Result<Option<String>, String> {
        match m.get(key) {
            None => Ok(None),
            Some(v) => v
                .as_str()
                .map(|s| Some(s.to_string()))
                .ok_or_else(|| err(format!("mod.{key} is not a string"))),
        }
    };
    let name = string("name")?.ok_or_else(|| err("no mod.name".into()))?;
    if !valid_name(&name) {
        return Err(err(format!(
            "mod.name {name:?}: lowercase letters, digits, '.', '_' and '-', starting with a \
             letter or digit"
        )));
    }
    let version = string("version")?.ok_or_else(|| err("no mod.version".into()))?;
    if version.is_empty() || version.contains(char::is_whitespace) {
        return Err(err(format!("mod.version {version:?}: must be one word")));
    }
    let author = string("author")?.filter(|a| !a.trim().is_empty());
    let description = string("description")?;
    let url = string("url")?
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());

    let mut preload = Vec::new();
    if let Some(v) = m.get("preload") {
        let list = v
            .as_array()
            .ok_or_else(|| err("mod.preload is not a list".into()))?;
        for item in list {
            let lib = item
                .as_str()
                .ok_or_else(|| err("mod.preload contains a non-string".into()))?;
            let inside = !lib.is_empty()
                && !lib.starts_with('/')
                && !lib.split('/').any(|part| part == "..");
            if !inside {
                return Err(err(format!(
                    "mod.preload {lib:?}: must be a path inside the mod"
                )));
            }
            if !valid_preload(lib) {
                return Err(err(format!(
                    "mod.preload {lib:?}: must not contain ':', spaces, quotes or backslashes"
                )));
            }
            preload.push(lib.to_string());
        }
    }

    let mut env = Vec::new();
    if let Some(v) = m.get("env") {
        let vars = v
            .as_table()
            .ok_or_else(|| err("mod.env is not a table".into()))?;
        for (key, value) in vars {
            let value = value
                .as_str()
                .ok_or_else(|| err(format!("mod.env.{key} is not a string")))?;
            if !valid_env_key(key) {
                return Err(err(format!("mod.env.{key}: not a variable name")));
            }
            if key == "LD_PRELOAD" {
                return Err(err("mod.env.LD_PRELOAD: use mod.preload".into()));
            }
            if !valid_env_value(value) {
                return Err(err(format!(
                    "mod.env.{key}: the value contains a quote, a backslash or a control \
                     character, or starts or ends with a space"
                )));
            }
            env.push((key.clone(), value.to_string()));
        }
    }

    let compat = match table.get("compat") {
        None => Declared::default(),
        Some(v) => Declared::parse(v).map_err(err)?,
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
