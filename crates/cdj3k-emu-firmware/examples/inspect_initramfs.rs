//! Parse a real `initramfs.cpio.gz` and report what is in it.
//!
//! Usage: cargo run -p cdj3k-emu-firmware --example inspect_initramfs -- <path>

use std::collections::BTreeMap;

use std::path::Path;

use cdj3k_emu_firmware::cpio::{self, Entry, Kind};
use flate2::read::GzDecoder;

fn main() {
    let path = std::env::args().nth(1).expect("path to initramfs.cpio.gz");
    let gz = std::fs::read(&path).expect("read");
    eprintln!("{path}: {} bytes compressed", gz.len());

    // A real initramfs is a single gzip member. MultiGzDecoder would read
    // trailing padding as a broken second header, so this uses GzDecoder.
    let mut raw = Vec::new();
    std::io::Read::read_to_end(&mut GzDecoder::new(&gz[..]), &mut raw).expect("gunzip");
    eprintln!("{} bytes of cpio", raw.len());

    let entries = match cpio::read(&raw) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Parsing the cpio failed: {e}");
            std::process::exit(1);
        }
    };

    summarize(&entries);
    let argv: Vec<String> = std::env::args().skip(2).collect();
    run_flags(&entries, &argv);
    lookup(&entries, &argv);
    round_trip(&entries);
}

/// Prints the entry counts by kind, mode and owner, and the largest file.
fn summarize(entries: &[Entry]) {
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    let mut modes: BTreeMap<u32, usize> = BTreeMap::new();
    let mut owners: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    let mut biggest = (0usize, String::new());
    for e in entries {
        let k = match &e.kind {
            Kind::File(d) => {
                if d.len() > biggest.0 {
                    biggest = (d.len(), e.path.display().to_string());
                }
                "file"
            }
            Kind::Dir => "dir",
            Kind::Symlink(_) => "symlink",
            Kind::CharDev { .. } => "chardev",
        };
        *kinds.entry(k).or_default() += 1;
        *modes.entry(e.mode).or_default() += 1;
        *owners.entry((e.uid, e.gid)).or_default() += 1;
    }

    println!("entries: {}", entries.len());
    println!("kinds:   {kinds:?}");
    println!("owners (uid,gid): {owners:?}");
    let mut top: Vec<_> = modes.into_iter().collect();
    top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    println!(
        "top modes: {:?}",
        top.iter()
            .take(6)
            .map(|(m, n)| format!("{m:04o}x{n}"))
            .collect::<Vec<_>>()
    );
    println!("largest file: {} ({} bytes)", biggest.1, biggest.0);
}

/// `--ls <prefix>` lists a directory; `--cat <path>` prints a file.
fn run_flags(entries: &[Entry], argv: &[String]) {
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--ls" => {
                ls(entries, &argv[i + 1]);
                i += 2;
            }
            "--cat" => {
                cat(entries, &argv[i + 1]);
                i += 2;
            }
            _ => i += 1,
        }
    }
}

fn ls(entries: &[Entry], prefix: &str) {
    println!("--- ls {prefix} ---");
    for e in entries
        .iter()
        .filter(|e| e.path.to_string_lossy().starts_with(prefix))
        .take(40)
    {
        let what = match &e.kind {
            Kind::File(d) => format!("file {}", d.len()),
            Kind::Dir => "dir".into(),
            Kind::Symlink(t) => format!("-> {t}"),
            Kind::CharDev { major, minor } => format!("chardev {major}:{minor}"),
        };
        println!("  {:04o} {} [{what}]", e.mode, e.path.display());
    }
}

fn cat(entries: &[Entry], want: &str) {
    if let Some(Kind::File(d)) = entries
        .iter()
        .find(|e| e.path == Path::new(want))
        .map(|e| &e.kind)
    {
        println!("--- cat {want} ---\n{}", String::from_utf8_lossy(d));
    }
}

/// Any extra arguments that are absolute paths are looked up.
fn lookup(entries: &[Entry], argv: &[String]) {
    for want in argv.iter().filter(|a| a.starts_with('/')) {
        match entries.iter().find(|e| e.path == Path::new(want)) {
            Some(e) => println!(
                "lookup {want}: {:?} mode {:04o} uid {} gid {}",
                match &e.kind {
                    Kind::File(d) => format!("file {} bytes", d.len()),
                    Kind::Dir => "dir".into(),
                    Kind::Symlink(t) => format!("symlink -> {t}"),
                    Kind::CharDev { major, minor } => format!("chardev {major}:{minor}"),
                },
                e.mode,
                e.uid,
                e.gid
            ),
            None => println!("lookup {want}: absent"),
        }
    }
}

/// Re-encodes and re-parses the entries to check that the codec is lossless
/// on a real archive.
fn round_trip(entries: &[Entry]) {
    let re = cpio::write(entries);
    match cpio::read(&re) {
        Ok(again) if again == entries => {
            println!("round trip: OK ({} bytes re-encoded)", re.len())
        }
        Ok(again) => {
            println!(
                "round trip: lossy, with {} entries after re-parsing and {} before",
                again.len(),
                entries.len()
            );
            if let Some((a, _)) = entries.iter().zip(&again).find(|(a, b)| a != b) {
                println!("  first difference at {}", a.path.display());
            }
        }
        Err(e) => println!("round trip: re-parsing failed: {e}"),
    }
}
