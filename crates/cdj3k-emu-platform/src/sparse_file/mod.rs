//! Files that are created large and written little, like guest RAM.

#[cfg(unix)]
#[path = "unix.rs"]
mod imp;
#[cfg(windows)]
#[path = "windows.rs"]
mod imp;
#[cfg(not(any(unix, windows)))]
#[path = "unsupported.rs"]
mod imp;

/// Create (or truncate) `path` to be a sparse file of exactly `len` bytes,
/// where the filesystem has such files.
pub fn create(path: &std::path::Path, len: u64) -> std::io::Result<()> {
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    imp::allow_holes(&f)?;
    f.set_len(len)
}

#[cfg(test)]
mod tests {
    #[test]
    fn create_sets_the_length_and_truncates_what_was_there() {
        let path = std::env::temp_dir().join(format!("cdj3k-sparse-{}", std::process::id()));
        std::fs::write(&path, vec![7u8; 4096]).unwrap();
        super::create(&path, 1 << 20).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 1 << 20);
        assert!(std::fs::read(&path).unwrap().iter().all(|b| *b == 0));
        super::create(&path, 512).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 512);
        let _ = std::fs::remove_file(&path);
    }
}
