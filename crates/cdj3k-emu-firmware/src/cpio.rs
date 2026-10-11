//! `newc` cpio, read and written in Rust.
//!
//! An initramfs is a cpio archive, and the guest's file modes, ownership and
//! symlinks are stored in the archive itself. Handling the archive as bytes
//! keeps them on hosts that cannot represent them: NTFS has no mode bits and
//! restricts symlink creation, and unpacking and repacking on macOS stamps
//! every entry with the host UID.
//!
//! Only `newc` (magic `070701`) is produced. `crc` (`070702`) is accepted on
//! read, and its checksum field ignored, which is what the kernel does when
//! the archive is unpacked.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

const HEADER_LEN: usize = 110;
const MAGIC_NEWC: &[u8] = b"070701";
const MAGIC_CRC: &[u8] = b"070702";
const TRAILER: &str = "TRAILER!!!";

// S_IF* — the file-type bits of `st_mode`.
const S_IFMT: u32 = 0o170000;
const S_IFREG: u32 = 0o100000;
const S_IFDIR: u32 = 0o040000;
const S_IFLNK: u32 = 0o120000;
const S_IFCHR: u32 = 0o020000;

#[derive(Debug)]
pub enum CpioError {
    /// The archive ended inside a record.
    Truncated {
        at: usize,
    },
    BadMagic {
        at: usize,
    },
    BadField {
        at: usize,
        field: &'static str,
    },
    /// A file type this codec does not model (block device, FIFO, socket).
    UnsupportedMode {
        path: String,
        mode: u32,
    },
    MissingTrailer,
}

impl fmt::Display for CpioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { at } => write!(f, "the cpio archive ends early at offset {at}"),
            Self::BadMagic { at } => write!(f, "bad cpio magic at offset {at}"),
            Self::BadField { at, field } => {
                write!(f, "malformed cpio field {field} at offset {at}")
            }
            Self::UnsupportedMode { path, mode } => {
                write!(f, "unsupported file type for {path}: mode {mode:o}")
            }
            Self::MissingTrailer => write!(f, "cpio has no TRAILER!!! record"),
        }
    }
}

impl std::error::Error for CpioError {}

/// The type of an entry and the data that type carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    File(Vec<u8>),
    Dir,
    /// The target is stored as the record's payload.
    Symlink(String),
    CharDev {
        major: u32,
        minor: u32,
    },
}

impl Kind {
    fn type_bits(&self) -> u32 {
        match self {
            Self::File(_) => S_IFREG,
            Self::Dir => S_IFDIR,
            Self::Symlink(_) => S_IFLNK,
            Self::CharDev { .. } => S_IFCHR,
        }
    }
}

/// One archive member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Absolute guest path, e.g. `/etc/shadow`. Stored relative in the archive.
    pub path: PathBuf,
    pub kind: Kind,
    /// Permission bits only; the file-type bits come from `kind`.
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub mtime: u32,
}

impl Entry {
    pub fn file(path: impl Into<PathBuf>, data: Vec<u8>, mode: u32) -> Self {
        Self::new(path, Kind::File(data), mode)
    }

    pub fn dir(path: impl Into<PathBuf>, mode: u32) -> Self {
        Self::new(path, Kind::Dir, mode)
    }

    pub fn symlink(path: impl Into<PathBuf>, target: impl Into<String>) -> Self {
        Self::new(path, Kind::Symlink(target.into()), 0o777)
    }

    fn new(path: impl Into<PathBuf>, kind: Kind, mode: u32) -> Self {
        Self {
            path: path.into(),
            kind,
            mode: mode & 0o7777,
            uid: 0,
            gid: 0,
            // A fixed mtime keeps the archive byte-reproducible, so an overlay
            // can be cached on its content hash.
            mtime: 0,
        }
    }
}

// ── Reading ───────────────────────────────────────────────────────────────────

/// Parses an archive. Parsing stops at the first `TRAILER!!!`, so for
/// concatenated archives it returns only the entries of the first one.
pub fn read(bytes: &[u8]) -> Result<Vec<Entry>, CpioError> {
    let mut entries = Vec::new();
    let mut at = 0usize;

    loop {
        // Concatenated archives are padded to 4 bytes; skip any NUL run.
        while at < bytes.len() && bytes[at] == 0 {
            at += 1;
        }
        if at >= bytes.len() {
            return Err(CpioError::MissingTrailer);
        }
        let record = Record::parse(bytes, at)?;
        if record.name == TRAILER {
            return Ok(entries);
        }
        at = record.end;
        entries.push(record.into_entry()?);
    }
}

/// One record as it sits in the archive, bounds-checked.
struct Record<'a> {
    raw_mode: u32,
    uid: u32,
    gid: u32,
    mtime: u32,
    rdevmajor: u32,
    rdevminor: u32,
    name: String,
    data: &'a [u8],
    /// Offset of the next record, before NUL padding.
    end: usize,
}

impl<'a> Record<'a> {
    fn parse(bytes: &'a [u8], at: usize) -> Result<Self, CpioError> {
        if bytes.len() - at < HEADER_LEN {
            return Err(CpioError::Truncated { at });
        }
        let magic = &bytes[at..at + 6];
        if magic != MAGIC_NEWC && magic != MAGIC_CRC {
            return Err(CpioError::BadMagic { at });
        }

        // Field `i` of the thirteen 8-hex-digit fields after the magic.
        let f = |i: usize, name: &'static str| -> Result<u32, CpioError> {
            let off = at + 6 + i * 8;
            let s = std::str::from_utf8(&bytes[off..off + 8])
                .map_err(|_| CpioError::BadField { at, field: name })?;
            u32::from_str_radix(s.trim(), 16).map_err(|_| CpioError::BadField { at, field: name })
        };

        let raw_mode = f(1, "mode")?;
        let uid = f(2, "uid")?;
        let gid = f(3, "gid")?;
        let mtime = f(5, "mtime")?;
        let filesize = f(6, "filesize")? as usize;
        let rdevmajor = f(9, "rdevmajor")?;
        let rdevminor = f(10, "rdevminor")?;
        let namesize = f(11, "namesize")? as usize;

        // namesize includes the trailing NUL, so it is at least 1.
        let bad_namesize = CpioError::BadField {
            at,
            field: "namesize",
        };
        if namesize == 0 {
            return Err(bad_namesize);
        }
        let name_at = at + HEADER_LEN;
        let name_end = name_at.checked_add(namesize).ok_or(bad_namesize)?;
        if name_end > bytes.len() {
            return Err(CpioError::Truncated { at });
        }
        let name = String::from_utf8_lossy(&bytes[name_at..name_end - 1]).into_owned();

        let data_at = pad4(name_end - at) + at;
        let data_end = data_at.checked_add(filesize).ok_or(CpioError::BadField {
            at,
            field: "filesize",
        })?;
        if data_end > bytes.len() {
            return Err(CpioError::Truncated { at });
        }

        Ok(Record {
            raw_mode,
            uid,
            gid,
            mtime,
            rdevmajor,
            rdevminor,
            name,
            data: &bytes[data_at..data_end],
            end: pad4(data_end - at) + at,
        })
    }

    fn into_entry(self) -> Result<Entry, CpioError> {
        let kind = match self.raw_mode & S_IFMT {
            S_IFREG => Kind::File(self.data.to_vec()),
            S_IFDIR => Kind::Dir,
            S_IFLNK => Kind::Symlink(String::from_utf8_lossy(self.data).into_owned()),
            S_IFCHR => Kind::CharDev {
                major: self.rdevmajor,
                minor: self.rdevminor,
            },
            _ => {
                return Err(CpioError::UnsupportedMode {
                    path: self.name,
                    mode: self.raw_mode,
                })
            }
        };
        Ok(Entry {
            path: PathBuf::from("/").join(self.name.trim_start_matches('/')),
            kind,
            mode: self.raw_mode & 0o7777,
            uid: self.uid,
            gid: self.gid,
            mtime: self.mtime,
        })
    }
}

// ── Writing ───────────────────────────────────────────────────────────────────

/// Serialise entries, in the order given, terminated by `TRAILER!!!`.
///
/// The kernel unpacks concatenated archives in order, and a later entry
/// replaces an earlier one with the same path, so the order of the archives
/// sets their precedence.
pub fn write(entries: &[Entry]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        // Inode numbers only have to be distinct: every entry is written with
        // nlink 1 (2 for directories), so the kernel never looks for hardlinks.
        write_record(&mut out, e, i as u32 + 1);
    }
    write_trailer(&mut out);
    out
}

fn write_record(out: &mut Vec<u8>, e: &Entry, ino: u32) {
    let name = archive_name(&e.path);
    let payload: &[u8] = match &e.kind {
        Kind::File(data) => data,
        Kind::Symlink(target) => target.as_bytes(),
        Kind::Dir | Kind::CharDev { .. } => &[],
    };
    let (rdevmajor, rdevminor) = match &e.kind {
        Kind::CharDev { major, minor } => (*major, *minor),
        _ => (0, 0),
    };
    let nlink = if matches!(e.kind, Kind::Dir) { 2 } else { 1 };

    let start = out.len();
    out.extend_from_slice(MAGIC_NEWC);
    for v in [
        ino,
        e.kind.type_bits() | (e.mode & 0o7777),
        e.uid,
        e.gid,
        nlink,
        e.mtime,
        payload.len() as u32,
        0, // devmajor
        0, // devminor
        rdevmajor,
        rdevminor,
        name.len() as u32 + 1, // namesize, including the NUL
        0,                     // check, which newc does not use
    ] {
        out.extend_from_slice(format!("{v:08X}").as_bytes());
    }
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    pad_to_4(out, start);

    let data_start = out.len();
    out.extend_from_slice(payload);
    pad_to_4(out, data_start);
}

fn write_trailer(out: &mut Vec<u8>) {
    let start = out.len();
    out.extend_from_slice(MAGIC_NEWC);
    for v in [
        0u32,
        0,
        0,
        0,
        1,
        0,
        0,
        0,
        0,
        0,
        0,
        TRAILER.len() as u32 + 1,
        0,
    ] {
        out.extend_from_slice(format!("{v:08X}").as_bytes());
    }
    out.extend_from_slice(TRAILER.as_bytes());
    out.push(0);
    pad_to_4(out, start);
}

/// Archive names are relative and carry no leading slash.
fn archive_name(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let s = s.trim_start_matches('/').to_string();
    if s.is_empty() {
        ".".into()
    } else {
        s
    }
}

/// Round a length up to the next multiple of 4.
fn pad4(len: usize) -> usize {
    len.div_ceil(4) * 4
}

/// Pad `out` so its length is 4-aligned relative to `from`.
fn pad_to_4(out: &mut Vec<u8>, from: usize) {
    let want = pad4(out.len() - from) + from;
    out.resize(want, 0);
}

// ── Merge ─────────────────────────────────────────────────────────────────────

/// Applies archives in order, so that a later entry replaces an earlier one
/// with the same path, as the kernel's `unpack_to_rootfs` does for
/// concatenated archives.
///
/// As in the kernel, a merge cannot remove a file; the `/cdj3k-init` prelude
/// script handles deletions.
pub fn merge(archives: &[Vec<Entry>]) -> Vec<Entry> {
    let mut merged: BTreeMap<PathBuf, Entry> = BTreeMap::new();
    for archive in archives {
        for e in archive {
            merged.insert(e.path.clone(), e.clone());
        }
    }
    merged.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Entry> {
        vec![
            Entry::dir("/etc", 0o755),
            Entry::file("/etc/shadow", b"root::\n".to_vec(), 0o600),
            Entry::symlink("/etc/systemd/system/fix-clock.service", "/dev/null"),
            Entry {
                uid: 0,
                gid: 0,
                ..Entry::file("/a", b"odd-length".to_vec(), 0o644)
            },
        ]
    }

    #[test]
    fn a_round_trip_preserves_every_field() {
        let got = read(&write(&sample())).expect("parse");
        assert_eq!(got, sample());
    }

    #[test]
    fn modes_and_symlinks_survive_because_no_filesystem_is_involved() {
        let got = read(&write(&sample())).unwrap();
        let shadow = got.iter().find(|e| e.path.ends_with("shadow")).unwrap();
        assert_eq!(
            shadow.mode, 0o600,
            "mode bits are archive data, not FS state"
        );

        let mask = got
            .iter()
            .find(|e| e.path.ends_with("fix-clock.service"))
            .unwrap();
        assert_eq!(mask.kind, Kind::Symlink("/dev/null".into()));
    }

    #[test]
    fn every_record_starts_4_byte_aligned() {
        // A name and a payload of awkward length must still leave the next
        // header aligned, or the kernel reads garbage.
        let bytes = write(&[
            Entry::file("/odd-name-xyz", b"1".to_vec(), 0o644),
            Entry::file("/b", b"12345".to_vec(), 0o644),
        ]);
        let mut at = 0;
        let mut seen = 0;
        while at + HEADER_LEN <= bytes.len() {
            assert_eq!(at % 4, 0, "record at {at} is not 4-byte aligned");
            assert_eq!(&bytes[at..at + 6], MAGIC_NEWC);
            let hex = |i: usize| {
                u32::from_str_radix(
                    std::str::from_utf8(&bytes[at + 6 + i * 8..at + 14 + i * 8]).unwrap(),
                    16,
                )
                .unwrap() as usize
            };
            let filesize = hex(6);
            let namesize = hex(11);
            let name_end = at + HEADER_LEN + namesize;
            if bytes[at + HEADER_LEN..name_end - 1] == *TRAILER.as_bytes() {
                seen += 1;
                break;
            }
            seen += 1;
            at = pad4(pad4(name_end - at) + filesize) + at;
        }
        assert_eq!(seen, 3, "two entries plus the trailer");
    }

    #[test]
    fn the_archive_is_byte_reproducible() {
        assert_eq!(write(&sample()), write(&sample()));
    }

    #[test]
    fn paths_are_stored_relative() {
        let bytes = write(&[Entry::file("/etc/shadow", vec![], 0o600)]);
        let name_at = HEADER_LEN;
        assert_eq!(&bytes[name_at..name_at + 10], b"etc/shadow");
    }

    #[test]
    fn reading_stops_at_the_first_trailer_so_a_concatenation_is_readable() {
        let mut joined = write(&[Entry::file("/first", b"a".to_vec(), 0o644)]);
        joined.extend_from_slice(&write(&[Entry::file("/second", b"b".to_vec(), 0o644)]));

        let first = read(&joined).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].path, PathBuf::from("/first"));
    }

    #[test]
    fn merging_lets_the_later_archive_win() {
        let base = vec![
            Entry::file("/etc/hosts", b"pristine".to_vec(), 0o644),
            Entry::file("/keep", b"kept".to_vec(), 0o644),
        ];
        let overlay = vec![Entry::file("/etc/hosts", b"patched".to_vec(), 0o600)];

        let merged = merge(&[base, overlay]);
        let hosts = merged.iter().find(|e| e.path.ends_with("hosts")).unwrap();
        assert_eq!(hosts.kind, Kind::File(b"patched".to_vec()));
        assert_eq!(hosts.mode, 0o600);
        assert!(merged.iter().any(|e| e.path == Path::new("/keep")));
    }

    #[test]
    fn a_truncated_archive_is_an_error_not_a_panic() {
        let bytes = write(&sample());
        for cut in [1, HEADER_LEN - 1, HEADER_LEN + 2, bytes.len() - 4] {
            assert!(read(&bytes[..cut]).is_err(), "cut at {cut} should fail");
        }
    }

    #[test]
    fn a_missing_trailer_is_reported() {
        let mut bytes = write(&[Entry::file("/a", b"x".to_vec(), 0o644)]);
        bytes.truncate(HEADER_LEN + 8);
        assert!(matches!(
            read(&bytes),
            Err(CpioError::Truncated { .. }) | Err(CpioError::MissingTrailer)
        ));
    }

    #[test]
    fn each_truncation_reports_the_record_it_cut() {
        let bytes = write(&sample());
        for cut in [HEADER_LEN - 1, HEADER_LEN + 2] {
            assert!(
                matches!(read(&bytes[..cut]), Err(CpioError::Truncated { at: 0 })),
                "cut at {cut}"
            );
        }
        assert!(matches!(read(&[]), Err(CpioError::MissingTrailer)));
        assert!(matches!(read(&[0; 8]), Err(CpioError::MissingTrailer)));
    }

    #[test]
    fn a_foreign_magic_is_reported_where_it_starts() {
        let mut bytes = write(&sample());
        bytes[5] = b'7';
        assert!(matches!(read(&bytes), Err(CpioError::BadMagic { at: 0 })));
    }

    #[test]
    fn a_non_hex_field_names_itself() {
        let mut bytes = write(&sample());
        bytes[6 + 3 * 8] = b'g';
        assert!(matches!(
            read(&bytes),
            Err(CpioError::BadField {
                at: 0,
                field: "gid"
            })
        ));
    }

    #[test]
    fn a_zero_namesize_is_a_bad_field() {
        let mut bytes = write(&sample());
        bytes[6 + 11 * 8..6 + 12 * 8].copy_from_slice(b"00000000");
        assert!(matches!(
            read(&bytes),
            Err(CpioError::BadField {
                at: 0,
                field: "namesize"
            })
        ));
    }

    #[test]
    fn a_block_device_is_refused_by_name() {
        let mut bytes = write(&[Entry::file("/dev/sda", vec![], 0o600)]);
        // S_IFBLK | 0o600.
        bytes[6 + 8..6 + 16].copy_from_slice(b"00006180");
        match read(&bytes) {
            Err(CpioError::UnsupportedMode { path, mode }) => {
                assert_eq!(path, "dev/sda");
                assert_eq!(mode, 0o060600);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_char_device_keeps_its_numbers() {
        let console = Entry {
            kind: Kind::CharDev { major: 5, minor: 1 },
            ..Entry::dir("/dev/console", 0o600)
        };
        assert_eq!(
            read(&write(std::slice::from_ref(&console))).unwrap(),
            vec![console]
        );
    }
}
