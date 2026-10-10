//! The slot's mods in boot order, kept in `list.txt`.

use std::io;
use std::path::{Path, PathBuf};

use super::{manifest, mods_dir, report, write_atomic, INSTALLED};

pub(super) const LIST: &str = "list.txt";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Unpacked into the slot's `mods/installed/<name>/` from `origin`, which
    /// is the archive's file name or the URL it was downloaded from.
    Installed { origin: String },
    /// A folder elsewhere, read at every boot and never copied.
    Folder(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotMod {
    /// The name from its `mod.toml`, or the folder's name when a folder's
    /// `mod.toml` can no longer be read.
    pub name: String,
    pub source: Source,
    pub enabled: bool,
}

/// A slot's mods in boot order.
#[derive(Clone, Debug)]
pub struct SlotMods {
    pub(super) dir: PathBuf,
    pub mods: Vec<SlotMod>,
}

impl SlotMods {
    pub fn load(instance_id: u32) -> Self {
        Self::load_in(mods_dir(instance_id))
    }

    pub(super) fn load_in(dir: PathBuf) -> Self {
        let text = std::fs::read_to_string(dir.join(LIST)).unwrap_or_default();
        let mut mods: Vec<SlotMod> = Vec::new();
        // A folder's name comes from its mod.toml, so editing it can repeat a
        // name already in the list. The first entry with a name wins.
        for m in text.lines().filter_map(read_line) {
            if mods.iter().any(|kept| kept.name == m.name) {
                eprintln!(
                    "cdj3k-emu: {}: skipping a second mod named {}",
                    dir.display(),
                    m.name
                );
            } else {
                mods.push(m);
            }
        }
        Self { dir, mods }
    }

    pub fn save(&self) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let mut text = String::new();
        for m in &self.mods {
            text.push_str(if m.enabled { "on " } else { "off " });
            match &m.source {
                Source::Installed { origin } => {
                    text.push_str(&m.name);
                    text.push('\t');
                    text.push_str(origin);
                }
                Source::Folder(path) => text.push_str(&path.to_string_lossy()),
            }
            text.push('\n');
        }
        write_atomic(&self.dir.join(LIST), &text)
    }

    /// Where `m`'s files are.
    pub fn dir_of(&self, m: &SlotMod) -> PathBuf {
        match &m.source {
            Source::Installed { .. } => self.installed(&m.name),
            Source::Folder(path) => path.clone(),
        }
    }

    pub(super) fn installed(&self, name: &str) -> PathBuf {
        self.dir.join(INSTALLED).join(name)
    }

    /// An id for the version of `m` that a boot runs, made of its name, its
    /// source and its version. A mod with the same name from another source, or in
    /// another version, gets another id, so the last boot's report does not
    /// apply to it.
    pub fn boot_id(&self, m: &SlotMod) -> String {
        let source = match &m.source {
            Source::Installed { origin } => origin.clone(),
            Source::Folder(path) => path.display().to_string(),
        };
        let version =
            manifest::read(&self.dir_of(m)).map_or_else(|_| "?".into(), |man| man.version);
        format!("{}\t{source}\t{version}", m.name)
    }

    pub fn get(&self, name: &str) -> Option<&SlotMod> {
        self.mods.iter().find(|m| m.name == name)
    }

    /// Drop `name` from the list. An installed mod's files are deleted; a
    /// folder used in place is left as it is.
    pub fn remove(&mut self, name: &str) -> io::Result<()> {
        let Some(at) = self.mods.iter().position(|m| m.name == name) else {
            return Ok(());
        };
        let m = self.mods.remove(at);
        self.save()?;
        report::forget(&self.dir, &m.name);
        if matches!(m.source, Source::Installed { .. }) {
            match std::fs::remove_dir_all(self.installed(&m.name)) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        Ok(())
    }

    pub fn set_enabled(&mut self, name: &str, enabled: bool) -> io::Result<()> {
        if let Some(m) = self.mods.iter_mut().find(|m| m.name == name) {
            m.enabled = enabled;
        }
        self.save()
    }

    /// Move the mod at `from` to `to`, shifting the ones between.
    pub fn reorder(&mut self, from: usize, to: usize) -> io::Result<()> {
        if from < self.mods.len() && to < self.mods.len() && from != to {
            let m = self.mods.remove(from);
            self.mods.insert(to, m);
        }
        self.save()
    }
}

/// One line of `list.txt`, or `None` when it is not `on|off` followed by an
/// absolute folder path or a valid mod name.
fn read_line(line: &str) -> Option<SlotMod> {
    let (state, entry) = line.split_once(' ')?;
    let enabled = match state {
        "on" => true,
        "off" => false,
        _ => return None,
    };

    let path = Path::new(entry);
    let (name, source) = if path.is_absolute() {
        let name = manifest::read(path).map(|m| m.name).unwrap_or_else(|_| {
            path.file_name()
                .map_or_else(|| entry.to_string(), |f| f.to_string_lossy().into())
        });
        (name, Source::Folder(path.to_path_buf()))
    } else {
        let (name, origin) = entry.split_once('\t').unwrap_or((entry, entry));
        if !manifest::valid_name(name) {
            return None;
        }
        let origin = origin.to_string();
        (name.to_string(), Source::Installed { origin })
    };
    Some(SlotMod {
        name,
        source,
        enabled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::report::{LAST_BOOT, LAST_BOOT_LOGS};
    use crate::mods::test_support::*;
    use crate::mods::{LOADER, MANIFEST};

    #[test]
    fn the_name_comes_from_mod_toml() {
        let root = scratch("name");
        let dev = mod_folder(&root, "Some Folder", "dev-mod", "1");
        let mut mods = SlotMods::load_in(root.join("mods"));
        assert_eq!(mods.add(&dev, None).unwrap(), "dev-mod");
        mods.set_enabled("dev-mod", false).unwrap();
        let back = SlotMods::load_in(root.join("mods"));
        assert_eq!(back.mods, mods.mods);
        assert!(
            mods.add(&dev.join(MANIFEST), None).is_err(),
            "a second dev-mod"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_list_entry_that_is_not_a_mod_name_is_dropped() {
        let root = scratch("bad-list");
        let dir = root.join("mods");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(LIST), "on ../../x\tx.tgz\non good\tgood.tgz\n").unwrap();
        let mods = SlotMods::load_in(dir);
        assert_eq!(mods.mods.len(), 1);
        assert_eq!(mods.mods[0].name, "good");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_copy_from_elsewhere_or_another_version_is_another_boot_id() {
        let root = scratch("boot-id");
        let dev = mod_folder(&root, "dev", "theme", "1.0");
        tgz(&dev, &root.join("theme.tgz"), true);
        let mut mods = SlotMods::load_in(root.join("mods"));
        mods.add(&dev, None).unwrap();
        let as_folder = mods.boot_id(&mods.mods[0]);
        mods.commit(mods.prepare(&root.join("theme.tgz"), None).unwrap())
            .unwrap();
        let installed = mods.boot_id(&mods.mods[0]);
        assert_ne!(as_folder, installed);
        assert_eq!(installed, mods.boot_id(&mods.mods[0]), "stable");
        mod_folder(&root.join("mods/installed"), "theme", "theme", "2.0");
        assert_ne!(installed, mods.boot_id(&mods.mods[0]));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removing_a_mod_forgets_its_last_boot() {
        let root = scratch("forget");
        let dir = root.join("mods");
        let a = mod_folder(&root, "a", "a", "1");
        let b = mod_folder(&root, "b", "b", "1");
        let mut mods = SlotMods::load_in(dir.clone());
        mods.add(&a, None).unwrap();
        mods.add(&b, None).unwrap();
        let ids = [mods.boot_id(&mods.mods[0]), mods.boot_id(&mods.mods[1])];
        std::fs::write(
            dir.join(LAST_BOOT),
            format!(
                "mods begin\nmod a script 0 libs 0\nmod b script 0 libs 0\nmods end\nid {}\nid {}\n",
                ids[0], ids[1]
            ),
        )
        .unwrap();
        std::fs::create_dir_all(dir.join(LAST_BOOT_LOGS)).unwrap();
        std::fs::write(dir.join(LAST_BOOT_LOGS).join("a.log"), "x\n").unwrap();
        mods.remove("a").unwrap();
        let report = std::fs::read_to_string(dir.join(LAST_BOOT)).unwrap();
        assert_eq!(
            report,
            format!(
                "mods begin\nmod b script 0 libs 0\nmods end\nid {}\n",
                ids[1]
            )
        );
        assert!(!dir.join(LAST_BOOT_LOGS).join("a.log").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_repeated_name_in_the_list_is_skipped() {
        let root = scratch("dup");
        let a = mod_folder(&root, "a", "same", "1");
        let b = mod_folder(&root, "b", "same", "2");
        let dir = root.join("mods");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(LIST),
            format!("on {}\noff {}\n", a.display(), b.display()),
        )
        .unwrap();
        let mods = SlotMods::load_in(dir);
        assert_eq!(mods.mods.len(), 1);
        assert_eq!(mods.mods[0].source, Source::Folder(a));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removing_a_folder_leaves_it_alone() {
        let root = scratch("eject");
        let dev = mod_folder(&root, "dev", "dev", "1");
        let mut mods = SlotMods::load_in(root.join("mods"));
        mods.add(&dev, None).unwrap();
        mods.remove("dev").unwrap();
        assert!(dev.join(LOADER).is_file());
        assert!(mods.mods.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_list_file_has_one_line_per_mod() {
        let root = scratch("list-file");
        let dir = root.join("mods");
        let a = mod_folder(&root, "a", "a", "1");
        let b = mod_folder(&root, "b-src", "b", "1");
        tgz(&b, &root.join("b.tgz"), true);
        let mut mods = SlotMods::load_in(dir.clone());
        mods.add(&a, None).unwrap();
        mods.add(&root.join("b.tgz"), Some("https://x/b.tgz"))
            .unwrap();
        mods.set_enabled("a", false).unwrap();
        mods.set_enabled("nobody", true).unwrap();
        let a = std::fs::canonicalize(&a).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(LIST)).unwrap(),
            format!("off {}\non b\thttps://x/b.tgz\n", a.display())
        );
        mods.reorder(0, 1).unwrap();
        mods.reorder(0, 5).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(LIST)).unwrap(),
            format!("on b\thttps://x/b.tgz\noff {}\n", a.display())
        );
        assert_eq!(mods.dir_of(&mods.mods[0]), dir.join("installed/b"));
        assert_eq!(mods.dir_of(&mods.mods[1]), a);
        assert_eq!(
            mods.boot_id(&mods.mods[0]),
            "b\thttps://x/b.tgz\t1".to_string()
        );
        assert_eq!(
            mods.boot_id(&mods.mods[1]),
            format!("a\t{}\t1", a.display())
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn list_lines_that_cannot_be_read_are_skipped() {
        let root = scratch("list-lines");
        let dir = root.join("mods");
        std::fs::create_dir_all(&dir).unwrap();
        let gone = root.join("gone-folder");
        std::fs::write(
            dir.join(LIST),
            format!(
                "maybe x\tx.tgz\nnospace\non plain\noff {}\non ok\tok.tgz\n",
                gone.display()
            ),
        )
        .unwrap();
        let mods = SlotMods::load_in(dir);
        assert_eq!(
            mods.mods,
            [
                SlotMod {
                    name: "plain".into(),
                    source: Source::Installed {
                        origin: "plain".into()
                    },
                    enabled: true,
                },
                SlotMod {
                    name: "gone-folder".into(),
                    source: Source::Folder(gone),
                    enabled: false,
                },
                SlotMod {
                    name: "ok".into(),
                    source: Source::Installed {
                        origin: "ok.tgz".into()
                    },
                    enabled: true,
                },
            ]
        );
        assert_eq!(mods.boot_id(&mods.mods[0]), "plain\tplain\t?");
        std::fs::remove_dir_all(root).unwrap();
    }
}
