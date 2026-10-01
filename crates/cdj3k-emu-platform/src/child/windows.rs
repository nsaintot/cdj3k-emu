use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A console program started by a GUI process gets a console window unless
/// told not to.
pub fn quiet(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}
