//! Helper processes the user never sees: `qemu-img`, `netsh`, PowerShell.

#[cfg(unix)]
#[path = "unix.rs"]
mod imp;
#[cfg(windows)]
#[path = "windows.rs"]
mod imp;
#[cfg(not(any(unix, windows)))]
#[path = "unsupported.rs"]
mod imp;

pub use imp::{is_runnable, set_runnable, EXTRA_TOOL_DIRS};

use std::ffi::OsStr;
use std::process::Command;

/// A `Command` for `program` that opens no window of its own.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    imp::quiet(&mut command);
    command
}
