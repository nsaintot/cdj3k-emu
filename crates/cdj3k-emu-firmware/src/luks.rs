//! LUKS1 decryption - no external tools required.
//!
//! Supports the two cipher suites observed in the wild:
//!   aes-cbc-essiv:sha256  (key_bytes=32, hash=sha1)
//!   aes-xts-plain64       (key_bytes=64, hash=sha256)
//!
//! Flow: read LUKS1 header → dispatch on hash_spec/cipher_mode →
//! PBKDF2 slot key → AES-XTS or AES-CBC decrypt key material →
//! AFC merge → verify master key digest → decrypt payload.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use aes::cipher::KeyInit;
use aes::Aes256;
use xts_mode::Xts128;

const LUKS_MAGIC: &[u8; 6] = b"LUKS\xba\xbe";
pub(crate) const LUKS_SECTOR: u64 = 512;

mod crypto;
use crypto::{af_merge, decrypt_payload, pbkdf2_derive, xts_tweak};

/// A LUKS keyfile (raw binary, typically 32 bytes for AES-256).
pub struct LuksKey(pub Vec<u8>);

impl LuksKey {
    pub fn from_file(path: &Path) -> io::Result<Self> {
        Ok(Self(std::fs::read(path)?))
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Accept a key from user text input, trying in order:
    ///   1. Hex string (0-9 a-f A-F, whitespace/colons stripped)
    ///   2. Base64 (standard or URL-safe, with/without `==` padding)
    ///   3. File path - reads the file, then tries to parse its content as
    ///      hex/base64 (common for PEM-style key files), falling back to raw bytes.
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
        if all_hex && hex_clean.len() % 2 == 0 && !hex_clean.is_empty() {
            let bytes = (0..hex_clean.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex_clean[i..i + 2], 16).unwrap())
                .collect();
            return Ok(Self(bytes));
        }

        // 2. Base64 - standard (+/), URL-safe (-_), with and without = padding.
        if let Some(bytes) = try_base64(&clean) {
            return Ok(Self(bytes));
        }

        // 3. File path: read and return raw bytes exactly as cryptsetup does.
        //    cryptsetup never interprets keyfile content - it passes raw bytes to PBKDF2.
        let path = std::path::Path::new(trimmed);
        if path.exists() {
            let raw = std::fs::read(path).map_err(|e| format!("could not read key file: {e}"))?;
            return Ok(Self(raw));
        }

        let b64_note = if clean.ends_with('=') {
            let stripped_len = clean.trim_end_matches('=').len();
            let remainder = stripped_len % 4;
            format!(
                "base64: stripped to {stripped_len} chars, remainder mod 4 = {remainder} \
                     (need 0 for valid base64 - input may be truncated)"
            )
        } else {
            format!(
                "base64: {} chars, remainder mod 4 = {}",
                clean.len(),
                clean.len() % 4
            )
        };
        Err(format!(
            "could not parse key - tried hex ({} chars, {}), base64, and file path\n\
             {b64_note}\n\
             Tip: enter the path to the .key file directly instead of pasting its contents",
            hex_clean.len(),
            if all_hex {
                "valid hex but odd length"
            } else {
                "contains non-hex chars"
            },
        ))
    }
}

/// Try all base64 variants: standard (+/) and URL-safe (-_), with and without
/// `=` padding.  Strips any existing trailing `=` before re-adding, so inputs
/// like "abc==" (already padded but wrong length) are handled correctly.
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
    /// No slot matched; carries a diagnostic dump for display in the UI log.
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
            LuksError::Io(e) => write!(f, "I/O: {e}"),
            LuksError::BadMagic => write!(f, "not a LUKS container"),
            LuksError::UnsupportedVersion(v) => write!(f, "LUKS version {v} not supported"),
            LuksError::UnsupportedCipher(c, m) => write!(f, "unsupported cipher {c}-{m}"),
            LuksError::NoValidKeySlot(diag) => write!(f, "no valid key slot for this key\n{diag}"),
            LuksError::DecryptFailed => write!(f, "decryption or HMAC check failed"),
        }
    }
}

/// LUKS1 header (592 bytes on disk). `version` is validated to `1` during
/// parsing and then dropped — only LUKS1 is supported, so once it passes the
/// check it carries no information.
pub(crate) struct Luks1Header {
    pub(crate) cipher_name: String, // e.g. "aes"
    pub(crate) cipher_mode: String, // e.g. "cbc-essiv:sha256"
    pub(crate) hash_spec: String,   // e.g. "sha1"
    pub(crate) payload_offset: u32, // in sectors
    pub(crate) key_bytes: u32,      // master key length in bytes
    pub(crate) mk_digest: [u8; 20], // master key digest (SHA1-based)
    pub(crate) mk_digest_salt: [u8; 32],
    pub(crate) mk_digest_iter: u32,
    #[allow(dead_code)] // parsed for diagnostics but unused by the decrypt path
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
    let mut file = std::fs::File::open(upd_path)?;
    let header = read_luks1_header(&mut file)?;

    // Validate cipher.
    if header.cipher_name.to_lowercase() != "aes" {
        return Err(LuksError::UnsupportedCipher(
            header.cipher_name.clone(),
            header.cipher_mode.clone(),
        ));
    }

    // Try each active key slot.
    let master_key = try_key_slots(&mut file, &header, &key.0)?;

    // Decrypt payload and stream to output.
    decrypt_payload(&mut file, out_path, &header, &master_key).map_err(LuksError::Io)
}

fn try_key_slots<R: Read + Seek>(
    r: &mut R,
    h: &Luks1Header,
    keyfile: &[u8],
) -> Result<Vec<u8>, LuksError> {
    let mut diag = String::new();

    // Diagnostic trace of slot-decryption attempts.  Accumulated into `diag`
    // and surfaced in `LuksError::NoActiveSlot` so the firmware wizard can
    // show users exactly why their .UPD didn't decrypt (wrong key, no active
    // slot, etc.).  Not mirrored to stderr — the wizard owns the channel.
    macro_rules! d {
        ($($arg:tt)*) => {{
            let line = format!($($arg)*);
            diag.push_str(&line);
            diag.push('\n');
        }};
    }

    let active_count = h.key_slots.iter().filter(|s| s.active).count();
    d!("cipher={}-{}  hash={}  key_bytes={}  payload_sector={}  active_slots={}  key_input_bytes={}",
        h.cipher_name, h.cipher_mode, h.hash_spec,
        h.key_bytes, h.payload_offset, active_count, keyfile.len());

    for (i, slot) in h.key_slots.iter().enumerate() {
        if !slot.active {
            continue;
        }

        let km_size = h.key_bytes as usize * slot.stripes as usize;
        d!(
            "slot {i}: iters={}  km_sector={}  stripes={}  km_bytes={}",
            slot.iterations,
            slot.key_material_offset,
            slot.stripes,
            km_size
        );
        d!("slot {i}: keyfile_len={}", keyfile.len());

        // 1. PBKDF2 (hash from header): keyfile → slot key (refined in brute-force below).
        let mut slot_key = vec![0u8; h.key_bytes as usize];
        if pbkdf2_derive(
            &h.hash_spec,
            keyfile,
            &slot.salt,
            slot.iterations,
            &mut slot_key,
        )
        .is_err()
        {
            d!("slot {i}: PBKDF2 failed (invalid key length?)");
            continue;
        }

        // 2. Read raw key material.
        let km_offset = slot.key_material_offset as u64 * LUKS_SECTOR;
        if let Err(e) = r.seek(SeekFrom::Start(km_offset)) {
            d!("slot {i}: seek to {km_offset} failed: {e}");
            continue;
        }
        let mut km = vec![0u8; km_size];
        if let Err(e) = r.read_exact(&mut km) {
            d!("slot {i}: read {km_size} bytes failed: {e}");
            continue;
        }
        // Brute-force: (keyfile variant) × (sector_start) × (key_order) = up to 8 combinations.
        let km_sec_abs = slot.key_material_offset as u64;
        let mode_lc = h.cipher_mode.to_lowercase();

        // Build candidate keyfile inputs: raw bytes, and base64-decoded if the file looks like text.
        let mut kf_candidates: Vec<(&'static str, Vec<u8>)> = vec![("raw", keyfile.to_vec())];
        if let Ok(text) = std::str::from_utf8(keyfile) {
            let stripped: String = text.chars().filter(|c| !c.is_ascii_whitespace()).collect();
            if let Some(decoded) = try_base64(&stripped) {
                d!(
                    "slot {i}: also trying base64-decoded keyfile ({} bytes)",
                    decoded.len()
                );
                kf_candidates.push(("b64dec", decoded));
            }
        }

        // cryptsetup's LUKS_decrypt_from_storage calls crypt_storage_decrypt(s, 0, ...)
        // so the XTS IV always starts at sector 0 regardless of km_off.
        // We still probe both 0 and km_sec_abs as a fallback for non-standard images.
        for (_kf_label, kf) in &kf_candidates {
            let mut sk = vec![0u8; h.key_bytes as usize];
            if pbkdf2_derive(&h.hash_spec, kf, &slot.salt, slot.iterations, &mut sk).is_err() {
                continue;
            }
            for &sec in &[0u64, km_sec_abs] {
                for &swap in &[false, true] {
                    let (s1, s2) = if swap {
                        (&sk[32..], &sk[..32])
                    } else {
                        (&sk[..32], &sk[32..])
                    };
                    if let (Ok(c1), Ok(c2)) =
                        (Aes256::new_from_slice(s1), Aes256::new_from_slice(s2))
                    {
                        let xts_v = Xts128::new(c1, c2);
                        let mut km_v = km.clone();
                        for (idx, chunk) in km_v.chunks_mut(512).enumerate() {
                            xts_v.decrypt_sector(chunk, xts_tweak(sec + idx as u64, &mode_lc));
                        }
                        let mk_v = af_merge(
                            &km_v,
                            h.key_bytes as usize,
                            slot.stripes as usize,
                            &h.hash_spec,
                        );
                        let mut ck = vec![0u8; 20];
                        let ok = pbkdf2_derive(
                            &h.hash_spec,
                            &mk_v,
                            &h.mk_digest_salt,
                            h.mk_digest_iter,
                            &mut ck,
                        )
                        .is_ok()
                            && ck[..] == h.mk_digest[..];
                        if ok {
                            d!("slot {i}: digest OK");
                            return Ok(mk_v);
                        }
                    }
                }
            }
        }

        d!("slot {i}: digest MISMATCH");
        d!("  expected: {:02x?}", h.mk_digest);
        d!("  computed: (none matched)");
    }

    Err(LuksError::NoValidKeySlot(diag))
}

fn cstring(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}
