//! Decrypts LUKS1 containers without external tools.
//!
//! Opens `aes-xts-plain64` containers with a 64-byte key. The key material
//! is always decrypted with AES-XTS, so the slots of an `aes-cbc-essiv`
//! container do not open, although the payload decryptor handles CBC.
//!
//! Decryption reads the LUKS1 header, derives the slot key with PBKDF2 and
//! `hash_spec`, XTS-decrypts the key material, merges it with the
//! anti-forensic splitter and checks the master key digest. It then decrypts
//! the payload with the cipher that `cipher_mode` names.

use std::fmt::Write as _;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use aes::cipher::KeyInit;
use aes::Aes256;
use xts_mode::Xts128;

const LUKS_MAGIC: &[u8; 6] = b"LUKS\xba\xbe";
pub(crate) const LUKS_SECTOR: u64 = 512;
/// The anti-forensic stripe count cryptsetup writes and requires for LUKS1.
const LUKS1_STRIPES: u32 = 4000;

mod crypto;
mod rekey;
use crypto::{af_merge, decrypt_payload, pbkdf2_derive, xts_tweak};

pub use rekey::{add_keyslot, cabinet_passphrase, vendor_passphrase, RekeyError};

/// A LUKS keyfile (raw binary, typically 32 bytes for AES-256).
pub struct LuksKey(pub Vec<u8>);

impl LuksKey {
    pub fn from_file(path: &Path) -> io::Result<Self> {
        Ok(Self(std::fs::read(path)?))
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Parses a key typed by the user, trying in order:
    ///   1. A hex string (0-9 a-f A-F), with whitespace and colons stripped.
    ///   2. Base64, standard or URL-safe, with or without `=` padding.
    ///   3. A file path, whose content is used as raw bytes.
    pub fn from_user_input(s: &str) -> Result<Self, String> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err("key is empty".into());
        }

        // Strip all ASCII whitespace for hex/base64 attempts.
        let clean: String = trimmed
            .chars()
            .filter(|c| !c.is_ascii_whitespace())
            .collect();

        // 1. Hex: only hex digits (and colons, which we strip).
        let hex_clean: String = clean.chars().filter(|c| *c != ':').collect();
        let all_hex = hex_clean.chars().all(|c| c.is_ascii_hexdigit());
        if all_hex && hex_clean.len().is_multiple_of(2) && !hex_clean.is_empty() {
            let bytes = (0..hex_clean.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex_clean[i..i + 2], 16).unwrap())
                .collect();
            return Ok(Self(bytes));
        }

        // 2. Base64, standard (+/) or URL-safe (-_), with or without `=` padding.
        if let Some(bytes) = try_base64(&clean) {
            return Ok(Self(bytes));
        }

        // 3. A file path. Like cryptsetup, pass the file's raw bytes to PBKDF2
        //    without interpreting them.
        let path = std::path::Path::new(trimmed);
        if path.exists() {
            let raw =
                std::fs::read(path).map_err(|e| format!("could not read the key file: {e}"))?;
            return Ok(Self(raw));
        }

        let b64_note = if clean.ends_with('=') {
            let stripped_len = clean.trim_end_matches('=').len();
            let remainder = stripped_len % 4;
            format!(
                "base64: without padding the key has {stripped_len} characters, {remainder} \
                     more than a multiple of 4; valid base64 needs a multiple of 4, so the \
                     input may be truncated."
            )
        } else {
            format!(
                "base64: the key has {} characters, {} more than a multiple of 4.",
                clean.len(),
                clean.len() % 4
            )
        };
        Err(format!(
            "Could not parse the key as hex ({} characters, {}), as base64 or as a file path.\n\
             {b64_note}\n\
             Tip: enter the path to the .key file instead of pasting its contents.",
            hex_clean.len(),
            if all_hex {
                "valid hex but an odd length"
            } else {
                "contains non-hex characters"
            },
        ))
    }
}

/// Tries every base64 variant: standard (+/) and URL-safe (-_), with and
/// without `=` padding. Trailing `=` is stripped before padding is re-added,
/// so an input with wrong padding, such as "abc==", still decodes.
fn try_base64(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    let stripped = s.trim_end_matches('=');
    for pad in 0usize..=2 {
        let padded: std::borrow::Cow<str> = if pad == 0 {
            stripped.into()
        } else {
            format!("{}{}", stripped, "=".repeat(pad)).into()
        };
        let p = padded.as_ref();
        if let Ok(b) = base64::engine::general_purpose::STANDARD.decode(p) {
            return Some(b);
        }
        if let Ok(b) = base64::engine::general_purpose::URL_SAFE.decode(p) {
            return Some(b);
        }
        if let Ok(b) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(p) {
            return Some(b);
        }
    }
    None
}

#[derive(Debug)]
pub enum LuksError {
    Io(io::Error),
    BadMagic,
    UnsupportedVersion(u16),
    UnsupportedCipher(String, String),
    /// No slot matched. It holds a diagnostic trace for the UI log.
    NoValidKeySlot(String),
    DecryptFailed,
}

impl From<io::Error> for LuksError {
    fn from(e: io::Error) -> Self {
        LuksError::Io(e)
    }
}

impl std::fmt::Display for LuksError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LuksError::Io(e) => write!(f, "I/O error: {e}"),
            LuksError::BadMagic => write!(f, "not a LUKS container"),
            LuksError::UnsupportedVersion(v) => write!(f, "LUKS version {v} is not supported"),
            LuksError::UnsupportedCipher(c, m) => write!(f, "unsupported cipher {c}-{m}"),
            LuksError::NoValidKeySlot(diag) => write!(f, "no valid key slot for this key\n{diag}"),
            LuksError::DecryptFailed => write!(f, "decryption failed"),
        }
    }
}

/// LUKS1 header (592 bytes on disk). Parsing checks that `version` is 1 and
/// does not store it, because only LUKS1 is supported.
pub(crate) struct Luks1Header {
    pub(crate) cipher_name: String, // e.g. "aes"
    pub(crate) cipher_mode: String, // e.g. "cbc-essiv:sha256"
    pub(crate) hash_spec: String,   // e.g. "sha1"
    pub(crate) payload_offset: u32, // in sectors
    pub(crate) key_bytes: u32,      // master key length in bytes
    pub(crate) mk_digest: [u8; 20], // master key digest (20 bytes of PBKDF2 output)
    pub(crate) mk_digest_salt: [u8; 32],
    pub(crate) mk_digest_iter: u32,
    #[allow(dead_code)] // Parsed with the header; nothing reads it.
    pub(crate) uuid: String,
    pub(crate) key_slots: Vec<KeySlot>,
}

pub(crate) struct KeySlot {
    pub(crate) active: bool,
    pub(crate) iterations: u32,
    pub(crate) salt: [u8; 32],
    pub(crate) key_material_offset: u32, // in sectors
    pub(crate) stripes: u32,
}

/// Size of the decoy LUKS header the CDJ-3000X `.UPD` carries at offset 0,
/// ahead of the real container.
const DECOY_HEADER_BYTES: u64 = 512;

/// Presents `inner` as though it began at `base`.
///
/// Every seek on the LUKS path is `SeekFrom::Start`, so shifting those is all
/// it takes to parse a container that does not start at byte 0.
struct OffsetReader<R> {
    inner: R,
    base: u64,
}

impl<R: Read> Read for OffsetReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<R: Seek> Seek for OffsetReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let abs = match pos {
            SeekFrom::Start(n) => self.inner.seek(SeekFrom::Start(self.base + n))?,
            other => self.inner.seek(other)?,
        };
        Ok(abs.saturating_sub(self.base))
    }
}

fn read_luks1_header<R: Read + Seek>(r: &mut R) -> Result<Luks1Header, LuksError> {
    r.seek(SeekFrom::Start(0))?;
    let mut buf = [0u8; 592];
    r.read_exact(&mut buf)?;

    if &buf[0..6] != LUKS_MAGIC {
        return Err(LuksError::BadMagic);
    }
    let version = u16::from_be_bytes([buf[6], buf[7]]);
    if version != 1 {
        return Err(LuksError::UnsupportedVersion(version));
    }

    let cipher_name = cstring(&buf[8..40]);
    let cipher_mode = cstring(&buf[40..72]);
    let hash_spec = cstring(&buf[72..104]);
    let payload_offset = u32::from_be_bytes(buf[104..108].try_into().unwrap());
    let key_bytes = u32::from_be_bytes(buf[108..112].try_into().unwrap());
    let mut mk_digest = [0u8; 20];
    mk_digest.copy_from_slice(&buf[112..132]);
    let mut mk_digest_salt = [0u8; 32];
    mk_digest_salt.copy_from_slice(&buf[132..164]);
    let mk_digest_iter = u32::from_be_bytes(buf[164..168].try_into().unwrap());
    let uuid = cstring(&buf[168..208]);

    let mut key_slots = Vec::new();
    for i in 0..8usize {
        let base = 208 + i * 48;
        let active = u32::from_be_bytes(buf[base..base + 4].try_into().unwrap()) == 0x00AC_71F3;
        let iterations = u32::from_be_bytes(buf[base + 4..base + 8].try_into().unwrap());
        let mut salt = [0u8; 32];
        salt.copy_from_slice(&buf[base + 8..base + 40]);
        let key_material_offset = u32::from_be_bytes(buf[base + 40..base + 44].try_into().unwrap());
        let stripes = u32::from_be_bytes(buf[base + 44..base + 48].try_into().unwrap());
        key_slots.push(KeySlot {
            active,
            iterations,
            salt,
            key_material_offset,
            stripes,
        });
    }

    Ok(Luks1Header {
        cipher_name,
        cipher_mode,
        hash_spec,
        payload_offset,
        key_bytes,
        mk_digest,
        mk_digest_salt,
        mk_digest_iter,
        uuid,
        key_slots,
    })
}

/// Decrypt the UPD (LUKS1) container and write the plaintext to `out_path`.
/// `key` is the raw binary keyfile.
pub fn decrypt_upd(upd_path: &Path, key: &LuksKey, out_path: &Path) -> Result<(), LuksError> {
    // The CDJ-3000X hides the real container behind a decoy header with valid
    // LUKS magic, so the decoy parses and only its key slots fail. Try the
    // container at offset 0 and then behind the decoy. The first error is
    // reported, so a .UPD opened with the wrong key still shows its slot
    // diagnostics.
    let mut first_err = None;
    for base in [0, DECOY_HEADER_BYTES] {
        match decrypt_upd_at(upd_path, key, out_path, base) {
            Ok(()) => return Ok(()),
            Err(e) => first_err = first_err.or(Some(e)),
        }
    }
    Err(first_err.expect("one attempt per base"))
}

/// Decrypt the container that begins `base` bytes into `upd_path`.
fn decrypt_upd_at(
    upd_path: &Path,
    key: &LuksKey,
    out_path: &Path,
    base: u64,
) -> Result<(), LuksError> {
    let file = std::fs::File::open(upd_path)?;
    let mut r = OffsetReader { inner: file, base };
    let header = read_luks1_header(&mut r)?;

    if header.cipher_name.to_lowercase() != "aes" {
        return Err(LuksError::UnsupportedCipher(
            header.cipher_name.clone(),
            header.cipher_mode.clone(),
        ));
    }

    // Try each active key slot. This runs before the payload is written, so a
    // failed attempt leaves no partial ISO behind.
    let master_key = try_key_slots(&mut r, &header, &key.0)?;

    decrypt_payload(&mut r, out_path, &header, &master_key).map_err(LuksError::Io)
}

/// Returns the master key of the first active slot that `keyfile` opens.
///
/// Every attempt is traced into the [`LuksError::NoValidKeySlot`] diagnostic,
/// which the firmware wizard shows when the .UPD does not decrypt (wrong key,
/// no active slot, unreadable key material, or a master key size or stripe
/// count that LUKS1 does not allow).
fn try_key_slots<R: Read + Seek>(
    r: &mut R,
    h: &Luks1Header,
    keyfile: &[u8],
) -> Result<Vec<u8>, LuksError> {
    let mut diag = String::new();
    let active_count = h.key_slots.iter().filter(|s| s.active).count();
    let _ = writeln!(
        diag,
        "cipher={}-{}  hash={}  key_bytes={}  payload_sector={}  active_slots={}  key_input_bytes={}",
        h.cipher_name,
        h.cipher_mode,
        h.hash_spec,
        h.key_bytes,
        h.payload_offset,
        active_count,
        keyfile.len()
    );
    if !matches!(h.key_bytes, 32 | 64) {
        let _ = writeln!(diag, "the master key size is not 32 or 64 bytes");
        return Err(LuksError::NoValidKeySlot(diag));
    }

    for (i, slot) in h.key_slots.iter().enumerate().filter(|(_, s)| s.active) {
        if let Some(mk) = open_slot(r, h, i, slot, keyfile, &mut diag) {
            return Ok(mk);
        }
    }
    Err(LuksError::NoValidKeySlot(diag))
}

/// Read slot `i`'s key material and try every keyfile candidate on it.
fn open_slot<R: Read + Seek>(
    r: &mut R,
    h: &Luks1Header,
    i: usize,
    slot: &KeySlot,
    keyfile: &[u8],
    diag: &mut String,
) -> Option<Vec<u8>> {
    let km_size = h.key_bytes as usize * slot.stripes as usize;
    let _ = writeln!(
        diag,
        "slot {i}: iters={}  km_sector={}  stripes={}  km_bytes={}",
        slot.iterations, slot.key_material_offset, slot.stripes, km_size
    );
    if slot.stripes != LUKS1_STRIPES {
        let _ = writeln!(diag, "slot {i}: the stripe count is not 4000");
        return None;
    }
    let _ = writeln!(diag, "slot {i}: keyfile_len={}", keyfile.len());

    let km_offset = slot.key_material_offset as u64 * LUKS_SECTOR;
    if let Err(e) = r.seek(SeekFrom::Start(km_offset)) {
        let _ = writeln!(diag, "slot {i}: seek to {km_offset} failed: {e}");
        return None;
    }
    let mut km = vec![0u8; km_size];
    if let Err(e) = r.read_exact(&mut km) {
        let _ = writeln!(diag, "slot {i}: read {km_size} bytes failed: {e}");
        return None;
    }

    for kf in keyfile_candidates(keyfile, i, diag) {
        if let Some(mk) = try_keyfile(h, slot, &km, &kf) {
            let _ = writeln!(diag, "slot {i}: digest OK");
            return Some(mk);
        }
    }

    let _ = writeln!(diag, "slot {i}: the digest does not match");
    let _ = writeln!(diag, "  expected: {:02x?}", h.mk_digest);
    let _ = writeln!(diag, "  computed: (no candidate matched)");
    None
}

/// Returns the keyfile as given and, when it is base64 text, its decoded bytes.
fn keyfile_candidates(keyfile: &[u8], i: usize, diag: &mut String) -> Vec<Vec<u8>> {
    let mut candidates = vec![keyfile.to_vec()];
    if let Ok(text) = std::str::from_utf8(keyfile) {
        let stripped: String = text.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        if let Some(decoded) = try_base64(&stripped) {
            let _ = writeln!(
                diag,
                "slot {i}: also trying base64-decoded keyfile ({} bytes)",
                decoded.len()
            );
            candidates.push(decoded);
        }
    }
    candidates
}

/// Returns the master key that keyfile `kf` recovers from the slot's key
/// material `km`, or `None`.
///
/// cryptsetup's LUKS_decrypt_from_storage calls crypt_storage_decrypt(s, 0, ...),
/// so the XTS IV starts at sector 0 whatever the key material offset. For
/// non-standard images, this also tries numbering the sectors from the key
/// material's own sector, and swapping the key halves.
fn try_keyfile(h: &Luks1Header, slot: &KeySlot, km: &[u8], kf: &[u8]) -> Option<Vec<u8>> {
    let mut sk = vec![0u8; h.key_bytes as usize];
    pbkdf2_derive(&h.hash_spec, kf, &slot.salt, slot.iterations, &mut sk).ok()?;
    let mode_lc = h.cipher_mode.to_lowercase();
    let variants = [0u64, slot.key_material_offset as u64]
        .into_iter()
        .flat_map(|first_sector| [(first_sector, false), (first_sector, true)]);
    for (first_sector, swap) in variants {
        let (data_key, tweak_key) = if swap {
            (&sk[32..], &sk[..32])
        } else {
            (&sk[..32], &sk[32..])
        };
        let Some(material) = xts_decrypt(km, data_key, tweak_key, first_sector, &mode_lc) else {
            continue;
        };
        let mk = af_merge(
            &material,
            h.key_bytes as usize,
            slot.stripes as usize,
            &h.hash_spec,
        );
        if digest_matches(h, &mk) {
            return Some(mk);
        }
    }
    None
}

/// Decrypts `km` with AES-256 XTS, numbering the sectors from `first_sector`.
/// Returns `None` when a key half is not 32 bytes.
fn xts_decrypt(
    km: &[u8],
    data_key: &[u8],
    tweak_key: &[u8],
    first_sector: u64,
    mode_lc: &str,
) -> Option<Vec<u8>> {
    let xts = Xts128::new(
        Aes256::new_from_slice(data_key).ok()?,
        Aes256::new_from_slice(tweak_key).ok()?,
    );
    let mut material = km.to_vec();
    for (idx, chunk) in material.chunks_mut(512).enumerate() {
        xts.decrypt_sector(chunk, xts_tweak(first_sector + idx as u64, mode_lc));
    }
    Some(material)
}

/// Returns whether `mk` matches the header's master key digest.
fn digest_matches(h: &Luks1Header, mk: &[u8]) -> bool {
    let mut check = [0u8; 20];
    pbkdf2_derive(
        &h.hash_spec,
        mk,
        &h.mk_digest_salt,
        h.mk_digest_iter,
        &mut check,
    )
    .is_ok()
        && check == h.mk_digest
}

fn cstring(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

#[cfg(test)]
mod tests;
