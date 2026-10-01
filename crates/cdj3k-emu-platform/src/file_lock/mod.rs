//! An exclusive claim on an open file, which keeps two app windows off one
//! slot's eMMC.
//!
//! Every implementation is released by the OS when the handle closes, which
//! is what lets a slot stay claimable after a crash.

#[cfg(unix)]
#[path = "unix.rs"]
mod imp;
#[cfg(windows)]
#[path = "windows.rs"]
mod imp;
#[cfg(not(any(unix, windows)))]
#[path = "unsupported.rs"]
mod imp;

use std::io;
use std::path::Path;

/// Take the exclusive claim on the open `file`, failing with
/// [`WouldBlock`](io::ErrorKind::WouldBlock) when another open file holds it.
pub fn lock(file: &std::fs::File, path: &Path) -> io::Result<()> {
    imp::lock(file, path)
}

#[cfg(all(test, any(unix, windows)))]
mod tests {
    use super::*;
    use std::io::{Read, Seek, Write};

    fn open(path: &Path) -> std::fs::File {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap()
    }

    #[test]
    fn a_second_open_file_cannot_claim_until_the_first_closes() {
        let path = std::env::temp_dir().join(format!("cdj3k-lock-{}.img", std::process::id()));
        std::fs::write(&path, b"image").unwrap();

        let first = open(&path);
        lock(&first, &path).unwrap();
        let second = open(&path);
        assert_eq!(
            lock(&second, &path).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );

        drop(first);
        lock(&second, &path).unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_claim_leaves_the_file_usable_through_other_handles() {
        let path = std::env::temp_dir().join(format!("cdj3k-lock-rw-{}.img", std::process::id()));
        std::fs::write(&path, b"image").unwrap();

        let holder = open(&path);
        lock(&holder, &path).unwrap();
        let mut other = open(&path);
        other.write_all(b"IM").unwrap();
        other.rewind().unwrap();
        let mut back = String::new();
        other.read_to_string(&mut back).unwrap();
        assert_eq!(back, "IMage");
        let _ = std::fs::remove_file(&path);
    }
}
