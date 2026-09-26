//! Provision the initramfs by patching it inside a guest.
//!
//! The host-side pipeline in [`crate::initramfs`] needs a POSIX host, and a
//! non-root unpack drops setuid bits (`busybox`, `ping`, `fusermount`). The
//! aarch64 guest has bash, sed and a filesystem that holds every mode the
//! firmware needs, so the patch scripts run there unchanged:
//!
//! 1. The host writes a small cpio holding the patch scripts, the guest
//!    binaries and a `/cdj3k-init`, and concatenates it after the pristine
//!    archive. The kernel merges the two as it unpacks.
//! 2. `rdinit=/cdj3k-init` runs the ordinary `patch-rootfs.sh` against `/`,
//!    tars the result onto a scratch disk, and powers off.
//! 3. The host turns that tar back into the initramfs the emulator boots.
//!
//! This happens once, when firmware is installed. Normal boots are untouched.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use flate2::write::GzEncoder;
use flate2::Compression;
use tar::EntryType;

use crate::cpio::{self, Entry, Kind};
use crate::initramfs::PatchError;

/// Where the provisioning init is installed, and what `rdinit=` must name.
pub const PRELUDE_PATH: &str = "/cdj3k-init";
/// Where the patch set is staged inside the guest.
const STAGE: &str = "/cdj3k-patch";
/// The symlink manifest the guest leaves for [`repair_symlinks`].
const LINKS: &str = "/cdj3k-links";

/// Runs the provisioning guest.
///
/// Implemented by the caller because this crate does not own QEMU: it is
/// handed a kernel and an initramfs, boots them with `scratch` attached as a
/// block device, and returns once the guest has powered off.
pub trait GuestRunner {
    fn run(&self, kernel: &Path, initramfs: &Path, scratch: &Path) -> Result<(), String>;
}

/// Everything the provisioning boot needs that `patch_initramfs` does not take.
pub struct GuestProvision<'a> {
    /// Kernel to boot the provisioning guest with.
    pub kernel: &'a Path,
    /// Scratch file the guest hands the patched rootfs back on. Must already
    /// be large enough for the uncompressed rootfs.
    pub scratch: &'a Path,
    pub runner: &'a dyn GuestRunner,
}

/// Provision `initramfs_gz` into `out_path` by patching it inside a guest.
pub fn patch_initramfs_in_guest(
    initramfs_gz: &Path,
    resources_dir: &Path,
    out_path: &Path,
    guest: &GuestProvision,
) -> Result<(), PatchError> {
    let started = std::time::Instant::now();
    let patch_dir = resources_dir.join("patch");
    let tools_dir = resources_dir.join("tools");
    if !patch_dir.join("patch-rootfs.sh").is_file() {
        return Err(PatchError::MissingResource(
            patch_dir.join("patch-rootfs.sh").display().to_string(),
        ));
    }

    // 1. Stage the patch set into a second cpio after the pristine one.
    let staged = out_path.with_extension("provision.cpio.gz");
    stage_provisioning_image(initramfs_gz, &patch_dir, &tools_dir, &staged)?;

    // 2. Let the guest do the work.
    guest
        .runner
        .run(guest.kernel, &staged, guest.scratch)
        .map_err(PatchError::CommandFailed)?;

    // 3. Turn what came back into the image the emulator boots.
    let entries = read_handback(guest.scratch)?;
    write_cpio_gz(&entries, out_path)?;
    let _ = std::fs::remove_file(&staged);

    eprintln!(
        "[initramfs] guest provision: {} entries in {:.1}s",
        entries.len(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

// ── 1. Staging ────────────────────────────────────────────────────────────────

/// `pristine ++ {patch scripts, tools, /cdj3k-init}`.
///
/// The pristine bytes are copied, never rebuilt. They are trimmed to the end of
/// their gzip member first: an archive carved out of a `.UPD` carries slack
/// past its own, and the kernel stops unpacking at the first thing that is not
/// a member — so anything appended past the slack is never seen.
fn stage_provisioning_image(
    initramfs_gz: &Path,
    patch_dir: &Path,
    tools_dir: &Path,
    out: &Path,
) -> Result<(), PatchError> {
    let compressed = std::fs::read(initramfs_gz)?;
    let member_end = measure_pristine(&compressed)?.member_end;

    let mut entries = Vec::new();
    stage_tree(patch_dir, STAGE, &mut entries);
    stage_tree(tools_dir, &format!("{STAGE}/tools"), &mut entries);
    entries.push(Entry::file(
        PRELUDE_PATH,
        provisioning_init().into_bytes(),
        0o755,
    ));

    let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
    enc.write_all(&cpio::write(&entries))?;
    let overlay = enc.finish()?;

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::File::create(out)?;
    f.write_all(&compressed[..member_end])?;
    f.write_all(&overlay)?;
    Ok(())
}

fn stage_tree(dir: &Path, at: &str, out: &mut Vec<Entry>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    out.push(Entry::dir(at, 0o755));
    for e in rd.filter_map(|e| e.ok()) {
        let p = e.path();
        let name = p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let child = format!("{at}/{name}");
        if p.is_dir() {
            stage_tree(&p, &child, out);
        } else if let Ok(data) = std::fs::read(&p) {
            // Derived from the name, not from the host: Windows has no mode
            // bits to copy, and only the dispatcher and its steps are run.
            let mode = if name.ends_with(".sh") { 0o755 } else { 0o644 };
            out.push(Entry::file(child, data, mode));
        }
    }
}

/// Printed by the provisioning guest when a patch step fails.
const PATCHING_FAILED: &str = "=== cdj3k: PATCHING FAILED";
/// Printed by the provisioning guest once the rootfs is written back, with
/// the handback's exit status after it.
const HANDED_BACK: &str = "=== cdj3k: handed back rc=";

/// The outcome the provisioning guest reported in its console `log`.
///
/// A patch failure powers off without writing, so success is the handback
/// marker with status 0, not the absence of a failure.
pub fn guest_outcome(log: &str) -> Result<(), &'static str> {
    if log.contains(PATCHING_FAILED) {
        Err("the provisioning guest failed to patch the rootfs")
    } else if !log.contains(&format!("{HANDED_BACK}0 ===")) {
        Err("the provisioning guest did not hand back a rootfs")
    } else {
        Ok(())
    }
}

/// The script `rdinit=` runs, in the firmware's own Linux userspace.
fn provisioning_init() -> String {
    [
        "#!/usr/bin/bash".into(),
        "set +e".into(),
        // Real device nodes before anything redirects to /dev/null, or it
        // becomes a regular file in the image.
        "/usr/bin/mount -t devtmpfs devtmpfs /dev".into(),
        "/usr/bin/mount -t proc proc /proc".into(),
        // Guest binaries into place: `patch_initramfs` does this in Rust, so
        // no patch step owns it.
        format!("/usr/bin/cp {STAGE}/tools/deck_shim.so /home/root/"),
        format!(
            "for t in {STAGE}/tools/*; do case \"$t\" in *deck_shim.so) ;; \
             *) /usr/bin/cp \"$t\" /usr/bin/ ;; esac; done"
        ),
        "/usr/bin/chmod 755 /home/root/deck_shim.so /usr/bin/subucom_* 2>/dev/null".into(),
        format!("export ROOTFS=/ PATCH_ASSETS_DIR={STAGE} PATCH_TOOLS_DIR={STAGE}/tools"),
        format!("/usr/bin/bash {STAGE}/patch-rootfs.sh /"),
        "rc=$?".into(),
        format!("/usr/bin/rm -rf {STAGE}"),
        // Hand back nothing rather than a half-patched rootfs.
        format!(
            r#"if [ "$rc" != 0 ]; then echo "{PATCHING_FAILED} ($rc) ==="; /usr/bin/busybox poweroff -f; fi"#
        ),
        // The scratch disk is mmcblk1, not vda: the kernel renames virtio-blk
        // so the firmware finds its eMMC where it expects one.
        "/usr/bin/mkdir -p /cdj3k-dev".into(),
        "/usr/bin/mount -t devtmpfs devtmpfs /cdj3k-dev".into(),
        // Drop the live device tree so the image keeps the /dev the firmware
        // shipped — one console node.
        "/usr/bin/umount /dev".into(),
        "/usr/bin/rm -f /dev/null".into(),
        // tar's linkname field is 100 bytes and busybox writes no @LongLink for
        // targets, so long symlinks arrive truncated. Ship the real targets.
        // busybox find has no GNU `-not`; `!` is the portable spelling.
        "/usr/bin/find / -type l ! -path '/proc/*' ! -path '/cdj3k-dev/*' > /cdj3k-lp".into(),
        // One line per link, `path<TAB>target`: pairing across two lines
        // silently desynchronises everything after a link that reads back
        // empty.
        format!(
            "while read -r l; do printf '%s\\t%s\\n' \"$l\" \"$(/usr/bin/readlink \"$l\")\";              done < /cdj3k-lp > {LINKS}"
        ),
        "/usr/bin/rm -f /cdj3k-lp".into(),
        // busybox cpio here is extract-only, so the rootfs comes back as tar.
        "/usr/bin/tar -cf /cdj3k-dev/mmcblk1 --exclude=./cdj3k-init \
         --exclude=./cdj3k-dev --exclude=./proc -C / ."
            .into(),
        format!("echo \"{HANDED_BACK}$? ===\""),
        "/usr/bin/sync".into(),
        "/usr/bin/busybox poweroff -f".into(),
    ]
    .join("\n")
        + "\n"
}

// ── 3. Reading the handback ───────────────────────────────────────────────────

/// Parse the tar the guest wrote and rebuild cpio entries from it.
///
/// tar carries everything a cpio header needs — mode, uid/gid, symlink
/// targets, device nodes — so the crossing is lossless apart from the two
/// cases handled here: hard links, which cpio can only express with an
/// ino/nlink model this codec does not have, and over-long symlink targets.
fn read_handback(scratch: &Path) -> Result<Vec<Entry>, PatchError> {
    let mut archive = tar::Archive::new(std::fs::File::open(scratch)?);
    let mut entries: Vec<Entry> = Vec::new();
    let mut seen: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut links_manifest: Option<String> = None;

    for e in archive.entries()? {
        let mut e = e?;
        let header = e.header().clone();
        let path = normalise(&e.path()?.to_string_lossy());
        let mode = header.mode().unwrap_or(0o644) & 0o7777;

        let kind = match header.entry_type() {
            EntryType::Regular | EntryType::Continuous => {
                let mut data = Vec::new();
                e.read_to_end(&mut data)?;
                if path == LINKS {
                    links_manifest = Some(String::from_utf8_lossy(&data).into_owned());
                    continue;
                }
                seen.insert(path.clone(), data.clone());
                Kind::File(data)
            }
            EntryType::Directory => Kind::Dir,
            EntryType::Symlink => Kind::Symlink(link_name(&header)),
            // A hard link becomes a copy of a target already seen.
            EntryType::Link => match seen.get(&normalise(&link_name(&header))) {
                Some(data) => Kind::File(data.clone()),
                None => continue,
            },
            EntryType::Char => Kind::CharDev {
                major: header.device_major().ok().flatten().unwrap_or(0),
                minor: header.device_minor().ok().flatten().unwrap_or(0),
            },
            _ => continue,
        };

        entries.push(Entry {
            path: path.into(),
            kind,
            mode,
            uid: header.uid().unwrap_or(0) as u32,
            gid: header.gid().unwrap_or(0) as u32,
            mtime: 0,
        });
    }

    if let Some(manifest) = links_manifest {
        repair_symlinks(&mut entries, &manifest);
    }
    Ok(entries)
}

fn link_name(header: &tar::Header) -> String {
    header
        .link_name()
        .ok()
        .flatten()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Absolute, with tar's `./` prefix removed.
fn normalise(path: &str) -> String {
    let p = path.trim_start_matches('.');
    let p = if p.starts_with('/') {
        p.to_string()
    } else {
        format!("/{p}")
    };
    let trimmed = p.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Restore symlink targets tar truncated, from the guest's manifest.
///
/// One `path\ttarget` per line.
fn repair_symlinks(entries: &mut [Entry], manifest: &str) {
    let targets: BTreeMap<&str, &str> = manifest
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .filter(|(_, target)| !target.is_empty())
        .collect();

    let mut repaired = 0usize;
    for e in entries.iter_mut() {
        if let Kind::Symlink(current) = &e.kind {
            if let Some(real) = targets.get(e.path.to_string_lossy().as_ref()) {
                if real != current {
                    e.kind = Kind::Symlink((*real).to_string());
                    repaired += 1;
                }
            }
        }
    }
    if repaired > 0 {
        eprintln!("[initramfs] guest provision: repaired {repaired} truncated symlink target(s)");
    }
}

fn write_cpio_gz(entries: &[Entry], out_path: &Path) -> Result<(), PatchError> {
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut gz = GzEncoder::new(std::fs::File::create(out_path)?, Compression::default());
    gz.write_all(&cpio::write(entries))?;
    gz.finish()?;
    Ok(())
}

/// The pristine archive's uncompressed size, and how much of the file the
/// kernel will actually read.
///
/// Accepts a raw cpio (from `extract_initramfs`) or a `.UPD`-carved
/// `.cpio.gz` with slack after its member (see [`stage_provisioning_image`]).
///
/// A `bufread` decoder consumes exactly one member and leaves the cursor on
/// the boundary; a whole-slice reader does not.
fn measure_pristine(data: &[u8]) -> Result<Pristine, PatchError> {
    if data.starts_with(b"070701") || data.starts_with(b"070702") {
        return Ok(Pristine {
            raw_len: data.len(),
            member_end: data.len(),
        });
    }
    let mut cursor = std::io::Cursor::new(data);
    let raw_len = std::io::copy(
        &mut flate2::bufread::GzDecoder::new(&mut cursor),
        &mut std::io::sink(),
    )? as usize;
    Ok(Pristine {
        raw_len,
        member_end: cursor.position() as usize,
    })
}

struct Pristine {
    /// Uncompressed size of the archive.
    raw_len: usize,
    /// How many bytes of the file to copy through.
    member_end: usize,
}

/// Size the scratch file must be to hold the handback: the uncompressed
/// rootfs plus tar's per-member overhead and its trailing padding.
pub fn scratch_size_for(initramfs_gz: &Path) -> Result<u64, PatchError> {
    let data = std::fs::read(initramfs_gz)?;
    let raw_len = measure_pristine(&data)?.raw_len;
    // A generous margin: tar adds a 512-byte header per member and rounds every
    // payload up to 512, and the patch set only adds to the tree.
    Ok((raw_len as u64) * 5 / 4 + 64 * 1024 * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The markers the init script prints are the ones the outcome reads.
    #[test]
    fn the_init_prints_what_the_outcome_reads() {
        let init = provisioning_init();
        assert!(init.contains(PATCHING_FAILED), "{init}");
        assert!(init.contains(HANDED_BACK), "{init}");
        assert_eq!(guest_outcome("=== cdj3k: handed back rc=0 ==="), Ok(()));
        assert!(guest_outcome("=== cdj3k: handed back rc=1 ===").is_err());
        assert!(guest_outcome("=== cdj3k: PATCHING FAILED (3) ===").is_err());
        assert!(guest_outcome("").is_err());
    }

    #[test]
    fn tar_paths_become_absolute() {
        assert_eq!(normalise("./etc/shadow"), "/etc/shadow");
        assert_eq!(normalise("./"), "/");
        assert_eq!(normalise("etc/shadow"), "/etc/shadow");
        assert_eq!(normalise("./usr/bin/"), "/usr/bin");
    }

    #[test]
    fn a_truncated_target_is_restored_from_the_manifest() {
        let long =
            "../../../usr/share/ca-certificates/mozilla/Some_Very_Long_Certificate_Name_Indeed.crt";
        let mut entries = vec![
            Entry::symlink("/etc/ssl/certs/a.pem", &long[..long.len() - 4]),
            Entry::symlink("/etc/ssl/certs/b.pem", "short"),
        ];
        let manifest = format!("/etc/ssl/certs/a.pem\t{long}\n/etc/ssl/certs/b.pem\tshort\n");
        repair_symlinks(&mut entries, &manifest);
        assert_eq!(entries[0].kind, Kind::Symlink(long.to_string()));
        assert_eq!(entries[1].kind, Kind::Symlink("short".into()));
    }

    #[test]
    fn a_manifest_without_an_entry_leaves_the_link_alone() {
        let mut entries = vec![Entry::symlink("/bin", "usr/bin")];
        repair_symlinks(&mut entries, "/other\ttarget\n");
        assert_eq!(entries[0].kind, Kind::Symlink("usr/bin".into()));
    }
}
