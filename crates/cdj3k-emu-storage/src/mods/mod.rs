//! A slot's mods: folders with a `mod.toml` at their root ([`manifest`]),
//! which the guest loads once per boot before the player app (initramfs
//! patch 30).
//!
//! `instance-N/mods/` holds:
//!
//! - `list.txt`: the order, one mod per line, `on|off <name>\t<origin>` for an
//!   installed mod and `on|off <absolute path>` for a folder used in place
//! - `installed/<name>/`: an installed mod, unpacked from its archive
//! - `boot-initramfs.cpio.gz`: the slot's initramfs with the last boot's mods
//!   appended ([`boot`])
//! - `last-boot.txt`: the guest's mods report for the last boot, as the
//!   runtime received it, followed by an `id <boot id>` line for each slot
//!   mod that boot ran ([`SlotMods::boot_id`])
//! - `last-boot/<name>.log`: each mod's journal from that boot

pub mod boot;
pub mod compat;
pub mod manifest;

use std::io;
use std::path::{Path, PathBuf};

use crate::instance_dir;

pub use boot::{
    current_boot, planned_slot_run, prepare_boot, set_current_boot, slot_mods_on, BootMods,
};
pub use compat::Compat;
pub use manifest::{Manifest, FILE as MANIFEST, LOADER};

const LIST: &str = "list.txt";
const LAST_BOOT: &str = "last-boot.txt";
const LAST_BOOT_LOGS: &str = "last-boot";
/// The boot ids file of earlier versions, deleted when the next report is
/// saved.
const OLD_LAST_BOOT_IDS: &str = "last-boot-mods.txt";
const ID_LINE: &str = "id ";
const INSTALLED: &str = "installed";
/// The most mods a slot holds.
pub const MAX_MODS: usize = 10;
/// The maximum total size an archive may unpack to.
const UNPACKED_LIMIT: u64 = 1 << 30;
/// The most files and folders an archive may hold.
const ENTRY_LIMIT: usize = 10_000;
/// How old a leftover from an add, a replace or a download must be before
/// [`SlotMods::sweep`] deletes it.
const LEFTOVER_AGE: std::time::Duration = std::time::Duration::from_secs(3600);

/// `instance-N/mods`.
pub fn mods_dir(instance_id: u32) -> PathBuf {
    instance_dir(instance_id).join("mods")
}

/// The guest's mods report for the slot's last boot, as received.
pub fn last_boot_report(instance_id: u32) -> Option<String> {
    let text = std::fs::read_to_string(mods_dir(instance_id).join(LAST_BOOT)).ok()?;
    Some(
        text.lines()
            .filter(|l| !l.starts_with(ID_LINE))
            .map(|l| format!("{l}\n"))
            .collect(),
    )
}

/// Save `text` as the slot's last-boot report, with `ids`, the
/// [`SlotMods::boot_id`]s of the slot mods that boot ran, in the same file.
/// The journals of the previous boot are deleted.
pub fn save_last_boot_report(instance_id: u32, text: &str, ids: &[String]) -> io::Result<()> {
    let dir = mods_dir(instance_id);
    std::fs::create_dir_all(&dir)?;
    let _ = std::fs::remove_dir_all(dir.join(LAST_BOOT_LOGS));
    let _ = std::fs::remove_file(dir.join(OLD_LAST_BOOT_IDS));
    let mut all: String = text.lines().map(|l| format!("{l}\n")).collect();
    for id in ids {
        all.push_str(&format!("{ID_LINE}{id}\n"));
    }
    write_atomic(&dir.join(LAST_BOOT), &all)
}

/// The [`SlotMods::boot_id`]s of the slot mods that the last report covers.
pub fn last_boot_ids(instance_id: u32) -> Vec<String> {
    std::fs::read_to_string(mods_dir(instance_id).join(LAST_BOOT))
        .map(|text| {
            text.lines()
                .filter_map(|l| l.strip_prefix(ID_LINE))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Write `text` to `path` through a temporary file, so a reader sees the old
/// file or the new one.
fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// `name`'s journal from the slot's last boot.
pub fn last_boot_log(instance_id: u32, name: &str) -> Option<String> {
    std::fs::read_to_string(
        mods_dir(instance_id)
            .join(LAST_BOOT_LOGS)
            .join(format!("{name}.log")),
    )
    .ok()
}

pub fn save_last_boot_log(instance_id: u32, name: &str, text: &str) -> io::Result<()> {
    let dir = mods_dir(instance_id).join(LAST_BOOT_LOGS);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(format!("{name}.log")), text)
}

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
    dir: PathBuf,
    pub mods: Vec<SlotMod>,
}

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
    pub fn load(instance_id: u32) -> Self {
        Self::load_in(mods_dir(instance_id))
    }

    fn load_in(dir: PathBuf) -> Self {
        let text = std::fs::read_to_string(dir.join(LIST)).unwrap_or_default();
        let mut mods: Vec<SlotMod> = Vec::new();
        let entries = text.lines().filter_map(|line| {
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
        });
        // A folder's name comes from its mod.toml, so editing it can repeat a
        // name already in the list. The first entry with a name wins.
        for m in entries {
            if mods.iter().any(|kept| kept.name == m.name) {
                eprintln!(
                    "cdj3k-emu: {}: a second mod named {}, skipped",
                    dir.display(),
                    m.name
                );
            } else {
                mods.push(m);
            }
        }
        Self { dir, mods }
    }

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

    fn installed(&self, name: &str) -> PathBuf {
        self.dir.join(INSTALLED).join(name)
    }

    /// An id for the copy of `m` a boot runs: its name, where it comes from,
    /// and its version. A mod with the same name from another source, or in
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

    /// Read the mod at `path`: a `.tgz`/`.tar.gz`/`.tar` (holding the mod
    /// folder or its contents), a folder used in place, or the folder's
    /// `mod.toml`. `origin` is the source the list shows for an archive; its
    /// file name by default.
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
            Some(folder) => {
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
                (manifest, PendingKind::Folder(folder), origin)
            }
            None => {
                std::fs::create_dir_all(&self.dir)?;
                let tmp = self.dir.join(format!(
                    ".add-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_nanos())
                ));
                let read = unpack(path, &tmp).and_then(|()| {
                    let root = mod_root(&tmp).ok_or_else(|| {
                        io::Error::other(format!("{}: no {MANIFEST} at its root", path.display()))
                    })?;
                    let manifest = manifest::read(&root).map_err(io::Error::other)?;
                    Ok((manifest, root))
                });
                let (manifest, root) = match read {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = std::fs::remove_dir_all(&tmp);
                        return Err(e);
                    }
                };
                let origin = origin
                    .map(str::to_string)
                    .or(file)
                    .unwrap_or_else(|| path.display().to_string());
                (manifest, PendingKind::Archive { tmp, root }, origin)
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
        let aside = self.dir.join(format!(
            ".replaced-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
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
                self.forget(&name);
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

    /// Drop `name` from the list. An installed mod's files are deleted; a
    /// folder used in place is left as it is.
    pub fn remove(&mut self, name: &str) -> io::Result<()> {
        let Some(at) = self.mods.iter().position(|m| m.name == name) else {
            return Ok(());
        };
        let m = self.mods.remove(at);
        self.save()?;
        self.forget(&m.name);
        if matches!(m.source, Source::Installed { .. }) {
            match std::fs::remove_dir_all(self.installed(&m.name)) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        Ok(())
    }

    /// Delete what the last boot reported about `name`: its report line, its
    /// boot id and its journal. Best effort: a file left behind only causes a
    /// stale status.
    fn forget(&self, name: &str) {
        let keep = |file: &str, gone: &dyn Fn(&str) -> bool| {
            let path = self.dir.join(file);
            if let Ok(text) = std::fs::read_to_string(&path) {
                let kept: String = text
                    .lines()
                    .filter(|l| !gone(l))
                    .map(|l| format!("{l}\n"))
                    .collect();
                if kept != text {
                    let _ = write_atomic(&path, &kept);
                }
            }
        };
        keep(LAST_BOOT, &|l| {
            let reported = l
                .strip_prefix("mod ")
                .and_then(|rest| rest.split_whitespace().next());
            let id = l.strip_prefix(ID_LINE).and_then(|id| id.split('\t').next());
            reported == Some(name) || id == Some(name)
        });
        let _ = std::fs::remove_file(self.dir.join(LAST_BOOT_LOGS).join(format!("{name}.log")));
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

/// Unpack `archive` into `into`: files and folders only, at most
/// [`UNPACKED_LIMIT`] in all.
fn unpack(archive: &Path, into: &Path) -> io::Result<()> {
    std::fs::create_dir_all(into)?;
    let file = std::fs::File::open(archive)?;
    let read: Box<dyn io::Read> = if archive.to_string_lossy().ends_with(".tar") {
        Box::new(file)
    } else {
        Box::new(flate2::read::GzDecoder::new(file))
    };
    let fail = |what: String| io::Error::other(format!("{}: {what}", archive.display()));
    // Tar headers count towards the limit too: a long-name header's body is
    // read into memory before its entry is seen.
    let read = Capped {
        inner: read,
        left: UNPACKED_LIMIT,
    };
    let mut tar = tar::Archive::new(read);
    let mut total = 0u64;
    for (count, entry) in tar.entries().map_err(|e| fail(e.to_string()))?.enumerate() {
        if count >= ENTRY_LIMIT {
            return Err(fail(format!(
                "holds more than {ENTRY_LIMIT} files and folders"
            )));
        }
        let mut entry = entry.map_err(|e| fail(e.to_string()))?;
        let path = entry
            .path()
            .map_or_else(|_| "?".into(), |p| p.display().to_string());
        let kind = entry.header().entry_type();
        // Archives from `git archive` and GitHub start with a pax header that
        // names the commit.
        if kind.is_pax_global_extensions() || kind.is_pax_local_extensions() {
            continue;
        }
        if !(kind.is_file() || kind.is_dir()) {
            return Err(fail(format!(
                "{path}: not a file or a folder; a mod archive may contain only those"
            )));
        }
        // Unpacking applies the entry's mode. An entry its owner cannot read
        // could be neither booted nor removed.
        let needs = if kind.is_dir() { 0o700 } else { 0o600 };
        if entry.header().mode().is_ok_and(|m| m & needs != needs) {
            return Err(fail(format!("{path}: its owner cannot read it")));
        }
        total += entry.size();
        if total > UNPACKED_LIMIT {
            return Err(fail(format!(
                "unpacks to more than {} MiB",
                UNPACKED_LIMIT >> 20
            )));
        }
        if !entry
            .unpack_in(into)
            .map_err(|e| fail(format!("{path}: {e}")))?
        {
            return Err(fail(format!("{path}: outside the archive")));
        }
    }
    Ok(())
}

/// A reader that fails once `left` bytes have been read.
struct Capped<R> {
    inner: R,
    left: u64,
}

impl<R: io::Read> io::Read for Capped<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 {
            return Err(io::Error::other(format!(
                "unpacks to more than {} MiB",
                UNPACKED_LIMIT >> 20
            )));
        }
        let max = buf
            .len()
            .min(usize::try_from(self.left).unwrap_or(usize::MAX));
        let n = self.inner.read(&mut buf[..max])?;
        self.left -= n as u64;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cdj3k-mods-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn mod_folder(parent: &Path, folder: &str, name: &str, version: &str) -> PathBuf {
        let dir = parent.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(MANIFEST),
            format!("[mod]\nname = \"{name}\"\nversion = \"{version}\"\n"),
        )
        .unwrap();
        std::fs::write(dir.join(LOADER), "echo hi\n").unwrap();
        dir
    }

    fn tgz(src: &Path, dest: &Path, wrapped: bool) {
        let gz = flate2::write::GzEncoder::new(
            std::fs::File::create(dest).unwrap(),
            flate2::Compression::fast(),
        );
        let mut tar = tar::Builder::new(gz);
        if wrapped {
            tar.append_dir_all("folder", src).unwrap();
        } else {
            for f in [MANIFEST, LOADER] {
                tar.append_path_with_name(src.join(f), f).unwrap();
            }
        }
        tar.into_inner().unwrap().finish().unwrap();
    }

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
    fn an_archive_with_a_link_is_refused() {
        let root = scratch("link");
        let src = mod_folder(&root, "src", "linked", "1");
        let gz = flate2::write::GzEncoder::new(
            std::fs::File::create(root.join("linked.tgz")).unwrap(),
            flate2::Compression::fast(),
        );
        let mut tar = tar::Builder::new(gz);
        tar.append_path_with_name(src.join(MANIFEST), MANIFEST)
            .unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        tar.append_link(&mut header, "secret", "/etc/passwd")
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        let mods = SlotMods::load_in(root.join("mods"));
        let err = mods.prepare(&root.join("linked.tgz"), None).unwrap_err();
        assert!(err.to_string().contains("secret"), "{err}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_git_archive_installs() {
        let root = scratch("git-archive");
        let src = mod_folder(&root, "src", "from-git", "1");
        let archive = root.join("from-git.tar.gz");
        let gz = flate2::write::GzEncoder::new(
            std::fs::File::create(&archive).unwrap(),
            flate2::Compression::fast(),
        );
        let mut tar = tar::Builder::new(gz);
        let comment = b"52 comment=0123456789abcdef0123456789abcdef01234567\n";
        let mut header = tar::Header::new_ustar();
        header.set_path("pax_global_header").unwrap();
        header.set_entry_type(tar::EntryType::XGlobalHeader);
        header.set_size(comment.len() as u64);
        header.set_cksum();
        tar.append(&header, &comment[..]).unwrap();
        for f in [MANIFEST, LOADER] {
            tar.append_path_with_name(src.join(f), format!("from-git/{f}"))
                .unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
        let mut mods = SlotMods::load_in(root.join("mods"));
        assert_eq!(mods.add(&archive, None).unwrap(), "from-git");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_archive_entry_its_owner_cannot_read_is_refused() {
        let root = scratch("mode");
        let src = mod_folder(&root, "src", "locked", "1");
        let gz = flate2::write::GzEncoder::new(
            std::fs::File::create(root.join("locked.tgz")).unwrap(),
            flate2::Compression::fast(),
        );
        let mut tar = tar::Builder::new(gz);
        tar.append_path_with_name(src.join(MANIFEST), MANIFEST)
            .unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Directory);
        header.set_mode(0o000);
        header.set_size(0);
        tar.append_data(&mut header, "lib", std::io::empty())
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        let mods = SlotMods::load_in(root.join("mods"));
        let err = mods.prepare(&root.join("locked.tgz"), None).unwrap_err();
        assert!(err.to_string().contains("lib"), "{err}");
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
}
