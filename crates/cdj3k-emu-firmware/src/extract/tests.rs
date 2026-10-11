use super::*;
use std::cell::RefCell;
use std::io::Cursor;

const S: usize = ISO_SECTOR as usize;

/// One ISO 9660 directory record.
fn record(name: &[u8], lba: u32, size: u32, dir: bool) -> Vec<u8> {
    let mut r = vec![0u8; 33 + name.len()];
    r[0] = r.len() as u8;
    r[2..6].copy_from_slice(&lba.to_le_bytes());
    r[10..14].copy_from_slice(&size.to_le_bytes());
    r[25] = if dir { 0x02 } else { 0 };
    r[32] = name.len() as u8;
    r[33..].copy_from_slice(name);
    r
}

/// An ISO whose `IMAGES/APP.REV` contains `14926`: the PVD at sector 16, the
/// root directory at 17, `IMAGES` at 18 and the file at 19.  `extra_root`
/// is appended to the root directory's records.
fn iso(app_rev_size: u32, extra_root: &[u8]) -> Vec<u8> {
    let mut img = vec![0u8; 20 * S];
    let pvd = &mut img[16 * S..17 * S];
    pvd[0] = 1;
    pvd[1..6].copy_from_slice(b"CD001");
    pvd[158..162].copy_from_slice(&17u32.to_le_bytes());
    pvd[166..170].copy_from_slice(&(S as u32).to_le_bytes());
    let root = [
        record(b"\0", 17, S as u32, true),
        record(b"\x01", 17, S as u32, true),
        record(b"IMAGES", 18, S as u32, true),
        extra_root.to_vec(),
    ]
    .concat();
    img[17 * S..17 * S + root.len()].copy_from_slice(&root);
    let images = [
        record(b"\0", 18, S as u32, true),
        record(b"\x01", 17, S as u32, true),
        record(b"APP.REV;1", 19, app_rev_size, false),
    ]
    .concat();
    img[18 * S..18 * S + images.len()].copy_from_slice(&images);
    img[19 * S..19 * S + 5].copy_from_slice(b"14926");
    img
}

fn lookup(image: Vec<u8>, name: &str) -> Result<Vec<u8>, ExtractError> {
    read_iso_file(&mut Cursor::new(image), name)
}

#[test]
fn a_nested_file_is_found_case_insensitively() {
    assert_eq!(lookup(iso(5, &[]), "IMAGES/APP.REV").unwrap(), b"14926");
    assert_eq!(lookup(iso(5, &[]), "images/app.rev;1").unwrap(), b"14926");
}

#[test]
fn a_missing_file_lists_the_directory_it_searched() {
    match lookup(iso(5, &[]), "IMAGES/NOPE") {
        Err(ExtractError::FileNotFound(s)) => {
            assert_eq!(
                s,
                r#""NOPE" is not in the directory, which contains ["[F]APP.REV;1"]"#
            )
        }
        other => panic!("unexpected {other:?}"),
    }
    match lookup(iso(5, &[]), "BOOT") {
        Err(ExtractError::FileNotFound(s)) => {
            assert_eq!(
                s,
                r#""BOOT" is not in the directory, which contains ["[D]IMAGES"]"#
            )
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn the_root_listing_marks_directories() {
    let entries = list_iso_root(&mut Cursor::new(iso(5, &[]))).unwrap();
    assert_eq!(entries, ["IMAGES/"]);
}

/// The listing checks the PVD signature and the root extent's bounds before it
/// allocates anything.
#[test]
fn a_malformed_root_is_not_listed() {
    assert!(matches!(
        list_iso_root(&mut Cursor::new(vec![0u8; 20 * S])),
        Err(ExtractError::BadIso(
            "not an ISO 9660 primary volume descriptor"
        ))
    ));
    let mut huge_root = iso(5, &[]);
    huge_root[16 * S + 166..16 * S + 170].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        list_iso_root(&mut Cursor::new(huge_root)),
        Err(ExtractError::BadIso(
            "extent runs past the end of the image"
        ))
    ));
}

#[test]
fn malformed_images_are_errors_not_panics() {
    assert!(matches!(
        lookup(vec![0u8; 20 * S], "IMAGES/APP.REV"),
        Err(ExtractError::BadIso(
            "not an ISO 9660 primary volume descriptor"
        ))
    ));
    assert!(matches!(
        lookup(iso(u32::MAX, &[]), "IMAGES/APP.REV"),
        Err(ExtractError::BadIso(
            "extent runs past the end of the image"
        ))
    ));
    assert!(matches!(
        lookup(vec![0u8; 16 * S], "IMAGES/APP.REV"),
        Err(ExtractError::Io(_))
    ));
}

/// A record shorter than the 33-byte minimum ends the directory, for the
/// search and for the listing its error carries.
#[test]
fn a_short_record_ends_the_directory() {
    let mut short = vec![0u8; 20];
    short[0] = 20;
    match lookup(iso(5, &short), "BOOT") {
        Err(ExtractError::FileNotFound(s)) => {
            assert_eq!(
                s,
                r#""BOOT" is not in the directory, which contains ["[D]IMAGES"]"#
            )
        }
        other => panic!("unexpected {other:?}"),
    }
}

/// A gzipped tar of `files`.
fn tar_gz(files: &[(&str, &[u8])]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut tar = tar::Builder::new(gz);
    for (path, data) in files {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        tar.append_data(&mut h, path, *data).unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap()
}

/// Runs [`extract_image_from_tar_gz`] on `files` and returns the kernel it
/// wrote and the lines it logged.
fn extract(files: &[(&str, &[u8])]) -> (Result<Vec<u8>, ExtractError>, Vec<String>) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let out = std::env::temp_dir().join(format!("cdj3k-extract-test-{}-{n}", std::process::id()));
    let log = RefCell::new(Vec::new());
    let result = extract_image_from_tar_gz(&tar_gz(files), &out, &|l: &str| {
        log.borrow_mut().push(l.to_string())
    })
    .map(|()| std::fs::read(&out).unwrap());
    let _ = std::fs::remove_file(&out);
    (result, log.into_inner())
}

#[test]
fn the_rk3399_kernel_is_chosen_by_path_then_by_content() {
    let (got, log) = extract(&[("g2m/Image", b"rcar-du"), ("rk3399/Image", b"k")]);
    assert_eq!(got.unwrap(), b"k");
    assert_eq!(
        log.last().unwrap(),
        r#"[kernel] Selected "rk3399/Image" because its path contains 'rk3399'."#
    );

    let (got, log) = extract(&[("a/Image", b"plain"), ("b/IMAGE", b"..rk3399..")]);
    assert_eq!(got.unwrap(), b"..rk3399..");
    assert_eq!(
        log[0],
        r#"[kernel] Found the Image candidate "a/Image" (0 MB)."#
    );
    assert_eq!(
        log.last().unwrap(),
        r#"[kernel] Selected "b/IMAGE" because its content contains 'rk3399'."#
    );
}

#[test]
fn a_tar_without_an_rk3399_kernel_is_refused() {
    let (got, _) = extract(&[("boot/zImage", b"rk3399")]);
    assert!(matches!(
        got,
        Err(ExtractError::FileNotFound(s)) if s == "Image not found in images.tar.gz"
    ));

    let (got, _) = extract(&[("salvator/Image", b"x")]);
    assert!(matches!(
        got,
        Err(ExtractError::UnsupportedG2M(d))
            if d == r#"the tar has R-Car markers such as rcar, r8a7796 or salvator; the candidates are ["salvator/Image"]"#
    ));

    let (got, _) = extract(&[("a/Image", b"x"), ("b/Image", b"y")]);
    assert!(matches!(
        got,
        Err(ExtractError::UnsupportedG2M(d))
            if d == r#"no candidate has an rk3399 marker; the candidates are ["a/Image", "b/Image"]"#
    ));
}
