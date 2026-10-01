//! Starting QEMU, which is not the same operation on every host.
//!
//! On macOS QEMU is linked into this binary as a dylib, so it runs in a
//! re-exec of ourselves with `--qemu-worker`: a fresh process per launch keeps
//! QEMU's global state clean, and the worker hands its arguments straight to
//! QEMU's `main`. On Linux and Windows QEMU is an ordinary executable shipped
//! beside us.

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod imp;
#[cfg(windows)]
#[path = "windows.rs"]
mod imp;
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
#[path = "unsupported.rs"]
mod imp;

use std::process::Command;

pub use imp::{qemu_command, run_worker_if_asked};

/// A command that will run QEMU, and whether it expects our `argv[0]`.
///
/// The worker passes what it is given to QEMU's `main`, so `argv[0]` has to be
/// there or QEMU reads the first option as its own program name. An executable
/// supplies its own.
pub struct QemuCommand {
    pub command: Command,
    pub wants_argv0: bool,
}

impl QemuCommand {
    /// Append a full argv, including its `argv[0]`, dropping that first
    /// element for hosts that do not want it.
    pub fn with_argv(mut self, argv: &[String]) -> Command {
        self.command
            .args(if self.wants_argv0 { argv } else { &argv[1..] });
        self.command
    }
}

/// Describe the command for a log line, without running it.
pub fn describe(argv: &[String]) -> String {
    match qemu_command(None) {
        Ok(c) => format!(
            "{} {}",
            c.command.get_program().to_string_lossy(),
            argv.join(" ")
        ),
        Err(e) => format!("<no QEMU: {e}> {}", argv.join(" ")),
    }
}
