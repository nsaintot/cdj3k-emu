//! Turn the tar the guest handed back into the initramfs the emulator boots.
//!
//! The guest packs as tar because busybox's cpio is extract-only. tar carries
//! everything a cpio header needs — mode, uid/gid, symlink targets, device
//! nodes — so nothing is lost in the crossing.
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

    let mut archive = tar::Archive::new(std::fs::File::open(tar_path).expect("open tar"));
    let mut entries: Vec<Entry> = Vec::new();
    let mut skipped = 0usize;
    // Content of every regular file seen so far, so a hard link becomes a
    // full copy: this codec has no ino/nlink model to express one.
    let mut seen: BTreeMap<String, Vec<u8>> = BTreeMap::new();

    for e in archive.entries().expect("tar entries") {
        let mut e = e.expect("tar entry");
        let header = e.header().clone();
        let path = format!("/{}", e.path().expect("path").to_string_lossy());
        let path = path.replace("/./", "/");
        let mode = header.mode().unwrap_or(0o644) & 0o7777;

        let kind = match header.entry_type() {
            EntryType::Regular | EntryType::Continuous => {
                let mut data = Vec::new();
                e.read_to_end(&mut data).expect("read");
                seen.insert(path.clone(), data.clone());
                Kind::File(data)
            }
            EntryType::Link => {
                let target = header
                    .link_name()
                    .ok()
                    .flatten()
                    .map(|p| format!("/{}", p.to_string_lossy()).replace("/./", "/"))
                    .unwrap_or_default();
                match seen.get(&target) {
                    Some(data) => Kind::File(data.clone()),
                    None => {
                        eprintln!("  hard link {path} -> {target}: target not seen, skipping");
                        skipped += 1;
                        continue;
                    }
                }
            }
            EntryType::Directory => Kind::Dir,
            EntryType::Symlink => Kind::Symlink(
                header
                    .link_name()
                    .ok()
                    .flatten()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
            EntryType::Char => Kind::CharDev {
                major: header.device_major().ok().flatten().unwrap_or(0),
                minor: header.device_minor().ok().flatten().unwrap_or(0),
            },
            _ => {
                skipped += 1;
                continue;
            }
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
