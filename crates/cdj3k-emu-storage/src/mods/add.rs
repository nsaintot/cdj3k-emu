//! Adding a mod to a slot: read and check it ([`SlotMods::prepare`]), then
//! put it in the list ([`SlotMods::commit`]).

use std::io;
use std::path::{Path, PathBuf};

use super::archive::unpack;
use super::list::{SlotMod, SlotMods, Source};
use super::{manifest, report, Manifest, INSTALLED, MANIFEST, MAX_MODS};

/// How old a leftover from an add, a replace or a download must be before
/// [`SlotMods::sweep`] deletes it.
const LEFTOVER_AGE: std::time::Duration = std::time::Duration::from_secs(3600);

/// A mod that has been read and checked but is not in the list yet.
/// [`SlotMods::commit`] adds it; dropping it deletes its unpacked files.
#[derive(Debug)]
pub struct PendingAdd {
    pub manifest: Manifest,
    /// The version of the same-name mod it would replace, or `"?"` when that
    /// mod's `mod.toml` cannot be read.
    pub replaces: Option<String>,
    /// Where it came from, as the list shows it.
    pub origin: String,
    /// `None` once [`SlotMods::commit`] has taken it.
    kind: Option<PendingKind>,
}

#[derive(Debug)]
enum PendingKind {
    /// Unpacked into `tmp`, with the mod's folder at `root` inside it.
    Archive {
        tmp: PathBuf,
        root: PathBuf,
    },
    Folder(PathBuf),
}

impl Drop for PendingAdd {
    fn drop(&mut self) {
        if let Some(PendingKind::Archive { tmp, .. }) = &self.kind {
            let _ = std::fs::remove_dir_all(tmp);
        }
    }
}

impl SlotMods {
    /// Delete what an add, a replace or a download left behind more than an
    /// hour ago, for example when the app quit with the Replace dialog open.
    /// A mod whose replace stopped halfway gets its old copy back.
    pub fn sweep(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let file = entry.file_name().to_string_lossy().into_owned();
            let leftover = [".add-", ".download-", ".replaced-"]
                .iter()
                .any(|p| file.starts_with(p))
                || file.ends_with(".tmp");
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age >= LEFTOVER_AGE);
            if !leftover || !old {
                continue;
            }
            let path = entry.path();
            // `.replaced-<pid>-<nanos>-<name>`
            let name = file
                .strip_prefix(".replaced-")
                .and_then(|rest| rest.splitn(3, '-').nth(2));
            if let Some(name) = name.filter(|n| manifest::valid_name(n)) {
                let installed = self.installed(name);
                if self.get(name).is_some() && !installed.exists() {
                    let _ = std::fs::rename(&path, &installed);
                    continue;
                }
            }
            let _ = if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
        }
    }

    /// Read the mod at `path`: a `.tgz`/`.tar.gz`/`.tar` (holding the mod
    /// folder or its contents), a folder used in place, or the folder's
    /// `mod.toml`. `origin` is the source the list shows for an archive; it
    /// defaults to the archive's file name.
    pub fn prepare(&self, path: &Path, origin: Option<&str>) -> io::Result<PendingAdd> {
        self.sweep();
        let file = path.file_name().map(|f| f.to_string_lossy().into_owned());
        let folder = if path.is_dir() {
            Some(path.to_path_buf())
        } else if file.as_deref() == Some(MANIFEST) {
            path.parent().map(Path::to_path_buf)
        } else {
            None
        };
        let (manifest, kind, origin) = match folder {
            Some(folder) => self.read_folder(folder)?,
            None => {
                let (manifest, kind) = self.read_archive(path)?;
                let origin = origin
                    .map(str::to_string)
                    .or(file)
                    .unwrap_or_else(|| path.display().to_string());
                (manifest, kind, origin)
            }
        };
        let replaces = self.get(&manifest.name).map(|old| {
            manifest::read(&self.dir_of(old))
                .map(|m| m.version)
                .unwrap_or_else(|_| "?".into())
        });
        let add = PendingAdd {
            manifest,
            replaces,
            origin: origin.replace(['\t', '\n'], " "),
            kind: Some(kind),
        };
        if add.replaces.is_none() && self.mods.len() >= MAX_MODS {
            return Err(io::Error::other(format!(
                "a slot holds at most {MAX_MODS} mods; remove one first"
            )));
        }
        Ok(add)
    }

    /// Read a folder used in place. Its origin is its canonical path.
    fn read_folder(&self, folder: PathBuf) -> io::Result<(Manifest, PendingKind, String)> {
        let folder = std::fs::canonicalize(folder)?;
        // Replacing an installed mod with its own folder would delete it.
        if std::fs::canonicalize(&self.dir).is_ok_and(|d| folder.starts_with(d)) {
            return Err(io::Error::other(format!(
                "{}: this folder is inside the slot's mods folder; add the mod's archive \
                 or a copy of the folder",
                folder.display()
            )));
        }
        let manifest = manifest::read(&folder).map_err(io::Error::other)?;
        let origin = folder.display().to_string();
        Ok((manifest, PendingKind::Folder(folder), origin))
    }

    /// Unpack `archive` into a new `.add-` folder of the slot, which is
    /// deleted again when it holds no valid mod.
    fn read_archive(&self, archive: &Path) -> io::Result<(Manifest, PendingKind)> {
        std::fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join(format!(".add-{}", unique_suffix()));
        let read = unpack(archive, &tmp).and_then(|()| {
            let root = mod_root(&tmp).ok_or_else(|| {
                io::Error::other(format!(
                    "{}: the archive has no {MANIFEST} at its root",
                    archive.display()
                ))
            })?;
            let manifest = manifest::read(&root).map_err(io::Error::other)?;
            Ok((manifest, root))
        });
        match read {
            Ok((manifest, root)) => Ok((manifest, PendingKind::Archive { tmp, root })),
            Err(e) => {
                let _ = std::fs::remove_dir_all(&tmp);
                Err(e)
            }
        }
    }

    /// Add `add` to the list. It replaces a mod of the same name and keeps
    /// that mod's position and on/off state; otherwise it is appended,
    /// enabled. Returns its name.
    pub fn commit(&mut self, mut add: PendingAdd) -> io::Result<String> {
        let name = add.manifest.name.clone();
        let at = self.mods.iter().position(|m| m.name == name);
        let enabled = at.is_none_or(|i| self.mods[i].enabled);
        let installed = self.installed(&name);
        // Move the current folder at `installed` aside, and move it back if
        // the new copy cannot be put in its place.
        let aside = self
            .dir
            .join(format!(".replaced-{}-{name}", unique_suffix()));
        let stepped = std::fs::rename(&installed, &aside).is_ok();
        let source = match add.kind.take() {
            None => return Err(io::Error::other("this add was already committed")),
            Some(PendingKind::Folder(folder)) => Source::Folder(folder),
            Some(PendingKind::Archive { tmp, root }) => {
                let moved = std::fs::create_dir_all(self.dir.join(INSTALLED))
                    .and_then(|()| std::fs::rename(&root, &installed));
                let _ = std::fs::remove_dir_all(&tmp);
                if let Err(e) = moved {
                    if stepped {
                        let _ = std::fs::rename(&aside, &installed);
                    }
                    return Err(e);
                }
                Source::Installed {
                    origin: add.origin.clone(),
                }
            }
        };
        let _ = std::fs::remove_dir_all(&aside);
        let entry = SlotMod {
            name: name.clone(),
            source,
            enabled,
        };
        match at {
            Some(i) => {
                self.mods[i] = entry;
                report::forget(&self.dir, &name);
            }
            None => self.mods.push(entry),
        }
        self.save()?;
        Ok(name)
    }

    /// [`Self::prepare`] then [`Self::commit`], refusing a name already in the
    /// list.
    pub fn add(&mut self, path: &Path, origin: Option<&str>) -> io::Result<String> {
        let add = self.prepare(path, origin)?;
        if add.replaces.is_some() {
            let name = add.manifest.name.clone();
            return Err(io::Error::other(format!(
                "this slot already has a mod named {name}"
            )));
        }
        self.commit(add)
    }
}

/// `<pid>-<nanos>`: the part of a `.add-` or `.replaced-` folder name that
/// makes it unique.
fn unique_suffix() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    )
}

/// The mod's folder in an unpacked archive: the archive's root, or its only
/// folder.
fn mod_root(tmp: &Path) -> Option<PathBuf> {
    if tmp.join(MANIFEST).is_file() {
        return Some(tmp.to_path_buf());
    }
    let dirs: Vec<PathBuf> = std::fs::read_dir(tmp)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    match dirs.as_slice() {
        [only] if only.join(MANIFEST).is_file() => Some(only.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::test_support::*;
    use crate::mods::LOADER;

    #[test]
    fn a_folder_without_mod_toml_is_refused() {
        let root = scratch("bare");
        let dir = root.join("bare");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(LOADER), "true\n").unwrap();
        let mods = SlotMods::load_in(root.join("mods"));
        assert!(mods.prepare(&dir, None).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_folder_is_named_by_its_mod_toml() {
        let root = scratch("named");
        let dev = mod_folder(&root, "dev", "dev", "1");
        let mods = SlotMods::load_in(root.join("mods"));
        let pick = mods.prepare(&dev.join(MANIFEST), None).unwrap();
        assert_eq!(pick.manifest.name, "dev");
        drop(pick);
        assert!(mods.prepare(&dev.join(LOADER), None).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_archive_installs_from_its_folder_or_its_root() {
        let root = scratch("archive");
        let a = mod_folder(&root, "a-src", "led-theme", "1");
        let b = mod_folder(&root, "b-src", "bare-mod", "1");
        tgz(&a, &root.join("wrapped.tgz"), true);
        tgz(&b, &root.join("flat.tgz"), false);
        let mut mods = SlotMods::load_in(root.join("mods"));
        assert_eq!(
            mods.add(&root.join("wrapped.tgz"), None).unwrap(),
            "led-theme"
        );
        let url = "https://x/flat.tgz";
        assert_eq!(
            mods.add(&root.join("flat.tgz"), Some(url)).unwrap(),
            "bare-mod"
        );
        assert!(root
            .join("mods/installed/led-theme")
            .join(MANIFEST)
            .is_file());
        mods.remove("led-theme").unwrap();
        assert!(!root.join("mods/installed/led-theme").exists());
        let back = SlotMods::load_in(root.join("mods"));
        let origin = url.to_string();
        assert_eq!(back.mods[0].source, Source::Installed { origin });
        let leftovers = std::fs::read_dir(root.join("mods"))
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".add-")
            })
            .count();
        assert_eq!(leftovers, 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_same_name_add_replaces_in_place() {
        let root = scratch("replace");
        let v1 = mod_folder(&root, "v1", "theme", "1.0");
        let other = mod_folder(&root, "other", "other", "1");
        let v2 = mod_folder(&root, "v2", "theme", "2.0");
        tgz(&v1, &root.join("theme-1.tgz"), true);
        let mut mods = SlotMods::load_in(root.join("mods"));
        mods.add(&root.join("theme-1.tgz"), None).unwrap();
        mods.add(&other, None).unwrap();
        mods.set_enabled("theme", false).unwrap();
        let add = mods.prepare(&v2, None).unwrap();
        assert_eq!(add.replaces.as_deref(), Some("1.0"));
        mods.commit(add).unwrap();
        assert_eq!(mods.mods[0].name, "theme");
        assert!(!mods.mods[0].enabled, "keeps its on/off");
        assert_eq!(
            mods.mods[0].source,
            Source::Folder(std::fs::canonicalize(&v2).unwrap())
        );
        assert!(
            !root.join("mods/installed/theme").exists(),
            "the installed copy is gone"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_slot_holds_at_most_ten_mods() {
        let root = scratch("cap");
        let mut mods = SlotMods::load_in(root.join("mods"));
        for i in 0..MAX_MODS {
            let dir = mod_folder(&root, &format!("m{i}"), &format!("m{i}"), "1");
            mods.add(&dir, None).unwrap();
        }
        let extra = mod_folder(&root, "extra", "extra", "1");
        assert!(mods.add(&extra, None).is_err());
        let again = mod_folder(&root, "m0-v2", "m0", "2");
        assert!(
            mods.prepare(&again, None).is_ok(),
            "a replace is not an add"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_slot_folder_cannot_replace_itself() {
        let root = scratch("self");
        let src = mod_folder(&root, "src", "theme", "1");
        tgz(&src, &root.join("theme.tgz"), true);
        let mut mods = SlotMods::load_in(root.join("mods"));
        mods.add(&root.join("theme.tgz"), None).unwrap();
        assert!(mods
            .prepare(&root.join("mods/installed/theme"), None)
            .is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn prepare_refusals_have_their_messages() {
        let root = scratch("prepare-msgs");
        let mods_dir = root.join("mods");
        let src = mod_folder(&root, "src", "theme", "1");
        tgz(&src, &root.join("theme.tgz"), true);
        let mut mods = SlotMods::load_in(mods_dir.clone());
        mods.add(&root.join("theme.tgz"), None).unwrap();

        let inside = std::fs::canonicalize(mods_dir.join("installed/theme")).unwrap();
        assert_eq!(
            mods.prepare(&inside, None).unwrap_err().to_string(),
            format!(
                "{}: this folder is inside the slot's mods folder; add the mod's archive or a \
                 copy of the folder",
                inside.display()
            )
        );
        assert_eq!(
            mods.add(&root.join("theme.tgz"), None)
                .unwrap_err()
                .to_string(),
            "this slot already has a mod named theme"
        );

        let empty = root.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::write(empty.join("readme"), "x").unwrap();
        let two = root.join("two");
        mod_folder(&two, "x", "x", "1");
        mod_folder(&two, "y", "y", "1");
        tgz(&empty, &root.join("no-manifest.tgz"), true);
        let two_folders = root.join("two.tgz");
        tar_gz(&two_folders, |tar| {
            tar.append_dir_all("x", two.join("x")).unwrap();
            tar.append_dir_all("y", two.join("y")).unwrap();
        });
        for archive in [root.join("no-manifest.tgz"), two_folders] {
            assert_eq!(
                mods.prepare(&archive, None).unwrap_err().to_string(),
                format!(
                    "{}: the archive has no mod.toml at its root",
                    archive.display()
                )
            );
        }

        let bad = root.join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join(MANIFEST), "[mod]\nname = \"bad\"\n").unwrap();
        std::fs::write(bad.join(LOADER), "true\n").unwrap();
        tgz(&bad, &root.join("bad.tgz"), false);
        assert_eq!(
            mods.prepare(&root.join("bad.tgz"), None)
                .unwrap_err()
                .to_string(),
            "mod.toml: mod.version is missing"
        );
        assert_eq!(
            mods.prepare(&bad, None).unwrap_err().to_string(),
            "mod.toml: mod.version is missing"
        );

        let missing = root.join("missing.tgz");
        let err = mods.prepare(&missing, None).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        let err = mods
            .prepare(&root.join("nowhere/mod.toml"), None)
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(add_leftovers(&mods_dir), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_cap_message_names_the_limit() {
        let root = scratch("cap-msg");
        let mut mods = SlotMods::load_in(root.join("mods"));
        for i in 0..MAX_MODS {
            let dir = mod_folder(&root, &format!("m{i}"), &format!("m{i}"), "1");
            mods.add(&dir, None).unwrap();
        }
        let extra = mod_folder(&root, "extra", "extra", "1");
        assert_eq!(
            mods.prepare(&extra, None).unwrap_err().to_string(),
            "a slot holds at most 10 mods; remove one first"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn prepare_reports_origin_and_replaced_version() {
        let root = scratch("origin");
        let src = mod_folder(&root, "src", "theme", "1");
        tgz(&src, &root.join("theme.tgz"), true);
        let mut mods = SlotMods::load_in(root.join("mods"));

        let add = mods.prepare(&root.join("theme.tgz"), None).unwrap();
        assert_eq!(add.origin, "theme.tgz");
        assert_eq!(add.replaces, None);
        drop(add);
        let add = mods
            .prepare(&root.join("theme.tgz"), Some("https://x/a\tb\nc.tgz"))
            .unwrap();
        assert_eq!(add.origin, "https://x/a b c.tgz");
        drop(add);
        let add = mods.prepare(&src.join(MANIFEST), None).unwrap();
        let canonical = std::fs::canonicalize(&src).unwrap();
        assert_eq!(add.origin, canonical.display().to_string());
        drop(add);
        assert_eq!(add_leftovers(&root.join("mods")), 0);

        mods.add(&root.join("theme.tgz"), None).unwrap();
        std::fs::remove_file(root.join("mods/installed/theme").join(MANIFEST)).unwrap();
        let add = mods.prepare(&src, None).unwrap();
        assert_eq!(add.replaces.as_deref(), Some("?"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sweep_deletes_old_leftovers_and_restores_a_half_replace() {
        let root = scratch("sweep");
        let dir = root.join("mods");
        let src = mod_folder(&root, "src", "theme", "1");
        tgz(&src, &root.join("theme.tgz"), true);
        let mut mods = SlotMods::load_in(dir.clone());
        mods.add(&root.join("theme.tgz"), None).unwrap();
        // A replace that stopped after moving the installed copy aside. The
        // leftovers are files because a folder's modification time cannot be
        // set on every host.
        std::fs::remove_dir_all(dir.join("installed/theme")).unwrap();
        let aside = dir.join(".replaced-1-2-theme");
        let old_add = dir.join(".add-1-2");
        let old_download = dir.join(".download-5");
        let old_tmp = dir.join("list.9.tmp");
        let old_other = dir.join("keep.txt");
        let old_orphan = dir.join(".replaced-1-2-ghost");
        let hour_ago = std::time::SystemTime::now() - LEFTOVER_AGE;
        for path in [
            &aside,
            &old_add,
            &old_download,
            &old_tmp,
            &old_other,
            &old_orphan,
        ] {
            std::fs::write(path, "x").unwrap();
            std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(hour_ago)
                .unwrap();
        }
        let young_add = dir.join(".add-3-4");
        std::fs::create_dir_all(&young_add).unwrap();
        mods.sweep();
        assert!(!aside.exists());
        assert!(dir.join("installed/theme").is_file());
        assert!(!old_add.exists());
        assert!(young_add.exists());
        assert!(!old_download.exists());
        assert!(!old_tmp.exists());
        assert!(old_other.exists());
        assert!(!old_orphan.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
