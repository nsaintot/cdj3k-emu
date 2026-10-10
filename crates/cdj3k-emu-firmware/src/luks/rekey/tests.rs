use super::*;

/// Checks the deck derivation: a serial ending in `05` gives N = 5, so the
/// unit string repeats 6 times.
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
    // A last byte of zero repeats the unit string once.
    assert_eq!(
        cabinet_passphrase("CDJ3000X", "0123456789abcd00")
            .unwrap()
            .len(),
        128
    );
}

/// Checks the factory derivation, `sha512hex(model + sha512hex(images.tar.gz))`,
/// which gives 128 hex digits with no newline.
#[test]
fn vendor_passphrase_is_genkey_pbs() {
    let tgz = b"pretend this is images.tar.gz";
    let pw = vendor_passphrase("CDJ3000X", tgz);
    assert_eq!(pw.len(), 128, "128 lowercase hex digits, no newline");

    let inner = hex::encode(Sha512::digest(tgz));
    let outer = hex::encode(Sha512::digest(format!("CDJ3000X{inner}").as_bytes()));
    assert_eq!(pw, outer.into_bytes());
}

/// A container that no factory slot opens, such as a foreign cabinet or one
/// paired with another firmware's `images.tar.gz`, is refused and left unchanged.
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
pub(in crate::luks) fn synthetic_container(
    key_bytes: usize,
    hash: &str,
    vendor: &[u8],
) -> (Vec<u8>, Vec<u8>) {
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

/// Adds a unit slot to a synthetic container of each supported key size and
/// opens it again. The vendor slot still opens and the payload is unchanged.
#[test]
fn a_new_keyslot_round_trips_for_both_key_sizes() {
    for (key_bytes, hash) in [(32, "sha256"), (64, "sha256"), (32, "sha1"), (64, "sha1")] {
        let (mut image, master_key) = synthetic_container(key_bytes, hash, b"vendor");
        let payload = image.len() - PAYLOAD_SECTORS * SECTOR;
        let before_payload = image[payload..].to_vec();

        let unit = cabinet_passphrase("CDJ3000X", "0000000000000001").unwrap();
        let index =
            add_keyslot_with(&mut image, b"vendor", &unit, TEST_ITERATIONS).expect("keyslot added");
        assert_eq!(index, 0, "the unit key is tried first");
        let layout = Luks1Layout::parse(&image).unwrap();
        assert_eq!(layout.slots[0].iterations, TEST_ITERATIONS);
        assert!(layout.slots[1].active, "the vendor slot moves to slot 1");
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

/// Applies `mutate` to a valid container and checks that [`add_keyslot`]
/// fails with an error `check` accepts and leaves the image unchanged.
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

const SLOT0: usize = SLOT_TABLE_OFFSET;
const SLOT1: usize = SLOT_TABLE_OFFSET + SLOT_ENTRY_BYTES;

#[test]
fn truncation_at_every_depth_is_refused() {
    use RekeyError::*;
    refused(|i| i.clear(), |e| matches!(e, NotLuks1));
    refused(|i| i.truncate(HEADER_BYTES - 1), |e| matches!(e, NotLuks1));
    refused(|i| i.truncate(HEADER_BYTES), |e| matches!(e, Truncated));
    refused(|i| i.truncate(4096), |e| matches!(e, Truncated));
    refused(
        |i| i.truncate(i.len() - PAYLOAD_SECTORS * SECTOR - 1),
        |e| matches!(e, Truncated),
    );
}

#[test]
fn a_foreign_identity_is_refused() {
    use RekeyError::*;
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
}

#[test]
fn malformed_key_size_payload_and_digest_are_refused() {
    use RekeyError::*;
    for bad in [0u32, 16, 48, 65, 0x1000_0000, u32::MAX] {
        refused(move |i| put32(i, 108, bad), |e| matches!(e, Malformed(_)));
    }
    refused(|i| put32(i, 104, 0), |e| matches!(e, Malformed(_)));
    refused(
        |i| put32(i, 104, u32::MAX),
        |e| matches!(e, Truncated | Malformed(_)),
    );
    refused(|i| put32(i, 164, 0), |e| matches!(e, Malformed(_)));
}

/// Checks both the active vendor slot and an inactive slot.
#[test]
fn malformed_keyslot_entries_are_refused() {
    use RekeyError::*;
    for entry in [SLOT0, SLOT1] {
        refused(move |i| put32(i, entry, 0), |e| matches!(e, Malformed(_)));
        for stripes in [0, 4, u32::MAX] {
            refused(
                move |i| put32(i, entry + 44, stripes),
                |e| matches!(e, Malformed(_)),
            );
        }
        refused(
            move |i| put32(i, entry + 40, 0),
            |e| matches!(e, Malformed(_)),
        );
        refused(
            move |i| put32(i, entry + 40, u32::MAX),
            |e| matches!(e, Truncated | Malformed(_)),
        );
    }
    refused(|i| put32(i, SLOT0 + 4, 0), |e| matches!(e, Malformed(_)));
}

/// Moves slot 1 onto slot 0's material, and moves slot 7 so that its material
/// runs four sectors into the payload.
#[test]
fn overlapping_key_material_is_refused() {
    use RekeyError::*;
    refused(|i| put32(i, SLOT1 + 40, 8), |e| matches!(e, Malformed(_)));
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
