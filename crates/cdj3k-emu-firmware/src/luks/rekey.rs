//! Add a per-unit keyslot to a CDJ-3000X `cabinet.img`.
//!
//! The cabinet carries the Widevine material and the Device Library Plus key
//! file.  Every deck's copy descends from the one container the firmware ships
//! as `images/cabinet.img`: same UUID, same master key, a different keyslot.
//! The firmware ships it keyed for a factory passphrase the guest never uses;
//! the installer opens that slot with the passphrase `genkey_pb` computes from
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
use super::LUKS_SECTOR;

/// PBKDF2 rounds for the keyslot we add.
///
/// The guest runs them once per boot on an emulated RK3399, so a high count
/// only slows boot: this is a quarter of what the factory slot uses.
const SLOT_ITERATIONS: u32 = 200_000;

const SLOT_ACTIVE: u32 = 0x00AC_71F3;
const SLOT_INACTIVE: u32 = 0x0000_DEAD;
/// The anti-forensic stripe count cryptsetup writes and requires for LUKS1.
const LUKS1_STRIPES: u32 = 4000;
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
    /// key could not be recovered - not a CDJ-3000X cabinet, or the wrong
    /// firmware's `images.tar.gz`.
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

impl Luks1Layout {
    fn parse(image: &[u8]) -> Result<Self, RekeyError> {
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

        let key_bytes = match be32(image, 108) {
            32 => 32usize,
            64 => 64usize,
            _ => {
                return Err(RekeyError::Malformed(
                    "master key size is not 32 or 64 bytes",
                ))
            }
        };
        let payload_start = (be32(image, 104) as usize)
            .checked_mul(SECTOR)
            .ok_or(RekeyError::Malformed("payload offset"))?;
        if payload_start < HEADER_BYTES {
            return Err(RekeyError::Malformed("payload offset inside the header"));
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

        let mut slots = [Keyslot {
            active: false,
            iterations: 0,
            salt: [0; 32],
            km_start: 0,
            km_end: 0,
            stripes: 0,
        }; SLOT_COUNT];
        for (index, slot) in slots.iter_mut().enumerate() {
            let entry = SLOT_TABLE_OFFSET + index * SLOT_ENTRY_BYTES;
            let active = match be32(image, entry) {
                SLOT_ACTIVE => true,
                SLOT_INACTIVE => false,
                _ => return Err(RekeyError::Malformed("keyslot state")),
            };
            let iterations = be32(image, entry + 4);
            if active && iterations == 0 {
                return Err(RekeyError::Malformed("keyslot iterations are zero"));
            }
            let stripes = be32(image, entry + 44);
            if stripes != LUKS1_STRIPES {
                return Err(RekeyError::Malformed("keyslot stripes"));
            }
            let km_start = (be32(image, entry + 40) as usize)
                .checked_mul(SECTOR)
                .ok_or(RekeyError::Malformed("key material offset"))?;
            if km_start < HEADER_BYTES {
                return Err(RekeyError::Malformed("key material inside the header"));
            }
            let km_end = km_start
                .checked_add(key_bytes * stripes as usize)
                .ok_or(RekeyError::Malformed("key material offset"))?;
            if km_end > image.len() {
                return Err(RekeyError::Truncated);
            }
            if km_end > payload_start {
                return Err(RekeyError::Malformed("key material overlaps the payload"));
            }
            *slot = Keyslot {
                active,
                iterations,
                salt: image[entry + 8..entry + 40].try_into().unwrap(),
                km_start,
                km_end,
                stripes: stripes as usize,
            };
        }
        for (i, a) in slots.iter().enumerate() {
            for b in &slots[i + 1..] {
                if a.km_start < b.km_end && b.km_start < a.km_end {
                    return Err(RekeyError::Malformed("keyslot material regions overlap"));
                }
            }
        }

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

/// The cabinet passphrase `genkey_pr` computes, as it writes it to stdout:
/// 128 lowercase hex digits, no trailing newline.
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
/// stdout: 128 lowercase hex digits, no trailing newline.
///
/// ```text
/// KEY = sha512hex( model + sha512hex(images/images.tar.gz) )
/// ```
///
/// It opens the slot the firmware ships (slot 0 on a vendor cabinet), whose
/// master key every CDJ-3000X cabinet shares.  `model_env` is the U-Boot
/// `model` variable ([`cdj3k_emu_panel::ModelSpec::model_env`]); `images_tar_gz`
/// is the sibling of `cabinet.img` in the same `.UPD` package.
pub fn vendor_passphrase(model_env: &str, images_tar_gz: &[u8]) -> Vec<u8> {
    let inner = hex::encode(Sha512::digest(images_tar_gz));
    let outer = Sha512::digest(format!("{model_env}{inner}").as_bytes());
    hex::encode(outer).into_bytes()
}

/// Add a keyslot to `image` that opens with `unit_passphrase`, returning its
/// index.
///
/// The master key is recovered first by opening a factory slot with
/// `vendor_passphrase` ([`vendor_passphrase`]); a container no factory slot
/// opens is refused rather than corrupted.  Only the free keyslot's own header
/// entry and material region are written; the payload and the existing slots
/// are untouched.
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

    // Recover the master key from a factory slot.  This both yields the key to
    // wrap into the new slot and proves the container is a CDJ-3000X cabinet.
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
    Ok(index)
}

/// Key slot `index` of `image` for `passphrase`: fresh salt, PBKDF2 slot key,
/// AF-split and XTS-encrypted `master_key` into the region `layout` reserves,
/// and the slot's header entry marked active.
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

/// Recover the container's master key by opening an active keyslot with
/// `passphrase`: parse and validate the header, then [`unlock`].
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

/// The master key of the first active keyslot `passphrase` opens: PBKDF2 the
/// passphrase into the slot key, XTS-decrypt the slot's key material, AF-merge
/// it, and accept the result only if the header's mk-digest does.  `image` is
/// not modified.
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

/// XTS-encrypt (or decrypt) keyslot material in place, 512-byte sectors
/// numbered from 0 - cryptsetup encrypts keyslot material at sector 0 whatever
/// its offset.  The key's first half is the data key, the second the tweak
/// key: AES-128 halves for a 32-byte key, AES-256 halves for a 64-byte one.
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
    if material.len() % 16 != 0 {
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
mod tests {
    use super::*;

    /// The deck derivation: serial `…05` = 5, so the unit string repeats
    /// 6 times.
    #[test]
    fn the_passphrase_is_genkey_prs() {
        let pw = cabinet_passphrase("CDJ3000X", "0123456789abcd05").unwrap();
        assert_eq!(pw.len(), 128, "128 lowercase hex digits, no newline");

        let mut h = Sha512::new();
        for _ in 0..6 {
            h.update(b"CDJ3000X0123456789abcd05");
        }
        assert_eq!(pw, hex::encode(h.finalize()).into_bytes());

        assert!(matches!(
            cabinet_passphrase("CDJ3000X", "short"),
            Err(RekeyError::BadSerial)
        ));
        // A zero last byte would repeat the unit string once, not never.
        assert_eq!(
            cabinet_passphrase("CDJ3000X", "0123456789abcd00")
                .unwrap()
                .len(),
            128
        );
    }

    /// The factory derivation: `sha512hex(model + sha512hex(images.tar.gz))`,
    /// 128 hex digits with no newline.
    #[test]
    fn vendor_passphrase_is_genkey_pbs() {
        let tgz = b"pretend this is images.tar.gz";
        let pw = vendor_passphrase("CDJ3000X", tgz);
        assert_eq!(pw.len(), 128, "128 lowercase hex digits, no newline");

        let inner = hex::encode(Sha512::digest(tgz));
        let outer = hex::encode(Sha512::digest(format!("CDJ3000X{inner}").as_bytes()));
        assert_eq!(pw, outer.into_bytes());
    }

    /// A container no factory slot opens is refused untouched - a foreign
    /// cabinet, or the wrong firmware's `images.tar.gz`, must not be corrupted.
    #[test]
    fn a_container_no_factory_slot_opens_is_refused() {
        let (mut image, _) = synthetic_container(64, "sha256", b"vendor");
        let before = image.clone();

        assert!(matches!(
            add_keyslot(&mut image, b"not the vendor passphrase", b"unit"),
            Err(RekeyError::VendorSlotLocked)
        ));
        assert_eq!(image, before, "a refused container is left alone");
    }

    const TEST_ITERATIONS: u32 = 1000;
    const PAYLOAD_SECTORS: usize = 8;

    /// A cryptsetup-shaped LUKS1 container: all eight keyslots laid out and
    /// 4 KiB aligned, slot 0 keyed for `vendor`, then a small payload.
    /// Returns the image and its master key.
    fn synthetic_container(key_bytes: usize, hash: &str, vendor: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let km_sectors = (key_bytes * LUKS1_STRIPES as usize)
            .div_ceil(SECTOR)
            .next_multiple_of(8);
        let payload_sector = 8 + SLOT_COUNT * km_sectors;
        let mut image = vec![0u8; (payload_sector + PAYLOAD_SECTORS) * SECTOR];

        image[..6].copy_from_slice(super::super::LUKS_MAGIC);
        image[6..8].copy_from_slice(&1u16.to_be_bytes());
        image[8..11].copy_from_slice(b"aes");
        image[40..51].copy_from_slice(b"xts-plain64");
        image[72..72 + hash.len()].copy_from_slice(hash.as_bytes());
        image[104..108].copy_from_slice(&(payload_sector as u32).to_be_bytes());
        image[108..112].copy_from_slice(&(key_bytes as u32).to_be_bytes());
        image[132..164].copy_from_slice(&random_bytes(32));
        image[164..168].copy_from_slice(&TEST_ITERATIONS.to_be_bytes());
        image[168..204].copy_from_slice(b"00000000-0000-4000-8000-000000000000");
        for index in 0..SLOT_COUNT {
            let entry = SLOT_TABLE_OFFSET + index * SLOT_ENTRY_BYTES;
            let km_sector = 8 + index * km_sectors;
            image[entry..entry + 4].copy_from_slice(&SLOT_INACTIVE.to_be_bytes());
            image[entry + 40..entry + 44].copy_from_slice(&(km_sector as u32).to_be_bytes());
            image[entry + 44..entry + 48].copy_from_slice(&LUKS1_STRIPES.to_be_bytes());
        }
        let payload = payload_sector * SECTOR;
        image[payload..].fill(0x5a);

        let master_key = random_bytes(key_bytes);
        let mut digest = [0u8; 20];
        pbkdf2_derive(
            hash,
            &master_key,
            &image[132..164],
            TEST_ITERATIONS,
            &mut digest,
        )
        .unwrap();
        image[112..132].copy_from_slice(&digest);

        let layout = Luks1Layout::parse(&image).expect("the synthetic header parses");
        write_slot(&mut image, &layout, 0, &master_key, vendor, TEST_ITERATIONS).unwrap();
        (image, master_key)
    }

    /// Add a unit slot to a synthetic container of each supported key size
    /// and open it again; the vendor slot keeps opening and the payload is
    /// untouched.
    #[test]
    fn a_new_keyslot_round_trips_for_both_key_sizes() {
        for (key_bytes, hash) in [(32, "sha256"), (64, "sha256"), (32, "sha1"), (64, "sha1")] {
            let (mut image, master_key) = synthetic_container(key_bytes, hash, b"vendor");
            let payload = image.len() - PAYLOAD_SECTORS * SECTOR;
            let before_payload = image[payload..].to_vec();

            let unit = cabinet_passphrase("CDJ3000X", "0000000000000001").unwrap();
            let index = add_keyslot_with(&mut image, b"vendor", &unit, TEST_ITERATIONS)
                .expect("keyslot added");
            assert_eq!(index, 1, "slot 0 holds the vendor key");
            assert_eq!(
                &image[payload..],
                &before_payload[..],
                "the payload is untouched"
            );

            assert_eq!(open_master_key(&image, &unit, hash).unwrap(), master_key);
            assert_eq!(
                open_master_key(&image, b"vendor", hash).unwrap(),
                master_key
            );
            assert!(matches!(
                open_master_key(&image, b"neither", hash),
                Err(RekeyError::VendorSlotLocked)
            ));
        }
    }

    #[test]
    fn a_full_slot_table_is_refused() {
        let (mut image, _) = synthetic_container(32, "sha256", b"vendor");
        for _ in 1..SLOT_COUNT {
            add_keyslot_with(&mut image, b"vendor", b"unit", TEST_ITERATIONS).unwrap();
        }
        assert!(matches!(
            add_keyslot_with(&mut image, b"vendor", b"unit", TEST_ITERATIONS),
            Err(RekeyError::NoFreeSlot)
        ));
    }

    fn put32(image: &mut [u8], at: usize, value: u32) {
        image[at..at + 4].copy_from_slice(&value.to_be_bytes());
    }

    /// `mutate` applied to a valid container must make [`add_keyslot`] fail
    /// with an error `check` accepts, leaving the image untouched.
    #[track_caller]
    fn refused(mutate: impl Fn(&mut Vec<u8>), check: impl Fn(&RekeyError) -> bool) {
        let (mut image, _) = synthetic_container(64, "sha256", b"vendor");
        mutate(&mut image);
        let before = image.clone();
        let err = add_keyslot_with(&mut image, b"vendor", b"unit", TEST_ITERATIONS)
            .expect_err("a malformed container is refused");
        assert!(check(&err), "unexpected error {err:?}");
        assert_eq!(image, before, "a refused container is left alone");
    }

    #[test]
    fn malformed_headers_are_errors_not_panics() {
        use RekeyError::*;
        let slot0 = SLOT_TABLE_OFFSET;
        let slot1 = SLOT_TABLE_OFFSET + SLOT_ENTRY_BYTES;

        // Truncation, at every depth.
        refused(|i| i.truncate(0), |e| matches!(e, NotLuks1));
        refused(|i| i.truncate(HEADER_BYTES - 1), |e| matches!(e, NotLuks1));
        refused(|i| i.truncate(HEADER_BYTES), |e| matches!(e, Truncated));
        refused(|i| i.truncate(4096), |e| matches!(e, Truncated));
        refused(
            |i| i.truncate(i.len() - PAYLOAD_SECTORS * SECTOR - 1),
            |e| matches!(e, Truncated),
        );

        // Identity.
        refused(|i| i[0] = b'X', |e| matches!(e, NotLuks1));
        refused(
            |i| i[6..8].copy_from_slice(&2u16.to_be_bytes()),
            |e| matches!(e, NotLuks1),
        );
        refused(
            |i| i[8..40].copy_from_slice(&[0; 32]),
            |e| matches!(e, UnsupportedCipher(_)),
        );
        refused(
            |i| i[8..15].copy_from_slice(b"serpent"),
            |e| matches!(e, UnsupportedCipher(_)),
        );
        refused(
            |i| i[40..56].copy_from_slice(b"cbc-essiv:sha256"),
            |e| matches!(e, UnsupportedCipher(_)),
        );
        refused(
            |i| i[72..78].copy_from_slice(b"sha512"),
            |e| matches!(e, UnsupportedCipher(_)),
        );

        // Key size.
        for bad in [0u32, 16, 48, 65, 0x1000_0000, u32::MAX] {
            refused(move |i| put32(i, 108, bad), |e| matches!(e, Malformed(_)));
        }

        // Payload offset and mk-digest.
        refused(|i| put32(i, 104, 0), |e| matches!(e, Malformed(_)));
        refused(
            |i| put32(i, 104, u32::MAX),
            |e| matches!(e, Truncated | Malformed(_)),
        );
        refused(|i| put32(i, 164, 0), |e| matches!(e, Malformed(_)));

        // Keyslot entries: the active vendor slot and an inactive one.
        for entry in [slot0, slot1] {
            refused(move |i| put32(i, entry, 0), |e| matches!(e, Malformed(_)));
            refused(
                move |i| put32(i, entry + 44, 0),
                |e| matches!(e, Malformed(_)),
            );
            refused(
                move |i| put32(i, entry + 44, 4),
                |e| matches!(e, Malformed(_)),
            );
            refused(
                move |i| put32(i, entry + 44, u32::MAX),
                |e| matches!(e, Malformed(_)),
            );
            refused(
                move |i| put32(i, entry + 40, 0),
                |e| matches!(e, Malformed(_)),
            );
            refused(
                move |i| put32(i, entry + 40, u32::MAX),
                |e| matches!(e, Truncated | Malformed(_)),
            );
        }
        refused(|i| put32(i, slot0 + 4, 0), |e| matches!(e, Malformed(_)));
        // Slot 1 moved onto slot 0's material, and slot 7 four sectors into the
        // payload.
        refused(|i| put32(i, slot1 + 40, 8), |e| matches!(e, Malformed(_)));
        refused(
            |i| {
                let payload = be32(i, 104);
                put32(
                    i,
                    SLOT_TABLE_OFFSET + 7 * SLOT_ENTRY_BYTES + 40,
                    payload - 496,
                );
            },
            |e| matches!(e, Malformed(_)),
        );
    }

    #[test]
    fn xts_material_refuses_other_key_sizes() {
        let mut material = vec![0u8; 512];
        for len in [0, 16, 48, 128] {
            assert!(matches!(
                xts_material(&mut material, &vec![0u8; len], true),
                Err(RekeyError::Malformed(_))
            ));
        }
    }
}
