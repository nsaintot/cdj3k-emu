//! Building the initramfs that boots a slot with its mods.
//!
//! The enabled, compatible mods go into a second gzipped cpio archive,
//! appended to the slot's initramfs. The kernel unpacks both, in order:
//!
//! - `/opt/cdj3k-mods/manifest`: `deck_model=<slug>`, `fw_version=<release>`
//! - `/opt/cdj3k-mods/NN-<name>/`: each mod's folder, in boot order
//! - `/opt/cdj3k-mods/NN-<name>.mod`: what its `mod.toml` declares for the
//!   app, as `preload <guest path>` and `env KEY=VALUE` lines
//!
//! A boot without mods uses the slot's initramfs unchanged. The combined
//! file is kept in `mods/` and rebuilt only when the mods or the base
//! initramfs change.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use cdj3k_emu_firmware::cpio::{self, Entry};
use cdj3k_emu_panel::Model;
use sha2::{Digest, Sha256};

use super::compat::Compat;
use super::manifest;
use super::{mods_dir, SlotMods, MAX_MODS};

const GUEST_DIR: &str = "/opt/cdj3k-mods";
const CACHE: &str = "boot-initramfs.cpio.gz";
const CACHE_KEY: &str = "boot-initramfs.key";

/// The initramfs a boot uses, and the mods in it.
#[derive(Clone, Debug)]
pub struct BootMods {
    /// The file to pass to `-initrd`.
    pub initramfs: PathBuf,
    /// The mods appended, in boot order.
    pub run: Vec<String>,
    /// The slot's own mods in [`Self::run`] (not `--mod` folders), as
    /// [`SlotMods::boot_id`]s.
    pub slot_run: Vec<String>,
    /// Enabled mods left out: their `[compat]` table excludes the slot's
    /// firmware, or their files cannot be read (`Compat::Invalid`).
    pub incompatible: Vec<(String, Compat)>,
    /// Whether this process was started with `--no-mods`.
    pub no_mods: bool,
}

static CURRENT: std::sync::Mutex<Option<BootMods>> = std::sync::Mutex::new(None);

/// Record what this process's emulation is booting with.
pub fn set_current_boot(boot: BootMods) {
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(boot);
}

/// What this process's emulation last booted with.
pub fn current_boot() -> Option<BootMods> {
    CURRENT.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Whether `instance`'s own mods go into its boots: Enable Mods is on, and
/// the slot was installed with the mods runner.
pub fn slot_mods_on(instance_id: u32) -> bool {
    crate::InstanceSettings::saved_mods_enabled(instance_id) && !crate::slot_outdated(instance_id)
}

/// The initramfs for `instance`'s next boot: `base`, with the slot's enabled
/// and compatible mods appended when `enabled`, and `extra` (folders used for
/// this launch only) appended in any case. An `extra` folder takes the place
/// of a slot mod with the same name. At most [`MAX_MODS`] mods boot.
pub fn prepare_boot(
    instance_id: u32,
    base: &Path,
    model: Model,
    release: Option<&str>,
    enabled: bool,
    extra: &[PathBuf],
) -> io::Result<BootMods> {
    let slot = SlotMods::load(instance_id);
    slot.sweep();
    let candidates = slot
        .mods
        .iter()
        .filter(|m| enabled && m.enabled)
        .map(|m| (m.name.clone(), slot.dir_of(m)));
    let (mut chosen, mut incompatible) = sort_out(candidates, model, release);
    let extra = extra.iter().map(|dir| {
        let name = manifest::read(dir).map(|m| m.name).unwrap_or_else(|_| {
            dir.file_name()
                .map_or_else(|| "mod".into(), |f| f.to_string_lossy().into_owned())
        });
        (name, dir.clone())
    });
    let (extra_run, extra_incompatible) = sort_out(extra, model, release);
    let from_slot = chosen.len();
    let mut replaced = Vec::new();
    for (name, dir) in extra_run {
        if let Some(i) = chosen[..from_slot].iter().position(|(n, _)| *n == name) {
            eprintln!(
                "cdj3k-emu: --mod {} boots in place of the slot's mod {name}",
                dir.display()
            );
            chosen[i].1 = dir;
            replaced.push(name);
        } else {
            chosen.push((name, dir));
        }
    }
    incompatible.extend(extra_incompatible);
    for (name, _) in chosen.iter().skip(MAX_MODS) {
        eprintln!("cdj3k-emu: mod {name} skipped: a boot runs at most {MAX_MODS} mods");
    }
    chosen.truncate(MAX_MODS);
    let (packed, unreadable) = pack(&chosen);
    let chosen: Vec<(String, PathBuf)> = chosen
        .into_iter()
        .filter(|(name, _)| !unreadable.iter().any(|(n, _)| n == name))
        .collect();
    incompatible.extend(unreadable);
    let initramfs = if chosen.is_empty() {
        base.to_path_buf()
    } else {
        let overlay = overlay(&packed, model, release)?;
        cached_combined(&mods_dir(instance_id), base, &overlay)?
    };
    let slot_run = ids(
        &slot,
        chosen
            .iter()
            .filter(|(name, _)| !replaced.contains(name))
            .map(|(name, _)| name),
    );
    Ok(BootMods {
        initramfs,
        run: chosen.into_iter().map(|(n, _)| n).collect(),
        slot_run,
        incompatible,
        no_mods: false,
    })
}

/// The slot's mods that the next boot would run, in order, as
/// [`SlotMods::boot_id`]s: the same selection [`prepare_boot`] makes.
pub fn planned_slot_run(
    instance_id: u32,
    model: Model,
    release: Option<&str>,
    enabled: bool,
) -> Vec<String> {
    let slot = SlotMods::load(instance_id);
    let candidates = slot
        .mods
        .iter()
        .filter(|m| enabled && m.enabled)
        .map(|m| (m.name.clone(), slot.dir_of(m)));
    let chosen = sort_out(candidates, model, release).0;
    ids(&slot, chosen.iter().take(MAX_MODS).map(|(name, _)| name))
}

/// The [`SlotMods::boot_id`]s of the mods of `slot` named in `names`.
fn ids<'a>(slot: &SlotMods, names: impl Iterator<Item = &'a String>) -> Vec<String> {
    names
        .filter_map(|name| slot.get(name).map(|m| slot.boot_id(m)))
        .collect()
}

type Sorted = (Vec<(String, PathBuf)>, Vec<(String, Compat)>);

/// Split mods into those that run on the slot's firmware and those that do
/// not, each in order.
fn sort_out(
    mods: impl Iterator<Item = (String, PathBuf)>,
    model: Model,
    release: Option<&str>,
) -> Sorted {
    let mut run: Vec<(String, PathBuf)> = Vec::new();
    let mut incompatible = Vec::new();
    for (name, dir) in mods {
        if run.iter().any(|(n, _)| *n == name) {
            continue;
        }
        let compat = manifest::check_dir(&dir, model, release);
        if compat.runs() {
            run.push((name, dir));
        } else {
            incompatible.push((name, compat));
        }
    }
    (run, incompatible)
}

/// A mod's files and what its `mod.toml` passes to the app, read from its
/// folder.
pub struct Packed {
    name: String,
    /// Paths relative to the mod's folder.
    files: Vec<Entry>,
    declared: manifest::Manifest,
}

/// Read each mod's folder. Returns the mods read, and the others as
/// `Compat::Invalid` with the reason.
pub fn pack(mods: &[(String, PathBuf)]) -> (Vec<Packed>, Vec<(String, Compat)>) {
    let mut packed = Vec::new();
    let mut failed = Vec::new();
    for (name, dir) in mods {
        let read = (|| {
            let declared = manifest::read(dir)?;
            let mut files = Vec::new();
            let mut seen = Vec::new();
            walk(dir, Path::new(""), &mut files, &mut seen).map_err(|e| e.to_string())?;
            Ok::<_, String>(Packed {
                name: name.clone(),
                files,
                declared,
            })
        })();
        match read {
            Ok(p) => packed.push(p),
            Err(e) => {
                eprintln!("cdj3k-emu: mod {name} skipped: {e}");
                failed.push((name.clone(), Compat::Invalid(e)));
            }
        }
    }
    (packed, failed)
}

/// The gzipped cpio archive holding `mods` and the manifest.
pub fn overlay(mods: &[Packed], model: Model, release: Option<&str>) -> io::Result<Vec<u8>> {
    let mut entries = vec![
        Entry::dir(GUEST_DIR, 0o755),
        Entry::file(
            format!("{GUEST_DIR}/manifest"),
            format!(
                "deck_model={}\nfw_version={}\n",
                model.slug(),
                release.unwrap_or("")
            )
            .into_bytes(),
            0o644,
        ),
    ];
    for (i, m) in mods.iter().enumerate() {
        let guest = format!("{GUEST_DIR}/{:02}-{}", i + 1, m.name);
        entries.push(Entry::dir(&guest, 0o755));
        for file in &m.files {
            let mut file = file.clone();
            file.path = Path::new(&guest).join(&file.path);
            entries.push(file);
        }
        let declared = &m.declared;
        let mut lines = String::new();
        for lib in &declared.preload {
            lines.push_str(&format!("preload {guest}/{lib}\n"));
        }
        for (key, value) in &declared.env {
            lines.push_str(&format!("env {key}={value}\n"));
        }
        entries.push(Entry::file(
            format!("{guest}.mod"),
            lines.into_bytes(),
            0o644,
        ));
    }
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&cpio::write(&entries))?;
    gz.finish()
}

/// Add the files under `dir` to `entries` at the path `guest`, following
/// symlinks and skipping `.git`. `seen` holds the folders above `dir`, to
/// refuse a symlink that loops back to one of them.
fn walk(
    dir: &Path,
    guest: &Path,
    entries: &mut Vec<Entry>,
    seen: &mut Vec<PathBuf>,
) -> io::Result<()> {
    let real = std::fs::canonicalize(dir)?;
    if seen.contains(&real) {
        return Err(io::Error::other(format!(
            "{}: a symlink loops back to a folder above it",
            dir.display()
        )));
    }
    seen.push(real);
    let mut children: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    children.sort_by_key(|e| e.file_name());
    for child in children {
        let file_name = child.file_name();
        if file_name == ".git" {
            continue;
        }
        let path = child.path();
        let meta = std::fs::metadata(&path)
            .map_err(|e| io::Error::other(format!("{}: {e}", path.display())))?;
        let target = guest.join(&file_name);
        let mode = cdj3k_emu_firmware::file_mode::permissions(&meta);
        if meta.is_dir() {
            entries.push(Entry::dir(&target, mode));
            walk(&path, &target, entries, seen)?;
        } else if meta.is_file() {
            let data = std::fs::read(&path)
                .map_err(|e| io::Error::other(format!("{}: {e}", path.display())))?;
            entries.push(Entry::file(&target, data, mode));
        } else {
            return Err(io::Error::other(format!(
                "{}: not a file or a folder",
                path.display()
            )));
        }
    }
    seen.pop();
    Ok(())
}

/// A file in `dir` holding `base` followed by `overlay`, reused while both
/// are unchanged.
fn cached_combined(dir: &Path, base: &Path, overlay: &[u8]) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let meta = std::fs::metadata(base)?;
    let mut hash = Sha256::new();
    hash.update(base.to_string_lossy().as_bytes());
    hash.update(meta.len().to_le_bytes());
    if let Ok(modified) = meta.modified() {
        if let Ok(since) = modified.duration_since(std::time::UNIX_EPOCH) {
            hash.update(since.as_nanos().to_le_bytes());
        }
    }
    hash.update(overlay);
    let key = hex(&hash.finalize());

    let out = dir.join(CACHE);
    let key_path = dir.join(CACHE_KEY);
    if out.is_file() && std::fs::read_to_string(&key_path).is_ok_and(|k| k == key) {
        return Ok(out);
    }
    let tmp = dir.join(format!("{CACHE}.{}.tmp", std::process::id()));
    std::fs::copy(base, &tmp)?;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&tmp)?
        .write_all(overlay)?;
    // Delete the key first: if the app stops before the new key is written,
    // the next boot rebuilds the file instead of reusing it.
    match std::fs::remove_file(&key_path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    std::fs::rename(&tmp, &out)?;
    std::fs::write(&key_path, key)?;
    Ok(out)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cdj3k-boot-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn unpacked(gz: &[u8]) -> Vec<Entry> {
        let mut raw = Vec::new();
        io::Read::read_to_end(&mut flate2::read::GzDecoder::new(gz), &mut raw).unwrap();
        cpio::read(&raw).unwrap()
    }

    #[test]
    fn the_overlay_holds_the_manifest_and_each_mod_in_order() {
        let root = scratch("overlay");
        for name in ["a", "b"] {
            std::fs::create_dir_all(root.join(name).join("lib")).unwrap();
            std::fs::write(
                root.join(name).join("mod.toml"),
                format!(
                    "[mod]\nname = \"{name}\"\nversion = \"1\"\npreload = [\"lib/x.so\"]\n\
                     env = {{ K = \"v w\" }}\n"
                ),
            )
            .unwrap();
            std::fs::write(root.join(name).join("lib/x.so"), [0u8; 3]).unwrap();
        }
        std::fs::create_dir_all(root.join("a/.git")).unwrap();
        let mods = [
            ("b".to_string(), root.join("b")),
            ("a".to_string(), root.join("a")),
        ];
        let (packed, failed) = pack(&mods);
        assert!(failed.is_empty());
        let gz = overlay(&packed, Model::Cdj3kx, Some("1.40")).unwrap();
        let entries = unpacked(&gz);
        let paths: Vec<String> = entries
            .iter()
            .map(|e| e.path.display().to_string())
            .collect();
        assert_eq!(
            paths,
            [
                "/opt/cdj3k-mods",
                "/opt/cdj3k-mods/manifest",
                "/opt/cdj3k-mods/01-b",
                "/opt/cdj3k-mods/01-b/lib",
                "/opt/cdj3k-mods/01-b/lib/x.so",
                "/opt/cdj3k-mods/01-b/mod.toml",
                "/opt/cdj3k-mods/01-b.mod",
                "/opt/cdj3k-mods/02-a",
                "/opt/cdj3k-mods/02-a/lib",
                "/opt/cdj3k-mods/02-a/lib/x.so",
                "/opt/cdj3k-mods/02-a/mod.toml",
                "/opt/cdj3k-mods/02-a.mod",
            ]
        );
        assert_eq!(
            entries[6].kind,
            cpio::Kind::File(b"preload /opt/cdj3k-mods/01-b/lib/x.so\nenv K=v w\n".to_vec())
        );
        assert_eq!(
            entries[1].kind,
            cpio::Kind::File(b"deck_model=cdj3kx\nfw_version=1.40\n".to_vec())
        );
        assert!(entries.iter().all(|e| e.uid == 0 && e.gid == 0));
        assert_eq!(
            gz,
            overlay(&pack(&mods).0, Model::Cdj3kx, Some("1.40")).unwrap(),
            "reproducible"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_mod_that_cannot_be_read_is_left_out_alone() {
        let root = scratch("unreadable");
        for name in ["good", "looped", "dangling"] {
            std::fs::create_dir_all(root.join(name)).unwrap();
            std::fs::write(
                root.join(name).join("mod.toml"),
                format!("[mod]\nname = \"{name}\"\nversion = \"1\"\n"),
            )
            .unwrap();
        }
        std::os::unix::fs::symlink(".", root.join("looped/self")).unwrap();
        std::os::unix::fs::symlink("nowhere", root.join("dangling/link")).unwrap();
        let mods: Vec<(String, PathBuf)> = ["good", "looped", "dangling"]
            .iter()
            .map(|n| (n.to_string(), root.join(n)))
            .collect();
        let (packed, failed) = pack(&mods);
        assert_eq!(packed.len(), 1);
        assert_eq!(packed[0].name, "good");
        let names: Vec<&str> = failed.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["looped", "dangling"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_combined_file_is_the_base_then_the_overlay() {
        let root = scratch("combined");
        let base = root.join("base.gz");
        std::fs::write(&base, b"BASE").unwrap();
        let out = cached_combined(&root.join("mods"), &base, b"OVERLAY").unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"BASEOVERLAY");
        let modified = std::fs::metadata(&out).unwrap().modified().unwrap();
        let again = cached_combined(&root.join("mods"), &base, b"OVERLAY").unwrap();
        assert_eq!(
            std::fs::metadata(&again).unwrap().modified().unwrap(),
            modified
        );
        let other = cached_combined(&root.join("mods"), &base, b"OTHER").unwrap();
        assert_eq!(std::fs::read(&other).unwrap(), b"BASEOTHER");
        std::fs::remove_dir_all(root).unwrap();
    }
}
