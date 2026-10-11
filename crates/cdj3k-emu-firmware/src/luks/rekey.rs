//! Add a per-unit keyslot to a CDJ-3000X `cabinet.img`.
//!
//! The cabinet carries the Widevine material and the Device Library Plus key
//! file. Every deck's copy comes from the container the firmware ships as
//! `images/cabinet.img`. The copies share its UUID and master key and differ
//! only in their keyslot. The firmware ships it keyed for a factory passphrase
//! that the guest never uses. The installer opens that slot with the passphrase `genkey_pb` computes from
//! the `model` and `images/images.tar.gz` ([`vendor_passphrase`]), recovers the
//! master key, and adds a slot keyed for this unit's serial.
//!
//! The guest opens the unit slot on every boot with `genkey_pr | initoptenv`,
//! where `genkey_pr` derives the passphrase from the U-Boot `model` variable
//! and the `Serial` line of `/proc/cpuinfo` - see [`cabinet_passphrase`].

use aes::cipher::{BlockCipher, BlockDecrypt, BlockEncrypt, KeyInit};
use aes::{Aes128, Aes256};
use sha2::{Digest, Sha512};
use xts_mode::Xts128;

use super::crypto::{af_merge, af_split, pbkdf2_derive, random_bytes};
use super::{LUKS1_STRIPES, LUKS_SECTOR};

/// PBKDF2 rounds for the added keyslot, which the guest runs once per boot.
/// The factory slot's 2,868,880 rounds take about 30 s under TCG on x86_64.
const SLOT_ITERATIONS: u32 = 200_000;

const SLOT_ACTIVE: u32 = 0x00AC_71F3;
const SLOT_INACTIVE: u32 = 0x0000_DEAD;
const SLOT_COUNT: usize = 8;
const HEADER_BYTES: usize = 592;
const SLOT_TABLE_OFFSET: usize = 208;
const SLOT_ENTRY_BYTES: usize = 48;
const SECTOR: usize = LUKS_SECTOR as usize;

#[derive(Debug)]
pub enum RekeyError {
    /// Not a LUKS1 container, or too short to hold the header.
    NotLuks1,
    /// No active keyslot opened with the factory passphrase, so the master
    /// key could not be recovered. Either the image is not a CDJ-3000X
    /// cabinet, or the seed came from a different firmware.
    VendorSlotLocked,
    /// The header declares a cipher, mode or hash this code does not handle.
    UnsupportedCipher(String),
    /// A header field holds a value LUKS1 does not allow.
    Malformed(&'static str),
    /// All eight keyslots are in use.
    NoFreeSlot,
    /// A keyslot's material region or the payload falls outside the image.
    Truncated,
    /// The serial is not the 16 hex digits `genkey_pr` expects.
    BadSerial,
}

impl std::fmt::Display for RekeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RekeyError::NotLuks1 => write!(f, "not a LUKS1 container"),
            RekeyError::VendorSlotLocked => {
                write!(f, "no keyslot opened with the factory passphrase")
            }
            RekeyError::UnsupportedCipher(c) => write!(f, "unsupported cipher {c}"),
            RekeyError::Malformed(what) => write!(f, "malformed LUKS1 header: {what}"),
            RekeyError::NoFreeSlot => write!(f, "all eight keyslots are in use"),
            RekeyError::Truncated => write!(f, "the image is shorter than its header declares"),
            RekeyError::BadSerial => write!(f, "the SoC serial is not 16 hex digits"),
        }
    }
}

/// The LUKS1 header fields the keyslot code uses, validated against the image
/// they were read from: every keyslot region lies inside the image, after the
/// header and before the payload, and no two regions overlap.
struct Luks1Layout {
    /// Lowercase, one of `sha1` / `sha256`.
    hash_spec: String,
    /// Master key length: 32 (XTS-AES-128) or 64 (XTS-AES-256).
    key_bytes: usize,
    mk_digest: [u8; 20],
    mk_salt: [u8; 32],
    mk_iter: u32,
    slots: [Keyslot; SLOT_COUNT],
}

#[derive(Clone, Copy)]
struct Keyslot {
    active: bool,
    iterations: u32,
    salt: [u8; 32],
    /// Byte range of the key material, `key_bytes * stripes` long.
    km_start: usize,
    km_end: usize,
    stripes: usize,
}

fn be32(image: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([image[at], image[at + 1], image[at + 2], image[at + 3]])
}

/// Converts a header field counted in sectors to a byte offset.
fn sector_offset(sectors: u32, what: &'static str) -> Result<usize, RekeyError> {
    (sectors as usize)
        .checked_mul(SECTOR)
        .ok_or(RekeyError::Malformed(what))
}

impl Luks1Layout {
    fn parse(image: &[u8]) -> Result<Self, RekeyError> {
        let hash_spec = parse_identity(image)?;
        let key_bytes = match be32(image, 108) {
            32 => 32usize,
            64 => 64usize,
            _ => {
                return Err(RekeyError::Malformed(
                    "master key size is not 32 or 64 bytes",
                ))
            }
        };
        let payload_start = sector_offset(be32(image, 104), "payload offset")?;
        if payload_start < HEADER_BYTES {
            return Err(RekeyError::Malformed(
                "the payload offset is inside the header",
            ));
        }
        if payload_start > image.len() {
            return Err(RekeyError::Truncated);
        }

        let mk_digest: [u8; 20] = image[112..132].try_into().unwrap();
        let mk_salt: [u8; 32] = image[132..164].try_into().unwrap();
        let mk_iter = be32(image, 164);
        if mk_iter == 0 {
            return Err(RekeyError::Malformed("mk-digest iterations are zero"));
        }

        let mut slots = [Keyslot::UNUSED; SLOT_COUNT];
        for (index, slot) in slots.iter_mut().enumerate() {
            *slot = Keyslot::parse(image, index, key_bytes, payload_start)?;
        }
        check_disjoint(&slots)?;

        Ok(Luks1Layout {
            hash_spec,
            key_bytes,
            mk_digest,
            mk_salt,
            mk_iter,
            slots,
        })
    }
}

/// Checks the magic, version, cipher and mode, and returns the hash spec in
/// lowercase.
fn parse_identity(image: &[u8]) -> Result<String, RekeyError> {
    if image.len() < HEADER_BYTES || &image[0..6] != super::LUKS_MAGIC {
        return Err(RekeyError::NotLuks1);
    }
    if u16::from_be_bytes([image[6], image[7]]) != 1 {
        return Err(RekeyError::NotLuks1);
    }

    let cipher = cstr(&image[8..40]);
    let mode = cstr(&image[40..72]);
    if !cipher.eq_ignore_ascii_case("aes") || !mode.eq_ignore_ascii_case("xts-plain64") {
        return Err(RekeyError::UnsupportedCipher(format!("{cipher}-{mode}")));
    }
    let hash_spec = cstr(&image[72..104]).to_ascii_lowercase();
    if hash_spec != "sha1" && hash_spec != "sha256" {
        return Err(RekeyError::UnsupportedCipher(hash_spec));
    }
    Ok(hash_spec)
}

impl Keyslot {
    const UNUSED: Keyslot = Keyslot {
        active: false,
        iterations: 0,
        salt: [0; 32],
        km_start: 0,
        km_end: 0,
        stripes: 0,
    };

    /// Parses header entry `index` and checks that its key material lies
    /// between the header and the payload.
    fn parse(
        image: &[u8],
        index: usize,
        key_bytes: usize,
        payload_start: usize,
    ) -> Result<Self, RekeyError> {
        let entry = SLOT_TABLE_OFFSET + index * SLOT_ENTRY_BYTES;
        let active = match be32(image, entry) {
            SLOT_ACTIVE => true,
            SLOT_INACTIVE => false,
            _ => {
                return Err(RekeyError::Malformed(
                    "keyslot state is neither active nor inactive",
                ))
            }
        };
        let iterations = be32(image, entry + 4);
        if active && iterations == 0 {
            return Err(RekeyError::Malformed("keyslot iterations are zero"));
        }
        let stripes = be32(image, entry + 44);
        if stripes != LUKS1_STRIPES {
            return Err(RekeyError::Malformed("keyslot stripe count is not 4000"));
        }
        let km_start = sector_offset(be32(image, entry + 40), "key material offset")?;
        if km_start < HEADER_BYTES {
            return Err(RekeyError::Malformed(
                "the key material is inside the header",
            ));
        }
        let km_end = km_start
            .checked_add(key_bytes * stripes as usize)
            .ok_or(RekeyError::Malformed("key material end overflows"))?;
        if km_end > image.len() {
            return Err(RekeyError::Truncated);
        }
        if km_end > payload_start {
            return Err(RekeyError::Malformed("key material overlaps the payload"));
        }
        Ok(Keyslot {
            active,
            iterations,
            salt: image[entry + 8..entry + 40].try_into().unwrap(),
            km_start,
            km_end,
            stripes: stripes as usize,
        })
    }
}

/// Refuses two keyslots whose material regions overlap.
fn check_disjoint(slots: &[Keyslot]) -> Result<(), RekeyError> {
    for (i, a) in slots.iter().enumerate() {
        for b in &slots[i + 1..] {
            if a.km_start < b.km_end && b.km_start < a.km_end {
                return Err(RekeyError::Malformed("keyslot material regions overlap"));
            }
        }
    }
    Ok(())
}

/// The cabinet passphrase `genkey_pr` computes, as it writes it to stdout:
/// 128 lowercase hex digits with no trailing newline.
///
/// ```text
/// N   = strtol(last two digits of the serial, 16)
/// KEY = sha512hex( (model + serial) repeated N + 1 times )
/// ```
///
/// `model_env` is the U-Boot `model` variable the eMMC carries
/// ([`cdj3k_emu_panel::ModelSpec::model_env`]), `soc_serial` the value guest
/// patch 12 publishes as the `Serial` line of `/proc/cpuinfo`.
pub fn cabinet_passphrase(model_env: &str, soc_serial: &str) -> Result<Vec<u8>, RekeyError> {
    if soc_serial.len() != 16 || !soc_serial.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(RekeyError::BadSerial);
    }
    let reps = u32::from_str_radix(&soc_serial[14..], 16).map_err(|_| RekeyError::BadSerial)? + 1;

    let unit = format!("{model_env}{soc_serial}");
    let mut h = Sha512::new();
    for _ in 0..reps {
        h.update(unit.as_bytes());
    }
    Ok(hex::encode(h.finalize()).into_bytes())
}

/// The factory cabinet passphrase `genkey_pb` computes, as it writes it to
/// stdout: 128 lowercase hex digits with no trailing newline.
///
/// ```text
/// KEY = sha512hex( model + sha512hex(seed) )
/// ```
///
/// It opens the slot the firmware ships (slot 0 on a vendor cabinet), whose
/// master key every cabinet of that model shares. `model_env` is the U-Boot
/// `model` variable ([`cdj3k_emu_panel::ModelSpec::model_env`]); `seed` is the
/// file in the same `.UPD` package as `cabinet.img` whose hash forms the
/// passphrase ([`crate::read_cabinet_seed`]).
pub fn vendor_passphrase(model_env: &str, seed: &[u8]) -> Vec<u8> {
    let inner = hex::encode(Sha512::digest(seed));
    let outer = Sha512::digest(format!("{model_env}{inner}").as_bytes());
    hex::encode(outer).into_bytes()
}

/// Add a keyslot to `image` that opens with `unit_passphrase`, as slot 0.
///
/// The master key is recovered first by opening a factory slot with
/// `vendor_passphrase` ([`vendor_passphrase`]); a container that no factory
/// slot opens is refused and left unchanged. The new slot's material goes in a
/// free region, and its header entry trades places with slot 0's, because
/// cryptsetup tries active slots in order and the factory slot costs 2,868,880
/// PBKDF2 rounds.  The payload and the existing slots' material are untouched.
pub fn add_keyslot(
    image: &mut [u8],
    vendor_passphrase: &[u8],
    unit_passphrase: &[u8],
) -> Result<usize, RekeyError> {
    add_keyslot_with(image, vendor_passphrase, unit_passphrase, SLOT_ITERATIONS)
}

/// [`add_keyslot`] with the new slot's PBKDF2 round count as a parameter.
fn add_keyslot_with(
    image: &mut [u8],
    vendor_passphrase: &[u8],
    unit_passphrase: &[u8],
    iterations: u32,
) -> Result<usize, RekeyError> {
    let layout = Luks1Layout::parse(image)?;

    // Recover the master key from a factory slot. This yields the key to wrap
    // into the new slot and proves that the container is a CDJ-3000X cabinet.
    let master_key = unlock(&layout, image, vendor_passphrase)?;

    let index = layout
        .slots
        .iter()
        .position(|s| !s.active)
        .ok_or(RekeyError::NoFreeSlot)?;
    write_slot(
        image,
        &layout,
        index,
        &master_key,
        unit_passphrase,
        iterations,
    )?;
    swap_slot_entries(image, 0, index);
    Ok(0)
}

/// Exchanges two keyslots' header entries. Each entry keeps pointing at its
/// own material.
fn swap_slot_entries(image: &mut [u8], a: usize, b: usize) {
    if a == b {
        return;
    }
    let (lo, hi) = (a.min(b), a.max(b));
    let at = |i: usize| SLOT_TABLE_OFFSET + i * SLOT_ENTRY_BYTES;
    let (head, tail) = image.split_at_mut(at(hi));
    head[at(lo)..at(lo) + SLOT_ENTRY_BYTES].swap_with_slice(&mut tail[..SLOT_ENTRY_BYTES]);
}

/// Keys slot `index` of `image` for `passphrase`. It draws a fresh salt,
/// derives the slot key with PBKDF2, AF-splits and XTS-encrypts `master_key`
/// into the region `layout` reserves, and marks the slot's header entry active.
fn write_slot(
    image: &mut [u8],
    layout: &Luks1Layout,
    index: usize,
    master_key: &[u8],
    passphrase: &[u8],
    iterations: u32,
) -> Result<(), RekeyError> {
    let slot = &layout.slots[index];
    let salt = random_bytes(32);
    let mut slot_key = vec![0u8; layout.key_bytes];
    pbkdf2_derive(
        &layout.hash_spec,
        passphrase,
        &salt,
        iterations,
        &mut slot_key,
    )
    .map_err(|()| RekeyError::UnsupportedCipher(layout.hash_spec.clone()))?;

    let mut material = af_split(master_key, slot.stripes, &layout.hash_spec);
    xts_material(&mut material, &slot_key, true)?;
    image[slot.km_start..slot.km_end].copy_from_slice(&material);

    let entry = SLOT_TABLE_OFFSET + index * SLOT_ENTRY_BYTES;
    image[entry..entry + 4].copy_from_slice(&SLOT_ACTIVE.to_be_bytes());
    image[entry + 4..entry + 8].copy_from_slice(&iterations.to_be_bytes());
    image[entry + 8..entry + 40].copy_from_slice(&salt);
    Ok(())
}

/// Recovers the container's master key by opening an active keyslot with
/// `passphrase`. It parses and validates the header, then calls [`unlock`].
#[cfg(test)]
fn open_master_key(
    image: &[u8],
    passphrase: &[u8],
    hash_spec: &str,
) -> Result<Vec<u8>, RekeyError> {
    let layout = Luks1Layout::parse(image)?;
    if !layout.hash_spec.eq_ignore_ascii_case(hash_spec) {
        return Err(RekeyError::UnsupportedCipher(hash_spec.to_string()));
    }
    unlock(&layout, image, passphrase)
}

/// Returns the master key of the first active keyslot that `passphrase` opens.
/// For each slot it derives the slot key with PBKDF2, XTS-decrypts and
/// AF-merges the key material, and accepts the result only if it matches the
/// header's mk-digest. `image` is not modified.
fn unlock(layout: &Luks1Layout, image: &[u8], passphrase: &[u8]) -> Result<Vec<u8>, RekeyError> {
    let hash_spec = &layout.hash_spec;
    for slot in layout.slots.iter().filter(|s| s.active) {
        let mut slot_key = vec![0u8; layout.key_bytes];
        if pbkdf2_derive(
            hash_spec,
            passphrase,
            &slot.salt,
            slot.iterations,
            &mut slot_key,
        )
        .is_err()
        {
            continue;
        }
        let mut material = image[slot.km_start..slot.km_end].to_vec();
        xts_material(&mut material, &slot_key, false)?;
        let mk = af_merge(&material, layout.key_bytes, slot.stripes, hash_spec);

        let mut digest = [0u8; 20];
        if pbkdf2_derive(hash_spec, &mk, &layout.mk_salt, layout.mk_iter, &mut digest).is_ok()
            && digest == layout.mk_digest
        {
            return Ok(mk);
        }
    }
    Err(RekeyError::VendorSlotLocked)
}

/// Encrypts or decrypts keyslot material in place with XTS. The 512-byte
/// sectors are numbered from 0, because cryptsetup encrypts keyslot material
/// from sector 0 whatever its offset. The key's first half is the data key and
/// the second the tweak key: AES-128 halves for a 32-byte key, AES-256 halves
/// for a 64-byte one.
fn xts_material(material: &mut [u8], slot_key: &[u8], encrypt: bool) -> Result<(), RekeyError> {
    match slot_key.len() {
        32 => xts_sectors::<Aes128>(material, slot_key, encrypt),
        64 => xts_sectors::<Aes256>(material, slot_key, encrypt),
        _ => Err(RekeyError::Malformed(
            "master key size is not 32 or 64 bytes",
        )),
    }
}

fn xts_sectors<C>(material: &mut [u8], key: &[u8], encrypt: bool) -> Result<(), RekeyError>
where
    C: BlockCipher + BlockEncrypt + BlockDecrypt + KeyInit,
{
    let (data_key, tweak_key) = key.split_at(key.len() / 2);
    let bad_key = |_| RekeyError::Malformed("master key size is not 32 or 64 bytes");
    let xts = Xts128::new(
        C::new_from_slice(data_key).map_err(bad_key)?,
        C::new_from_slice(tweak_key).map_err(bad_key)?,
    );
    if !material.len().is_multiple_of(16) {
        return Err(RekeyError::Malformed(
            "key material is not whole AES blocks",
        ));
    }
    for (sector, chunk) in material.chunks_mut(SECTOR).enumerate() {
        let mut tweak = [0u8; 16];
        tweak[..8].copy_from_slice(&(sector as u64).to_le_bytes());
        if encrypt {
            xts.encrypt_sector(chunk, tweak);
        } else {
            xts.decrypt_sector(chunk, tweak);
        }
    }
    Ok(())
}

fn cstr(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

#[cfg(test)]
pub(super) mod tests;
