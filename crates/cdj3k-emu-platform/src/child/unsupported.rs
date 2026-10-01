use std::process::Command;

/// No console to hide.
pub fn quiet(_command: &mut Command) {}

pub const EXTRA_TOOL_DIRS: &[&str] = &[];

/// Runnability is the extension's, which the caller has resolved.
pub fn is_runnable(path: &std::path::Path) -> bool {
    path.is_file()
}

pub fn set_runnable(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}
