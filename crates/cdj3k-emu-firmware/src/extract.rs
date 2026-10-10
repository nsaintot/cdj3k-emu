//! Kernel image extraction from a decrypted UPD payload.
//!
//! The decrypted LUKS volume is an ISO 9660 image. Its `images/images.tar.gz`
//! holds `Image`, the AArch64 kernel (about 72 MB).
//!
//! The reader parses the ISO directory itself, so it works on every host
//! without mounting the image. The ISO 9660 primary volume descriptor is at
//! sector 16, and sectors are 2048 bytes.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use tar::Archive;

/// Version metadata extracted from the firmware ISO.
///
/// All fields are `None` when the corresponding file is missing from the
/// UPD. Do not fill in values from a specific UPD version: callers need to
/// know when the metadata is unknown.
#[derive(Debug, Clone, Default)]
pub struct FirmwareInfo {
    /// Pioneer release string (e.g. `"X.XX"`).
    pub release: Option<String>,
    /// Player application revision number (e.g. `"14926"`).
    pub rev_apl: Option<String>,
    /// System image revision number, from `IMAGES/SYSTEM.REV` (e.g. `"14944"`).
    pub rev_system: Option<String>,
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
    info.rev_system = read_trimmed(&mut cursor, "IMAGES/SYSTEM.REV");
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
            ExtractError::Io(e) => write!(f, "I/O error: {e}"),
            ExtractError::BadIso(s) => write!(f, "ISO parse error: {s}"),
            ExtractError::FileNotFound(s) => write!(f, "{s}"),
            ExtractError::TarError(e) => write!(f, "tar error: {e}"),
            ExtractError::UnsupportedG2M(detail) => write!(
                f,
                "unsupported firmware: this is a Renesas G2M (R-Car) build, which the emulator cannot run ({detail}). Use the CDJ-3000 RK3399 firmware instead. See docs/g2m-renesas.md."
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
                "[kernel] Found CDJ3K-RK3399.iso ({} MB); reading its images.tar.gz.",
                inner_bytes.len() / 1_048_576
            ));
            let mut inner = std::io::Cursor::new(inner_bytes);
            read_iso_file(&mut inner, "IMAGES/IMAGES.TAR.GZ")?
        }
        Err(_) => {
            log("[kernel] There is no CDJ3K-RK3399.iso; reading the outer images.tar.gz.");
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
    let root_lba = le32(&pvd, 158);
    let root_len = le32(&pvd, 166);
    let mut dir = vec![0u8; root_len as usize];
    r.seek(SeekFrom::Start(root_lba * ISO_SECTOR))?;
    r.read_exact(&mut dir)?;
    let entries = dir_records(&dir)
        .filter_map(|rec| {
            let raw = record_name(rec).filter(|n| !is_self_or_parent(n))?;
            let slash = if is_dir_record(rec) { "/" } else { "" };
            Some(format!("{}{slash}", String::from_utf8_lossy(raw)))
        })
        .collect();
    Ok(entries)
}

/// Patch all SMC #0 instructions to HVC #0 in the kernel binary in-place.
///
/// The CDJ-3000 kernel uses SMC for PSCI (EL3 firmware).
/// QEMU's virt machine handles PSCI via HVC; without this patch secondary
/// CPUs fail to start and the kernel panics on boot under HVF acceleration.
pub fn patch_kernel_smc_to_hvc(kernel_path: &Path) -> Result<usize, io::Error> {
    let mut data = std::fs::read(kernel_path)?;
    // In little-endian AArch64, `smc #0` encodes as 0xD4000003 and `hvc #0`
    // as 0xD4000002.
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

/// The default kernel path is a file named `Image` next to the ISO.
pub fn default_kernel_path(iso_path: &Path) -> PathBuf {
    iso_path.with_file_name("Image")
}

/// Read `images/cabinet.img` out of the decrypted UPD payload.
///
/// The CDJ-3000X ships it in the outer ISO; v3.13+ UPDs nest their payload in
/// `IMAGES/CDJ3K-RK3399.ISO`, so the inner ISO is tried first.  Returns `None`
/// when the firmware carries no cabinet image, and rejects anything that is not
/// a LUKS container - the deck's `initoptenv` only ever opens one.
pub fn read_cabinet_image(iso_path: &Path) -> Option<Vec<u8>> {
    const LUKS_MAGIC: &[u8] = b"LUKS\xba\xbe";

    let mut iso = std::fs::File::open(iso_path).ok()?;
    let found = match read_iso_file(&mut iso, "IMAGES/CDJ3K-RK3399.ISO") {
        Ok(inner) => {
            let mut inner = std::io::Cursor::new(inner);
            read_iso_file(&mut inner, "IMAGES/CABINET.IMG").ok()
        }
        Err(_) => None,
    };
    let data = match found {
        Some(d) => d,
        None => {
            iso.seek(SeekFrom::Start(0)).ok()?;
            read_iso_file(&mut iso, "IMAGES/CABINET.IMG").ok()?
        }
    };
    data.starts_with(LUKS_MAGIC).then_some(data)
}

/// Reads the file whose hash forms the factory cabinet passphrase:
/// `images/images.tar.gz` or `images/app.hash`, whichever the firmware
/// carries. It searches the inner and outer ISO in the same order as
/// [`read_cabinet_image`], so both files come from the same package.
pub fn read_cabinet_seed(iso_path: &Path) -> Option<Vec<u8>> {
    const SEEDS: [&str; 2] = ["IMAGES/IMAGES.TAR.GZ", "IMAGES/APP.HASH"];
    fn first_seed<R: Read + Seek>(r: &mut R) -> Option<Vec<u8>> {
        SEEDS.iter().find_map(|name| {
            r.seek(SeekFrom::Start(0)).ok()?;
            read_iso_file(r, name).ok()
        })
    }
    let mut iso = std::fs::File::open(iso_path).ok()?;
    read_iso_file(&mut iso, "IMAGES/CDJ3K-RK3399.ISO")
        .ok()
        .and_then(|inner| first_seed(&mut std::io::Cursor::new(inner)))
        .or_else(|| first_seed(&mut iso))
}

/// Write the initramfs in `images/boot.img` to `out_path`, as the firmware
/// ships it (a gzipped cpio).
///
/// A FIT `boot.img` carries it as its `ramdisk` image.
pub fn extract_boot_ramdisk(iso_path: &Path, out_path: &Path) -> Result<bool, ExtractError> {
    let mut iso = std::fs::File::open(iso_path)?;
    let Ok(boot) = read_iso_file(&mut iso, "IMAGES/BOOT.IMG") else {
        return Ok(false);
    };
    std::fs::write(out_path, crate::fit::image(&boot, "ramdisk")?)?;
    Ok(true)
}

// ── ISO 9660 reader ───────────────────────────────────────────────────────────

pub(crate) fn read_iso_file<R: Read + Seek>(
    r: &mut R,
    name: &str,
) -> Result<Vec<u8>, ExtractError> {
    let mut pvd = [0u8; ISO_SECTOR as usize];
    r.seek(SeekFrom::Start(PVD_LBA * ISO_SECTOR))?;
    r.read_exact(&mut pvd)?;

    if pvd[0] != 1 || &pvd[1..6] != b"CD001" {
        return Err(ExtractError::BadIso(
            "not an ISO 9660 primary volume descriptor",
        ));
    }

    // The 34-byte root directory record starts at offset 156.
    let root_lba = le32(&pvd, 158);
    let root_len = le32(&pvd, 166);

    find_in_dir(r, root_lba, root_len, name)
}

/// The little-endian half of an ISO 9660 both-endian u32 at `at`.
fn le32(bytes: &[u8], at: usize) -> u64 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as u64
}

/// Returns `len` as a buffer size after checking that the extent at sector
/// `lba` ends inside the image. The ISO sets these fields, so a buffer is
/// allocated only when the file holds that many bytes.
fn extent_len<R: Seek>(r: &mut R, lba: u64, len: u64) -> Result<usize, ExtractError> {
    let image_len = r.seek(SeekFrom::End(0))?;
    lba.checked_mul(ISO_SECTOR)
        .and_then(|start| start.checked_add(len))
        .filter(|&end| end <= image_len)
        .and_then(|_| usize::try_from(len).ok())
        .ok_or(ExtractError::BadIso(
            "extent runs past the end of the image",
        ))
}

/// The `len` bytes at sector `lba`.
fn read_extent<R: Read + Seek>(r: &mut R, lba: u64, len: u64) -> Result<Vec<u8>, ExtractError> {
    let mut data = vec![0u8; extent_len(r, lba, len)?];
    r.seek(SeekFrom::Start(lba * ISO_SECTOR))?;
    r.read_exact(&mut data)?;
    Ok(data)
}

/// The records of a directory extent.  A zero length byte pads to the next
/// sector.  A record that runs past the extent, or is shorter than the 33-byte
/// minimum (header plus a 1-byte identifier), ends the directory.
fn dir_records(dir: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut pos = 0usize;
    std::iter::from_fn(move || {
        while pos < dir.len() {
            let rec_len = dir[pos] as usize;
            if rec_len == 0 {
                pos = (pos / ISO_SECTOR as usize + 1) * ISO_SECTOR as usize;
                continue;
            }
            if pos + rec_len > dir.len() || rec_len < 33 {
                pos = dir.len();
                return None;
            }
            let rec = &dir[pos..pos + rec_len];
            pos += rec_len;
            return Some(rec);
        }
        None
    })
}

/// A record's file identifier, if it fits in the record.
fn record_name(rec: &[u8]) -> Option<&[u8]> {
    let name_len = rec[32] as usize;
    (33 + name_len <= rec.len()).then(|| &rec[33..33 + name_len])
}

fn is_dir_record(rec: &[u8]) -> bool {
    rec[25] & 0x02 != 0
}

/// The `.` and `..` identifiers.
fn is_self_or_parent(name: &[u8]) -> bool {
    name == b"\x00" || name == b"\x01"
}

fn find_in_dir<R: Read + Seek>(
    r: &mut R,
    dir_lba: u64,
    dir_len: u64,
    target: &str,
) -> Result<Vec<u8>, ExtractError> {
    let dir = read_extent(r, dir_lba, dir_len)?;
    let (head, tail) = match target.split_once('/') {
        Some((head, tail)) => (head, Some(tail)),
        None => (target, None),
    };

    let found = dir_records(&dir).find_map(|rec| {
        let name = std::str::from_utf8(record_name(rec)?).unwrap_or("");
        names_match(name, head).then(|| (le32(rec, 2), le32(rec, 10)))
    });
    match (found, tail) {
        (Some((lba, size)), Some(rest)) => find_in_dir(r, lba, size.max(ISO_SECTOR), rest),
        (Some((lba, size)), None) => read_extent(r, lba, size),
        (None, _) => Err(ExtractError::FileNotFound(format!(
            "{head:?} is not in the directory, which contains {:?}",
            dir_listing(&dir)
        ))),
    }
}

/// `[D]name` / `[F]name` for each entry of a directory, for error messages.
fn dir_listing(dir: &[u8]) -> Vec<String> {
    dir_records(dir)
        .filter_map(|rec| {
            let raw = record_name(rec).filter(|n| !is_self_or_parent(n))?;
            let d = if is_dir_record(rec) { "D" } else { "F" };
            Some(format!("[{d}]{}", String::from_utf8_lossy(raw)))
        })
        .collect()
}

/// Compares two names case-insensitively, after stripping the ISO 9660
/// version suffix, such as `;1`, from both.
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
    let candidates = image_candidates(tar_gz, log)?;
    if candidates.is_empty() {
        return Err(ExtractError::FileNotFound(
            "Image not found in images.tar.gz".into(),
        ));
    }
    let chosen_idx = choose_rk3399(&candidates, log)?;

    let mut out = std::fs::File::create(out_path)?;
    out.write_all(&candidates[chosen_idx].1)?;
    out.flush()?;
    Ok(())
}

/// Every entry whose file name is `Image`, as (full tar path, content).
fn image_candidates(
    tar_gz: &[u8],
    log: &impl Fn(&str),
) -> Result<Vec<(String, Vec<u8>)>, ExtractError> {
    let mut candidates = Vec::new();
    let mut archive = Archive::new(GzDecoder::new(tar_gz));
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
            .unwrap_or("");
        if !name.eq_ignore_ascii_case("Image") {
            continue;
        }
        let mut buf = Vec::new();
        io::copy(&mut entry, &mut buf).map_err(ExtractError::TarError)?;
        log(&format!(
            "[kernel] Found the Image candidate {:?} ({} MB).",
            tar_path,
            buf.len() / 1_048_576
        ));
        candidates.push((tar_path, buf));
    }
    Ok(candidates)
}

/// The candidate with an rk3399 marker, by path or else by content.  G2M
/// (Renesas R-Car) builds are refused: their kernel boots fine in QEMU but the
/// EP122 Xorg fbdev driver is hard-wired to the rcar-du DRM device and cannot
/// run on virtio-gpu (see docs/g2m-renesas.md).
fn choose_rk3399(
    candidates: &[(String, Vec<u8>)],
    log: &impl Fn(&str),
) -> Result<usize, ExtractError> {
    if let Some(i) = candidates
        .iter()
        .position(|(p, _)| p.to_ascii_lowercase().contains("rk3399"))
    {
        log(&format!(
            "[kernel] Selected {:?} because its path contains 'rk3399'.",
            candidates[i].0
        ));
        return Ok(i);
    }
    if let Some(i) = candidates.iter().position(|(_, b)| memmem(b, b"rk3399")) {
        log(&format!(
            "[kernel] Selected {:?} because its content contains 'rk3399'.",
            candidates[i].0
        ));
        return Ok(i);
    }

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
        format!("the tar has R-Car markers such as rcar, r8a7796 or salvator; the candidates are {paths:?}")
    } else {
        format!("no candidate has an rk3399 marker; the candidates are {paths:?}")
    };
    Err(ExtractError::UnsupportedG2M(detail))
}

/// Naive substring search, which avoids a dependency on memchr.
fn memmem(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests;
