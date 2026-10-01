//! A host whose sparse-file support is unknown: extending the file with
//! `set_len` is all that is asked of it.

pub fn allow_holes(_file: &std::fs::File) -> std::io::Result<()> {
    Ok(())
}
