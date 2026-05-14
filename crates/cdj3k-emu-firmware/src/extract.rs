//! Kernel image extraction from a decrypted UPD payload.
//!
//! The decrypted LUKS volume is an ISO 9660 image containing:
//!   images/images.tar.gz  →  Image  (AArch64 kernel, ~72 MB).
//!
//! The simpler cross-platform path: read the ISO directory, locate the file,
//! stream it out.  ISO 9660 primary volume descriptor is at sector 16 (LBA 16,
//! 2048 bytes/sector).

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use tar::Archive;

/// Version metadata extracted from the firmware ISO.
///
/// All fields are `None` when the corresponding file is missing from the
/// UPD. Do NOT substitute a specific UPD version's values as a fallback —
/// downstream callers should know when metadata is unknown.
#[derive(Debug, Clone, Default)]
pub struct FirmwareInfo {
    /// Pioneer release string (e.g. `"X.XX"`).
    pub release: Option<String>,
    /// Application (EP122) revision number (e.g. `"14926"`).
    pub rev_apl: Option<String>,
    /// Kernel revision number (e.g. `"14944"`).
    pub rev_kernel: Option<String>,
    /// MD5 of `miniloader.img` (e.g. `"47e3ef9b7f78f5b9be317882a74e9527"`).
    pub miniloader: Option<String>,
}

/// Read firmware version metadata from the decrypted outer ISO.
///
/// Reads `IMAGES/APP.REV`, `IMAGES/SYSTEM.REV`, `IMAGES/RELEASE.TXT`, and
/// `IMAGES/MINILOADER.IMG` from the inner `CDJ3K-RK3399.ISO` (v3.13+) or
/// directly from the outer ISO for legacy UPDs.  Falls back to defaults for
/// any file that cannot be found.
pub fn read_firmware_info(iso_path: &Path) -> Result<FirmwareInfo, ExtractError> {
    let mut iso = std::fs::File::open(iso_path)?;

    // v3.13+ UPDs nest a `CDJ3K-RK3399.iso` inside; older UPDs put the files
    // directly in the outer ISO. Read the inner ISO if present, otherwise
    // read the outer ISO whole into memory and parse from there.
    let inner_bytes = match read_iso_file(&mut iso, "IMAGES/CDJ3K-RK3399.ISO") {
        Ok(b) => b,
        Err(_) => std::fs::read(iso_path)?,
    };

    let mut cursor = std::io::Cursor::new(&inner_bytes);
    let mut info = FirmwareInfo::default();

    let read_trimmed = |c: &mut std::io::Cursor<&Vec<u8>>, name: &str| -> Option<String> {
        c.set_position(0);
        read_iso_file(c, name)
            .ok()
            .map(|b| String::from_utf8_lossy(&b).trim().to_string())
            .filter(|s| !s.is_empty())
    };

    info.release = read_trimmed(&mut cursor, "IMAGES/RELEASE.TXT");
    info.rev_apl = read_trimmed(&mut cursor, "IMAGES/APP.REV");
    info.rev_kernel = read_trimmed(&mut cursor, "IMAGES/SYSTEM.REV");
    cursor.set_position(0);
    info.miniloader = read_iso_file(&mut cursor, "IMAGES/MINILOADER.IMG")
        .ok()
        .map(|ml_bytes| format!("{:x}", md5::compute(&ml_bytes)));

    Ok(info)
}

#[derive(Debug)]
pub enum ExtractError {
    Io(io::Error),
    BadIso(&'static str),
    FileNotFound(String),
    TarError(io::Error),
    UnsupportedG2M(String),
}

impl From<io::Error> for ExtractError {
    fn from(e: io::Error) -> Self {
        ExtractError::Io(e)
    }
}

impl std::fmt::Display for ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtractError::Io(e) => write!(f, "I/O: {e}"),
            ExtractError::BadIso(s) => write!(f, "ISO parse error: {s}"),
            ExtractError::FileNotFound(s) => write!(f, "FileNotFound({s:?})"),
            ExtractError::TarError(e) => write!(f, "tar error: {e}"),
            ExtractError::UnsupportedG2M(detail) => write!(
                f,
                "unsupported firmware: Renesas G2M (R-Car) build is not supported by this emulator - use CDJ-3000 RK3399 firmware instead ({detail}). See docs/g2m-renesas.md for background."
            ),
        }
    }
}

const ISO_SECTOR: u64 = 2048;
const PVD_LBA: u64 = 16;
/// Extract the `Image` kernel from the decrypted UPD ISO payload.
///
/// `iso_path` is the raw ISO file produced by `decrypt_upd`.
/// The extracted kernel is written to `out_path`.
/// `log` receives diagnostic lines (tar entry listing, selection reason).
pub fn extract_kernel(
    iso_path: &Path,
    out_path: &Path,
    log: impl Fn(&str),
) -> Result<(), ExtractError> {
    let mut iso = std::fs::File::open(iso_path)?;

    if let Ok(root_entries) = list_iso_root(&mut iso) {
        log(&format!("[kernel] ISO root: {:?}", root_entries));
    }
    let _ = iso.seek(SeekFrom::Start(0));

    // v3.13+ UPDs contain a nested CDJ3K-RK3399.iso (rk3399 payload) alongside
    // an outer images/images.tar.gz (G2M/Renesas payload). Prefer the inner
    // rk3399 ISO when present; otherwise fall back to whatever the UPD contains.
    let tar_gz_data = match read_iso_file(&mut iso, "IMAGES/CDJ3K-RK3399.ISO") {
        Ok(inner_bytes) => {
            log(&format!(
                "[kernel] found CDJ3K-RK3399.iso ({} MB), reading inner images.tar.gz",
                inner_bytes.len() / 1_048_576
            ));
            let mut inner = std::io::Cursor::new(inner_bytes);
            read_iso_file(&mut inner, "IMAGES/IMAGES.TAR.GZ")?
        }
        Err(_) => {
            log("[kernel] no CDJ3K-RK3399.iso, reading outer images.tar.gz");
            let _ = iso.seek(SeekFrom::Start(0));
            read_iso_file(&mut iso, "IMAGES/IMAGES.TAR.GZ")?
        }
    };

    extract_image_from_tar_gz(&tar_gz_data, out_path, &log)?;
    Ok(())
}

fn list_iso_root<R: Read + Seek>(r: &mut R) -> Result<Vec<String>, ExtractError> {
    let mut pvd = [0u8; ISO_SECTOR as usize];
    r.seek(SeekFrom::Start(PVD_LBA * ISO_SECTOR))?;
    r.read_exact(&mut pvd)?;
    // `pvd` is a fixed-size [u8; ISO_SECTOR] (2048) so these slices are always
    // 4 bytes — the `try_into().unwrap()` cannot fail on a well-typed buffer.
    let root_lba = u32::from_le_bytes(pvd[158..162].try_into().unwrap()) as u64;
    let root_len = u32::from_le_bytes(pvd[166..170].try_into().unwrap()) as u64;
    let mut dir = vec![0u8; root_len as usize];
    r.seek(SeekFrom::Start(root_lba * ISO_SECTOR))?;
    r.read_exact(&mut dir)?;
    let mut entries = Vec::new();
    let mut pos = 0usize;
    while pos < dir.len() {
        let rec_len = dir[pos] as usize;
        if rec_len == 0 {
            pos = (pos / ISO_SECTOR as usize + 1) * ISO_SECTOR as usize;
            continue;
        }
        if pos + rec_len > dir.len() || rec_len < 33 {
            break;
        }
        let rec = &dir[pos..pos + rec_len];
        let name_len = rec[32] as usize;
        if 33 + name_len <= rec.len() {
            let raw = &rec[33..33 + name_len];
            if raw != b"\x00" && raw != b"\x01" {
                let is_dir = rec[25] & 0x02 != 0;
                entries.push(format!(
                    "{}{}",
                    String::from_utf8_lossy(raw),
                    if is_dir { "/" } else { "" }
                ));
            }
        }
        pos += rec_len;
    }
    Ok(entries)
}

/// Patch all SMC #0 instructions to HVC #0 in the kernel binary in-place.
///
/// The CDJ-3000 kernel uses SMC for PSCI (EL3 firmware).
/// QEMU's virt machine handles PSCI via HVC; without this patch secondary
/// CPUs fail to start and the kernel panics on boot under HVF acceleration.
pub fn patch_kernel_smc_to_hvc(kernel_path: &Path) -> Result<usize, io::Error> {
    let mut data = std::fs::read(kernel_path)?;
    // AArch64 LE: smc #0 = 0xD4000003, hvc #0 = 0xD4000002
    let smc: [u8; 4] = [0x03, 0x00, 0x00, 0xd4];
    let hvc: [u8; 4] = [0x02, 0x00, 0x00, 0xd4];
    let mut count = 0usize;
    let mut i = 0;
    while i + 4 <= data.len() {
        if data[i..i + 4] == smc {
            data[i..i + 4].copy_from_slice(&hvc);
            count += 1;
            i += 4;
        } else {
            i += 1;
        }
    }
    std::fs::write(kernel_path, &data)?;
    Ok(count)
}

/// Default output path: next to the ISO, named "Image".
pub fn default_kernel_path(iso_path: &Path) -> PathBuf {
    iso_path.with_file_name("Image")
}

// ── ISO 9660 reader ───────────────────────────────────────────────────────────

pub(crate) fn read_iso_file<R: Read + Seek>(
    r: &mut R,
    name: &str,
) -> Result<Vec<u8>, ExtractError> {
    // Read Primary Volume Descriptor at LBA 16.
    let mut pvd = [0u8; ISO_SECTOR as usize];
    r.seek(SeekFrom::Start(PVD_LBA * ISO_SECTOR))?;
    r.read_exact(&mut pvd)?;

    if pvd[0] != 1 || &pvd[1..6] != b"CD001" {
        return Err(ExtractError::BadIso(
            "not an ISO 9660 primary volume descriptor",
        ));
    }

    // Root directory record is at offset 156, 34 bytes.
    let root_lba = u32::from_le_bytes(pvd[158..162].try_into().unwrap()) as u64;
    let root_len = u32::from_le_bytes(pvd[166..170].try_into().unwrap()) as u64;

    find_in_dir(r, root_lba, root_len, name)
}

fn find_in_dir<R: Read + Seek>(
    r: &mut R,
    dir_lba: u64,
    dir_len: u64,
    target: &str,
) -> Result<Vec<u8>, ExtractError> {
    let mut dir_data = vec![0u8; dir_len as usize];
    r.seek(SeekFrom::Start(dir_lba * ISO_SECTOR))?;
    r.read_exact(&mut dir_data)?;

    // Split target into first component and remainder.
    let (head, tail) = match target.find('/') {
        Some(i) => (&target[..i], Some(&target[i + 1..])),
        None => (target, None),
    };

    let mut pos = 0usize;
    while pos < dir_data.len() {
        let rec_len = dir_data[pos] as usize;
        if rec_len == 0 {
            // Skip to next sector boundary.
            pos = (pos / ISO_SECTOR as usize + 1) * ISO_SECTOR as usize;
            continue;
        }
        if pos + rec_len > dir_data.len() {
            break;
        }
        // ISO 9660 directory record minimum size is 33 bytes (header + 1-byte
        // identifier).  A crafted .UPD could ship rec_len < 33 to trip the
        // fixed-offset reads below; treat that as end-of-directory.
        if rec_len < 33 {
            break;
        }

        let rec = &dir_data[pos..pos + rec_len];
        let file_lba = u32::from_le_bytes(rec[2..6].try_into().unwrap()) as u64;
        let file_size = u32::from_le_bytes(rec[10..14].try_into().unwrap()) as u64;
        let name_len = rec[32] as usize;
        if 33 + name_len > rec.len() {
            pos += rec_len;
            continue;
        }
        let file_name = std::str::from_utf8(&rec[33..33 + name_len]).unwrap_or("");

        if names_match(file_name, head) {
            if let Some(rest) = tail {
                return find_in_dir(r, file_lba, file_size.max(ISO_SECTOR), rest);
            } else {
                // Read file data.
                let mut data = vec![0u8; file_size as usize];
                r.seek(SeekFrom::Start(file_lba * ISO_SECTOR))?;
                r.read_exact(&mut data)?;
                return Ok(data);
            }
        }

        pos += rec_len;
    }

    // Build directory listing for the error message.
    let mut pos2 = 0usize;
    let mut entries = Vec::new();
    while pos2 < dir_data.len() {
        let rec_len = dir_data[pos2] as usize;
        if rec_len == 0 {
            pos2 = (pos2 / ISO_SECTOR as usize + 1) * ISO_SECTOR as usize;
            continue;
        }
        if pos2 + rec_len > dir_data.len() {
            break;
        }
        let rec = &dir_data[pos2..pos2 + rec_len];
        let name_len = rec[32] as usize;
        if pos2 + 33 + name_len <= pos2 + rec_len {
            let raw = &rec[33..33 + name_len];
            let d = if rec[25] & 0x02 != 0 { "D" } else { "F" };
            let n = String::from_utf8_lossy(raw);
            if raw != b"\x00" && raw != b"\x01" {
                entries.push(format!("[{d}]{n}"));
            }
        }
        pos2 += rec_len;
    }

    Err(ExtractError::FileNotFound(format!(
        "looking for {head:?}; dir contains: {entries:?}"
    )))
}

/// Case-insensitive match; strips ISO 9660 version suffix (`;1`) from both sides.
fn names_match(iso_name: &str, target: &str) -> bool {
    let a = iso_name.split(';').next().unwrap_or(iso_name);
    let b = target.split(';').next().unwrap_or(target);
    a.eq_ignore_ascii_case(b)
}

// ── tar.gz extraction ─────────────────────────────────────────────────────────

fn extract_image_from_tar_gz(
    tar_gz: &[u8],
    out_path: &Path,
    log: &impl Fn(&str),
) -> Result<(), ExtractError> {
    // Collect all entries whose filename is "Image".
    // Each candidate stores (full tar path, content).
    let mut candidates: Vec<(String, Vec<u8>)> = Vec::new();

    let gz = GzDecoder::new(tar_gz);
    let mut archive = Archive::new(gz);
    for entry in archive.entries().map_err(ExtractError::TarError)? {
        let mut entry = entry.map_err(ExtractError::TarError)?;
        let tar_path = entry
            .path()
            .map_err(ExtractError::TarError)?
            .to_string_lossy()
            .into_owned();
        let name = std::path::Path::new(&tar_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_owned();
        if name.eq_ignore_ascii_case("Image") {
            let mut buf = Vec::new();
            io::copy(&mut entry, &mut buf).map_err(ExtractError::TarError)?;
            log(&format!(
                "[kernel] tar Image candidate: {:?}  {} MB",
                tar_path,
                buf.len() / 1_048_576
            ));
            candidates.push((tar_path, buf));
        }
    }

    if candidates.is_empty() {
        return Err(ExtractError::FileNotFound(
            "Image not found in images.tar.gz".into(),
        ));
    }

    // Require an rk3399 marker (by path or content).  G2M (Renesas R-Car) builds
    // are rejected here: their kernel boots fine in QEMU but the EP122 Xorg fbdev
    // driver is hard-wired to the rcar-du DRM device and cannot run on virtio-gpu
    // (see docs/g2m-renesas.md).
    let chosen_idx = if let Some(i) = candidates
        .iter()
        .position(|(p, _)| p.to_ascii_lowercase().contains("rk3399"))
    {
        log(&format!(
            "[kernel] selected by path 'rk3399': {:?}",
            candidates[i].0
        ));
        i
    } else if let Some(i) = candidates.iter().position(|(_, b)| memmem(b, b"rk3399")) {
        log(&format!(
            "[kernel] selected by content 'rk3399': {:?}",
            candidates[i].0
        ));
        i
    } else {
        let g2m_hint = candidates.iter().any(|(p, b)| {
            let lp = p.to_ascii_lowercase();
            lp.contains("g2m")
                || lp.contains("salvator")
                || lp.contains("rcar")
                || memmem(b, b"rcar-du")
                || memmem(b, b"r8a7796")
                || memmem(b, b"salvator-x")
        });
        let paths: Vec<&str> = candidates.iter().map(|(p, _)| p.as_str()).collect();
        let detail = if g2m_hint {
            format!("tar contains rcar/r8a7796/salvator markers, candidates={paths:?}")
        } else {
            format!("no rk3399 marker found, candidates={paths:?}")
        };
        return Err(ExtractError::UnsupportedG2M(detail));
    };

    let mut out = std::fs::File::create(out_path)?;
    out.write_all(&candidates[chosen_idx].1)?;
    out.flush()?;
    Ok(())
}

/// Naive substring search (avoids pulling in a memchr crate just for this).
fn memmem(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
