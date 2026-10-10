//! Unix filesystems leave holes unallocated by default, so extending the file
//! with `set_len` is enough.

pub fn allow_holes(_file: &std::fs::File) -> std::io::Result<()> {
    Ok(())
}
