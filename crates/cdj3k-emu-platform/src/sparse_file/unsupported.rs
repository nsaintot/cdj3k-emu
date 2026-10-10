//! On a host whose sparse-file support is unknown, the file is only extended
//! with `set_len`.

pub fn allow_holes(_file: &std::fs::File) -> std::io::Result<()> {
    Ok(())
}
