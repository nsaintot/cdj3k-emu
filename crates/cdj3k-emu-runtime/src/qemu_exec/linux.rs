//! QEMU as an executable shipped beside us.

use std::io;
use std::process::Command;

use super::QemuCommand;

/// How to start QEMU on this host: the executable shipped beside us.
/// `keep_fd` is inherited by the child.
pub fn qemu_command(keep_fd: Option<i32>) -> io::Result<QemuCommand> {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(cdj3k_emu_platform::bundled::tool("qemu-system-aarch64"));
    // SAFETY: only async-signal-safe calls between fork and exec.
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
