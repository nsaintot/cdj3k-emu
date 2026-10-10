//! Runs QEMU as an executable that ships beside this binary.

use std::io;
use std::process::Command;

use super::QemuCommand;

/// Build the command that starts QEMU on this host: the executable that ships
/// beside this binary. The child inherits `keep_fd`.
pub fn qemu_command(keep_fd: Option<i32>) -> io::Result<QemuCommand> {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(cdj3k_emu_platform::bundled::tool("qemu-system-aarch64"));
    // SAFETY: the closure makes only async-signal-safe calls between fork and exec.
    unsafe {
        command.pre_exec(move || {
            if let Some(fd) = keep_fd {
                crate::process::keep_across_exec(fd);
            }
            Ok(())
        });
    }
    Ok(QemuCommand {
        command,
        wants_argv0: false,
    })
}

/// QEMU runs as its own executable here, so there is no worker mode.
pub fn run_worker_if_asked(_args: &[String]) {}
