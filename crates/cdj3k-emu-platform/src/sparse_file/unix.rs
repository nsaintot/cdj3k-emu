//! A filesystem that leaves holes unallocated without being told: extending
//! the file with `set_len` is enough.

pub fn allow_holes(_file: &std::fs::File) -> std::io::Result<()> {
    Ok(())
}
