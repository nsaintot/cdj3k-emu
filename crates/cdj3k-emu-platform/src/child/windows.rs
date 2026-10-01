use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A console program started by a GUI process gets a console window unless
/// told not to.
pub fn quiet(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

pub const EXTRA_TOOL_DIRS: &[&str] = &[];

/// Runnability is the extension's, which the caller has resolved.
pub fn is_runnable(path: &std::path::Path) -> bool {
    path.is_file()
}

pub fn set_runnable(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}
