//! Turn the tar the guest handed back into the initramfs the emulator boots.
//!
//! The guest packs a tar because busybox's cpio can only extract. tar carries
//! everything a cpio header needs (the mode, uid and gid, symlink targets and
//! device nodes), so the conversion loses nothing.
//!
//! Usage: <handback.tar> <out.cpio.gz>

use std::collections::BTreeMap;
use std::io::{Read, Write};

use cdj3k_emu_firmware::cpio::{self, Entry, Kind};
use flate2::write::GzEncoder;
use flate2::Compression;
use tar::EntryType;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let [tar_path, out_path] = a.as_slice() else {
        eprintln!("usage: <handback.tar> <out.cpio.gz>");
        std::process::exit(2);
    };

    let mut archive =
        tar::Archive::new(std::fs::File::open(tar_path).expect("the tar file should open"));
    let mut entries: Vec<Entry> = Vec::new();
    let mut skipped = 0usize;
    // The content of every regular file seen so far. A hard link becomes a
    // full copy, because the cpio codec has no inode or link count.
    let mut seen: BTreeMap<String, Vec<u8>> = BTreeMap::new();

    for e in archive.entries().expect("the tar entries should be listed") {
        let mut e = e.expect("the tar entry should be readable");
        let header = e.header().clone();
        let path = format!("/{}", e.path().expect("path").to_string_lossy());
        let path = path.replace("/./", "/");
        let mode = header.mode().unwrap_or(0o644) & 0o7777;

        let Some(kind) = entry_kind(&mut e, &header, &path, &mut seen) else {
            skipped += 1;
            continue;
        };
        entries.push(Entry {
            path: path.into(),
            kind,
            mode,
            uid: header.uid().unwrap_or(0) as u32,
            gid: header.gid().unwrap_or(0) as u32,
            mtime: 0,
        });
    }

    eprintln!("{} entries, {skipped} skipped", entries.len());
    let raw = cpio::write(&entries);
    let mut enc = GzEncoder::new(
        std::fs::File::create(out_path).expect("create"),
        Compression::default(),
    );
    enc.write_all(&raw).expect("gzip");
    enc.finish().expect("finish");
    println!("wrote {out_path}");
}

/// Returns the cpio kind for a tar entry, or `None` when the entry is skipped.
fn entry_kind<R: Read>(
    e: &mut tar::Entry<R>,
    header: &tar::Header,
    path: &str,
    seen: &mut BTreeMap<String, Vec<u8>>,
) -> Option<Kind> {
    let link_name = || {
        header
            .link_name()
            .ok()
            .flatten()
            .map(|p| p.to_string_lossy().into_owned())
    };
    match header.entry_type() {
        EntryType::Regular | EntryType::Continuous => {
            let mut data = Vec::new();
            e.read_to_end(&mut data).expect("read");
            seen.insert(path.to_string(), data.clone());
            Some(Kind::File(data))
        }
        EntryType::Link => {
            let target = link_name()
                .map(|p| format!("/{p}").replace("/./", "/"))
                .unwrap_or_default();
            let data = seen.get(&target);
            if data.is_none() {
                eprintln!(
                    "  Skipping the hard link {path}: its target {target} has not been seen."
                );
            }
            data.map(|d| Kind::File(d.clone()))
        }
        EntryType::Directory => Some(Kind::Dir),
        EntryType::Symlink => Some(Kind::Symlink(link_name().unwrap_or_default())),
        EntryType::Char => Some(Kind::CharDev {
            major: header.device_major().ok().flatten().unwrap_or(0),
            minor: header.device_minor().ok().flatten().unwrap_or(0),
        }),
        _ => None,
    }
}
