use std::io::Cursor;

use super::rekey::tests::synthetic_container;
use super::*;

/// `OffsetReader` hides `base` from the parser: an absolute seek lands
/// `base` bytes further into the file and reports the shifted position.
#[test]
fn offset_reader_shifts_absolute_seeks() {
    let data: Vec<u8> = (0..=255u8).collect();
    let mut r = OffsetReader {
        inner: std::io::Cursor::new(data),
        base: 16,
    };

    assert_eq!(r.seek(SeekFrom::Start(0)).unwrap(), 0);
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf).unwrap();
    assert_eq!(buf, [16, 17, 18, 19], "offset 0 reads the byte at base");

    assert_eq!(r.seek(SeekFrom::Start(32)).unwrap(), 32);
    r.read_exact(&mut buf).unwrap();
    assert_eq!(buf, [48, 49, 50, 51]);
}

/// An aes-xts-plain64 / sha256 container with slot 0 keyed for `vendor`,
/// its parsed header and its master key.
fn container() -> (Vec<u8>, Luks1Header, Vec<u8>) {
    let (image, mk) = synthetic_container(64, "sha256", b"vendor");
    let header = read_luks1_header(&mut Cursor::new(&image)).unwrap();
    (image, header, mk)
}

fn open(image: &[u8], h: &Luks1Header, keyfile: &[u8]) -> Result<Vec<u8>, LuksError> {
    try_key_slots(&mut Cursor::new(image), h, keyfile)
}

#[test]
fn the_keyfile_opens_its_slot() {
    let (image, h, mk) = container();
    assert_eq!(open(&image, &h, b"vendor").unwrap(), mk);
}

#[test]
fn a_base64_keyfile_opens_as_its_decoding() {
    let (image, h, mk) = container();
    assert_eq!(open(&image, &h, b"dmVuZG9y\n").unwrap(), mk);
}

/// Key material encrypted from its own sector number, with the key halves
/// swapped, still opens.
#[test]
fn swapped_halves_from_the_absolute_sector_open() {
    let (mut image, h, mk) = container();
    let slot = &h.key_slots[0];
    let mut sk = [0u8; 64];
    pbkdf2_derive("sha256", b"vendor", &slot.salt, slot.iterations, &mut sk).unwrap();
    let cipher = |k: &[u8]| Aes256::new_from_slice(k).unwrap();
    let standard = Xts128::new(cipher(&sk[..32]), cipher(&sk[32..]));
    let swapped = Xts128::new(cipher(&sk[32..]), cipher(&sk[..32]));
    let km_sector = slot.key_material_offset as u64;
    let km_at = km_sector as usize * 512;
    let km = &mut image[km_at..km_at + 64 * slot.stripes as usize];
    for (idx, chunk) in km.chunks_mut(512).enumerate() {
        standard.decrypt_sector(chunk, xts_tweak(idx as u64, "xts-plain64"));
        swapped.encrypt_sector(chunk, xts_tweak(km_sector + idx as u64, "xts-plain64"));
    }
    assert_eq!(open(&image, &h, b"vendor").unwrap(), mk);
}

#[test]
fn a_wrong_key_reports_every_slot_it_tried() {
    let (image, h, _) = container();
    let slot = &h.key_slots[0];
    let header = |keyfile_len: usize| {
        format!(
            "cipher=aes-xts-plain64  hash=sha256  key_bytes=64  payload_sector={}  \
             active_slots=1  key_input_bytes={keyfile_len}\n\
             slot 0: iters=1000  km_sector={}  stripes=4000  km_bytes=256000\n\
             slot 0: keyfile_len={keyfile_len}\n",
            h.payload_offset, slot.key_material_offset
        )
    };
    let mismatch = format!(
        "slot 0: the digest does not match\n  expected: {:02x?}\n  computed: (no candidate matched)\n",
        h.mk_digest
    );

    let Err(LuksError::NoValidKeySlot(diag)) = open(&image, &h, b"\xffwrong") else {
        panic!("a wrong key opened the container");
    };
    assert_eq!(diag, format!("{}{mismatch}", header(6)));

    let Err(LuksError::NoValidKeySlot(diag)) = open(&image, &h, b"AAAA") else {
        panic!("a wrong key opened the container");
    };
    assert_eq!(
        diag,
        format!(
            "{}slot 0: also trying base64-decoded keyfile (3 bytes)\n{mismatch}",
            header(4)
        )
    );

    let km_at = slot.key_material_offset as usize * 512;
    let Err(LuksError::NoValidKeySlot(diag)) = open(&image[..km_at + 10], &h, b"vendor") else {
        panic!("a truncated container opened");
    };
    assert_eq!(
        diag,
        format!(
            "{}slot 0: read 256000 bytes failed: failed to fill whole buffer\n",
            header(6)
        )
    );
}
