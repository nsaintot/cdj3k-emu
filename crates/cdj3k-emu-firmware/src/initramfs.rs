//! Initramfs extraction and in-app patching.
//!
//! extract_initramfs - pull the original initramfs.cpio.gz from the decrypted ISO.
//! patch_initramfs   - inject pre-built .ko modules + guest tools, run the bundled
//!                     patch-rootfs.sh script, repack as a new cpio.gz.
//!
//! # macOS bsdcpio UID/GID caveat
//! bsdcpio records the real host UID (e.g. 502) in every cpio header.  The Linux
//! kernel unpacks with that UID → dropbear rejects /etc/shadow with wrong ownership.
//! After repacking we scan the raw cpio for "070701" magic and force uid/gid fields
//! to "00000000" - same approach as initramfs-work/repack.sh but in pure Rust.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;

use crate::extract::ExtractError;

// ── Public error type ─────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum PatchError {
    Io(io::Error),
    Extract(ExtractError),
    MissingResource(String),
    CommandFailed(String),
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O: {e}"),
            Self::Extract(e) => write!(f, "extract: {e}"),
            Self::MissingResource(s) => write!(f, "missing bundled resource: {s}"),
            Self::CommandFailed(s) => write!(f, "command failed: {s}"),
        }
    }
}

impl From<io::Error> for PatchError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<ExtractError> for PatchError {
    fn from(e: ExtractError) -> Self {
        Self::Extract(e)
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Extract the embedded initramfs from a Pioneer kernel `Image` file.
///
/// Pioneer products embed the initramfs into the kernel image with different
/// compression depending on the build:
/// - CDJ-3000 RK3399 uses gzip (CONFIG_RD_GZIP).
/// - EP122 / G2M Renesas R-Car uses LZ4 legacy (CONFIG_RD_LZ4).
///
/// We scan for every supported magic, try to decompress each hit, and pick
/// the largest result that begins with a cpio newc/crc magic.  This naturally
/// rejects the kernel's embedded `.config` blob (small, gzipped) and other
/// random-byte false positives.
pub fn extract_initramfs(kernel_path: &Path, out_path: &Path) -> Result<(), ExtractError> {
    let data = std::fs::read(kernel_path)?;

    let gzip_offsets = scan_offsets(&data, b"\x1f\x8b\x08");
    let lz4_legacy_offsets = scan_offsets(&data, b"\x02\x21\x4c\x18");

    let mut best: Option<Vec<u8>> = None;
    let mut consider = |cpio: Vec<u8>| {
        if (cpio.starts_with(b"070701") || cpio.starts_with(b"070702"))
            && best.as_ref().is_none_or(|b| cpio.len() > b.len())
        {
            best = Some(cpio);
        }
    };

    for &offset in &gzip_offsets {
        let mut decoder = GzDecoder::new(&data[offset..]);
        let mut cpio = Vec::new();
        if decoder.read_to_end(&mut cpio).is_ok() {
            consider(cpio);
        }
    }

    for &offset in &lz4_legacy_offsets {
        if let Some(cpio) = decompress_lz4_legacy(&data[offset..]) {
            consider(cpio);
        }
    }

    if let Some(cpio) = best {
        std::fs::write(out_path, &cpio)?;
        return Ok(());
    }

    Err(ExtractError::FileNotFound(format!(
        "no embedded CPIO initramfs (gzip or LZ4 legacy) found in {}",
        kernel_path.display()
    )))
}

fn scan_offsets(data: &[u8], magic: &[u8]) -> Vec<usize> {
    data.windows(magic.len())
        .enumerate()
        .filter_map(|(i, w)| (w == magic).then_some(i))
        .collect()
}

/// LZ4 legacy frame: 4-byte magic, then repeating <u32 LE block_size><block data>.
/// Each block decompresses to at most 8 MiB.  We stop when the next u32 is not
/// a plausible block size (>= 8 MiB + slack, or extends past the buffer).
fn decompress_lz4_legacy(input: &[u8]) -> Option<Vec<u8>> {
    const LEGACY_MAGIC: &[u8] = b"\x02\x21\x4c\x18";
    const MAX_BLOCK_DECOMPRESSED: usize = 8 * 1024 * 1024;

    if !input.starts_with(LEGACY_MAGIC) {
        return None;
    }

    let mut cursor = LEGACY_MAGIC.len();
    let mut out = Vec::new();

    while cursor + 4 <= input.len() {
        let block_size = u32::from_le_bytes(input[cursor..cursor + 4].try_into().ok()?) as usize;
        // End-of-stream: another legacy magic, or implausibly large size.
        if block_size == 0 || block_size > MAX_BLOCK_DECOMPRESSED + 16 {
            break;
        }
        cursor += 4;
        let block_end = cursor.checked_add(block_size)?;
        if block_end > input.len() {
            break;
        }
        let mut block_out = vec![0u8; MAX_BLOCK_DECOMPRESSED];
        match lz4_flex::block::decompress_into(&input[cursor..block_end], &mut block_out) {
            Ok(n) => out.extend_from_slice(&block_out[..n]),
            Err(_) => break,
        }
        cursor = block_end;
    }

    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Inject modules + tools into the initramfs and apply bundled patches.
///
/// - `initramfs_gz`   - original `initramfs.cpio.gz` from the ISO
/// - `resources_dir`  - `Contents/Resources/` inside the .app bundle
/// - `out_path`       - destination for the patched `initramfs-patched.cpio.gz`
pub fn patch_initramfs(
    initramfs_gz: &Path,
    resources_dir: &Path,
    out_path: &Path,
) -> Result<(), PatchError> {
    let modules_dir = resources_dir.join("modules");
    let patch_dir = resources_dir.join("patch");
    let tools_dir = resources_dir.join("tools");

    for req in [&modules_dir, &patch_dir] {
        if !req.exists() {
            return Err(PatchError::MissingResource(req.display().to_string()));
        }
    }

    // Create a temp working directory.
    let tmp = tmp_dir("cdj3k-emu-initramfs")?;
    let rootfs = tmp.join("rootfs");
    std::fs::create_dir_all(&rootfs)?;

    // 1. Unpack the original initramfs into rootfs/.
    unpack_cpio_gz(initramfs_gz, &rootfs)?;

    // 2. Inject pre-built .ko files into rootfs/lib/modules/.
    let ko_dst = rootfs.join("lib/modules");
    std::fs::create_dir_all(&ko_dst)?;
    for entry in std::fs::read_dir(&modules_dir)? {
        let src = entry?.path();
        if src.extension().and_then(|e| e.to_str()) == Some("ko") {
            let dst = ko_dst.join(src.file_name().unwrap());
            std::fs::copy(&src, &dst)?;
        }
    }

    // 3. Inject guest tools (aarch64 ELFs) into rootfs/usr/bin/.
    if tools_dir.exists() {
        let bin_dst = rootfs.join("usr/bin");
        std::fs::create_dir_all(&bin_dst)?;
        let home_dst = rootfs.join("home/root");
        std::fs::create_dir_all(&home_dst)?;

        for entry in std::fs::read_dir(&tools_dir)? {
            let src = entry?.path();
            let name = src.file_name().unwrap().to_string_lossy().to_string();
            let dst = if name == "ep122_shim.so" {
                home_dst.join(&name)
            } else {
                bin_dst.join(&name)
            };
            std::fs::copy(&src, &dst)?;
            set_executable(&dst)?;
        }
    }

    // 4. Run patch-rootfs.sh from the bundled patch directory.
    let patch_script = patch_dir.join("patch-rootfs.sh");
    if !patch_script.exists() {
        return Err(PatchError::MissingResource(
            patch_script.display().to_string(),
        ));
    }
    let status = Command::new("bash")
        .arg(&patch_script)
        .arg(&rootfs)
        .env("ROOTFS", &rootfs)
        .env("PATCH_ASSETS_DIR", &patch_dir)
        .status()?;
    if !status.success() {
        return Err(PatchError::CommandFailed(format!(
            "patch-rootfs.sh exited with {status}"
        )));
    }

    // 5. Repack rootfs → raw cpio.
    let raw_cpio = tmp.join("initramfs-patched.cpio");
    repack_cpio(&rootfs, &raw_cpio)?;

    // 6. Fix uid/gid in the raw cpio (macOS bsdcpio records host UID).
    let mut cpio_bytes = std::fs::read(&raw_cpio)?;
    fix_cpio_ownership(&mut cpio_bytes);

    // 7. Gzip compress → out_path.
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let out_file = std::fs::File::create(out_path)?;
    let mut gz = GzEncoder::new(out_file, Compression::best());
    gz.write_all(&cpio_bytes)?;
    gz.finish()?;

    // 8. Clean up temp dir.
    let _ = std::fs::remove_dir_all(&tmp);

    Ok(())
}

// ── cpio helpers ──────────────────────────────────────────────────────────────

fn unpack_cpio_gz(gz_path: &Path, rootfs: &Path) -> Result<(), PatchError> {
    let raw = std::fs::read(gz_path)?;
    // Accept raw CPIO (starts with "070701") or gzip-compressed CPIO.
    let cpio_data = if raw.starts_with(b"070701") || raw.starts_with(b"070702") {
        raw
    } else {
        let mut decoder = GzDecoder::new(raw.as_slice());
        let mut out = Vec::new();
        decoder.read_to_end(&mut out)?;
        out
    };

    let mut child = Command::new("cpio")
        .args(["-id", "--quiet"])
        .current_dir(rootfs)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| PatchError::CommandFailed(format!("cpio: {e}")))?;

    child.stdin.take().unwrap().write_all(&cpio_data)?;

    let status = child.wait()?;
    // bsdcpio (macOS) exits 1 for non-fatal warnings (device nodes, etc.) - only fail on ≥2.
    if status.code().unwrap_or(2) >= 2 {
        return Err(PatchError::CommandFailed(format!(
            "cpio -id exited with {status}"
        )));
    }
    Ok(())
}

fn repack_cpio(rootfs: &Path, out: &Path) -> Result<(), PatchError> {
    // find . | sort | cpio -H newc -o  inside rootfs, output to out.
    let find = Command::new("find").arg(".").current_dir(rootfs).output()?;
    if !find.status.success() {
        return Err(PatchError::CommandFailed("find failed".into()));
    }

    // Sort the file list for deterministic output.
    let mut paths: Vec<&str> = std::str::from_utf8(&find.stdout)
        .unwrap_or("")
        .lines()
        .collect();
    paths.sort_unstable();
    let sorted = paths.join("\n");

    let out_file = std::fs::File::create(out)?;
    let mut child = Command::new("cpio")
        .args(["-H", "newc", "-o", "--quiet"])
        .current_dir(rootfs)
        .stdin(std::process::Stdio::piped())
        .stdout(out_file)
        .spawn()
        .map_err(|e| PatchError::CommandFailed(format!("cpio -o: {e}")))?;

    child.stdin.take().unwrap().write_all(sorted.as_bytes())?;

    let status = child.wait()?;
    // bsdcpio (macOS) exits 1 for non-fatal warnings (unreadable setuid files, device
    // nodes, etc.) but still produces a valid archive - only fail on ≥ 2.
    let code = status.code().unwrap_or(1);
    if code >= 2 {
        return Err(PatchError::CommandFailed(format!(
            "cpio -o exited with {status}"
        )));
    }
    Ok(())
}

/// Patch all NEWC cpio entries: force uid and gid fields to "00000000".
/// Mirrors the Python post-processor in initramfs-work/repack.sh.
fn fix_cpio_ownership(data: &mut Vec<u8>) {
    const MAGIC: &[u8] = b"070701";
    const HEADER: usize = 110;
    let mut pos = 0;
    while pos + HEADER <= data.len() {
        if &data[pos..pos + 6] != MAGIC {
            pos += 1;
            continue;
        }
        // uid at +22 (8 bytes), gid at +30 (8 bytes).
        data[pos + 22..pos + 30].copy_from_slice(b"00000000");
        data[pos + 30..pos + 38].copy_from_slice(b"00000000");

        let namesize = parse_hex8(&data[pos + 94..pos + 102]);
        let filesize = parse_hex8(&data[pos + 54..pos + 62]);
        let name_end = HEADER + namesize;
        let name_pad = (4 - name_end % 4) % 4;
        let data_pad = if filesize > 0 {
            (4 - filesize % 4) % 4
        } else {
            0
        };
        pos += name_end + name_pad + filesize + data_pad;
    }
}

fn parse_hex8(bytes: &[u8]) -> usize {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| usize::from_str_radix(s, 16).ok())
        .unwrap_or(0)
}

// ── misc ──────────────────────────────────────────────────────────────────────

fn tmp_dir(prefix: &str) -> io::Result<PathBuf> {
    // Nanosecond resolution + PID makes concurrent collisions vanishingly
    // unlikely without pulling in the `tempfile` crate just for this one site.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("{prefix}-{pid}-{ts}"));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(unix)]
fn set_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(perms.mode() | 0o111);
    std::fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}
