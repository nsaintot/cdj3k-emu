//! Initramfs extraction and in-app patching.
//!
//! - `extract_initramfs` takes the original initramfs out of the kernel image.
//! - `patch_initramfs` injects the guest tools, runs the bundled
//!   `patch-rootfs.sh` script and repacks the result as a new cpio.gz.
//!
//! # macOS bsdcpio UID/GID caveat
//! bsdcpio records the real host UID (for example 502) in every cpio header.
//! The Linux kernel unpacks the files with that UID, so dropbear rejects
//! /etc/shadow because of its owner. After repacking, the code scans the raw
//! cpio for the `070701` magic and sets the uid and gid fields to `00000000`.

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
    /// A bundled resource is present but unusable, such as a step manifest
    /// that does not parse or does not match the rootfs.
    InvalidResource(String),
    CommandFailed(String),
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Extract(e) => write!(f, "extract: {e}"),
            Self::MissingResource(s) => write!(f, "missing bundled resource: {s}"),
            Self::InvalidResource(s) => write!(f, "invalid bundled resource: {s}"),
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
/// - G2M Renesas R-Car uses LZ4 legacy (CONFIG_RD_LZ4).
///
/// The function scans for every supported magic, decompresses each hit, and
/// keeps the largest result that begins with a cpio newc or crc magic. This
/// rejects the kernel's embedded `.config` blob, which is small and gzipped,
/// and random false positives.
pub fn extract_initramfs(kernel_path: &Path, out_path: &Path) -> Result<(), ExtractError> {
    let data = std::fs::read(kernel_path)?;

    let gzip_offsets = scan_offsets(&data, b"\x1f\x8b\x08");
    let lz4_legacy_offsets = scan_offsets(&data, b"\x02\x21\x4c\x18");

    let mut best: Option<Vec<u8>> = None;
    let mut consider = |cpio: Vec<u8>| {
        if (cpio.starts_with(b"070701") || cpio.starts_with(b"070702"))
            && !matches!(&best, Some(b) if cpio.len() <= b.len())
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
        "{} has no embedded cpio initramfs (gzip or LZ4 legacy)",
        kernel_path.display()
    )))
}

fn scan_offsets(data: &[u8], magic: &[u8]) -> Vec<usize> {
    data.windows(magic.len())
        .enumerate()
        .filter_map(|(i, w)| (w == magic).then_some(i))
        .collect()
}

/// Decompresses an LZ4 legacy frame: a 4-byte magic followed by blocks, each a
/// little-endian u32 size and that many bytes of data. Each block decompresses
/// to at most 8 MiB. Decoding stops at a size that is not plausible: zero,
/// larger than 8 MiB plus slack, or running past the buffer.
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
        // Another legacy magic or an implausibly large size ends the stream.
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

/// Injects the guest tools into the initramfs and applies the bundled patches.
///
/// The app uses this host-side path only when `CDJ3K_INITRAMFS_HOST` is set,
/// and it needs bash, GNU sed and cpio. By default the app patches inside a
/// guest through [`crate::initramfs_guest`].
///
/// - `initramfs_gz`   - original `initramfs.cpio.gz` from the ISO
/// - `resources_dir`  - `Contents/Resources/` inside the .app bundle
/// - `out_path`       - destination for the patched `initramfs-patched.cpio.gz`
pub fn patch_initramfs(
    initramfs_gz: &Path,
    resources_dir: &Path,
    out_path: &Path,
) -> Result<(), PatchError> {
    let patch_dir = resources_dir.join("patch");
    let tools_dir = resources_dir.join("tools");

    if !patch_dir.exists() {
        return Err(PatchError::MissingResource(patch_dir.display().to_string()));
    }

    let mut clock = PhaseClock::start();

    let tmp = tmp_dir("cdj3k-emu-initramfs")?;
    let rootfs = tmp.join("rootfs");
    std::fs::create_dir_all(&rootfs)?;

    // 1. Unpack the original initramfs into rootfs/.
    unpack_cpio_gz(initramfs_gz, &rootfs)?;
    clock.phase("unpack");

    // 2. Copy the guest tools into the rootfs: deck_shim.so to /home/root
    //    and the rest to /usr/bin.
    if tools_dir.exists() {
        inject_tools(&tools_dir, &rootfs)?;
    }
    clock.phase("inject");

    // 3. Run patch-rootfs.sh from the bundled patch directory.
    run_patch_script(&patch_dir, &rootfs)?;
    clock.phase("patch scripts");

    // 4. Repack the rootfs into a raw cpio.
    let raw_cpio = tmp.join("initramfs-patched.cpio");
    repack_cpio(&rootfs, &raw_cpio)?;
    clock.phase("cpio repack");

    // 5. Reset the uid and gid in the raw cpio, because macOS bsdcpio records
    //    the host UID.
    let mut cpio_bytes = std::fs::read(&raw_cpio)?;
    fix_cpio_ownership(&mut cpio_bytes);
    clock.phase("ownership fix");

    // 6. Write the gzipped cpio to out_path.
    write_gz(out_path, &cpio_bytes)?;
    clock.phase("gzip");

    let _ = std::fs::remove_dir_all(&tmp);
    clock.phase("cleanup");
    clock.total();

    Ok(())
}

/// Per-phase timings for the provision log. The rootfs is unpacked to temp
/// files, repacked and deleted, so the cost depends on the file count as well
/// as the size, and differs sharply between models.
struct PhaseClock {
    started: std::time::Instant,
    mark: std::time::Instant,
}

impl PhaseClock {
    fn start() -> Self {
        let now = std::time::Instant::now();
        Self {
            started: now,
            mark: now,
        }
    }

    /// Logs the time since the previous phase ended.
    fn phase(&mut self, name: &str) {
        eprintln!(
            "[initramfs] {:<16} {:>6.1}s",
            name,
            self.mark.elapsed().as_secs_f64()
        );
        self.mark = std::time::Instant::now();
    }

    fn total(&self) {
        eprintln!(
            "[initramfs] {:<16} {:>6.1}s",
            "total",
            self.started.elapsed().as_secs_f64()
        );
    }
}

/// Copies every file of `tools_dir` into the rootfs as an executable:
/// `deck_shim.so` to `/home/root` and the rest to `/usr/bin`.
fn inject_tools(tools_dir: &Path, rootfs: &Path) -> Result<(), PatchError> {
    let bin_dst = rootfs.join("usr/bin");
    std::fs::create_dir_all(&bin_dst)?;
    let home_dst = rootfs.join("home/root");
    std::fs::create_dir_all(&home_dst)?;

    for entry in std::fs::read_dir(tools_dir)? {
        let src = entry?.path();
        let name = src.file_name().unwrap().to_string_lossy().to_string();
        let dst = if name == "deck_shim.so" {
            home_dst.join(&name)
        } else {
            bin_dst.join(&name)
        };
        std::fs::copy(&src, &dst)?;
        crate::file_mode::set_executable(&dst)?;
    }
    Ok(())
}

/// Runs the bundled `patch-rootfs.sh` over `rootfs`.
fn run_patch_script(patch_dir: &Path, rootfs: &Path) -> Result<(), PatchError> {
    let patch_script = patch_dir.join("patch-rootfs.sh");
    if !patch_script.exists() {
        return Err(PatchError::MissingResource(
            patch_script.display().to_string(),
        ));
    }
    let status = Command::new("bash")
        .arg(&patch_script)
        .arg(rootfs)
        .env("ROOTFS", rootfs)
        .env("PATCH_ASSETS_DIR", patch_dir)
        .status()?;
    if !status.success() {
        return Err(PatchError::CommandFailed(format!(
            "patch-rootfs.sh exited with {status}"
        )));
    }
    Ok(())
}

/// Gzips `bytes` to `out_path`, creating its directory.
fn write_gz(out_path: &Path, bytes: &[u8]) -> Result<(), PatchError> {
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let out_file = std::fs::File::create(out_path)?;
    // flate2's default level 6 compresses the 196 MiB CDJ-3000X rootfs to
    // 81 MiB, as level 9 does, and level 9 takes about 1.8 times as long.
    let mut gz = GzEncoder::new(out_file, Compression::default());
    gz.write_all(bytes)?;
    gz.finish()?;
    Ok(())
}

// ── cpio helpers ──────────────────────────────────────────────────────────────

fn unpack_cpio_gz(gz_path: &Path, rootfs: &Path) -> Result<(), PatchError> {
    let raw = std::fs::read(gz_path)?;
    // Accept a raw cpio (magic `070701` or `070702`) or a gzipped one.
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
    // macOS bsdcpio exits with 1 on non-fatal warnings, such as for device
    // nodes, so only an exit code of 2 or more is an error.
    if status.code().unwrap_or(2) >= 2 {
        return Err(PatchError::CommandFailed(format!(
            "cpio -id exited with {status}"
        )));
    }
    Ok(())
}

fn repack_cpio(rootfs: &Path, out: &Path) -> Result<(), PatchError> {
    // Run `find . | sort | cpio -H newc -o` inside `rootfs` and write the
    // archive to `out`.
    let find = Command::new("find").arg(".").current_dir(rootfs).output()?;
    if !find.status.success() {
        return Err(PatchError::CommandFailed("find . failed".into()));
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
    // macOS bsdcpio exits with 1 on non-fatal warnings, such as for unreadable
    // setuid files or device nodes, and still writes a valid archive, so only
    // an exit code of 2 or more is an error.
    let code = status.code().unwrap_or(1);
    if code >= 2 {
        return Err(PatchError::CommandFailed(format!(
            "cpio -o exited with {status}"
        )));
    }
    Ok(())
}

/// Sets the uid and gid fields of every newc cpio entry to `00000000`.
fn fix_cpio_ownership(data: &mut [u8]) {
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
    // A nanosecond timestamp and the PID make a name collision between
    // concurrent runs very unlikely, without a dependency on `tempfile`.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("{prefix}-{pid}-{ts}"));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
