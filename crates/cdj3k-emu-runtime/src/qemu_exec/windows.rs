//! QEMU as `qemu-system-aarch64.exe` shipped beside us.

use std::io;

use super::QemuCommand;

/// How to start QEMU on this host: the executable shipped beside us. No
/// descriptor is passed down, so `_keep_fd` is unused.
pub fn qemu_command(_keep_fd: Option<i32>) -> io::Result<QemuCommand> {
    let command = cdj3k_emu_platform::child::command(cdj3k_emu_platform::bundled::tool(
        "qemu-system-aarch64",
    ));
    Ok(QemuCommand {
        command,
        wants_argv0: false,
    })
}

/// QEMU runs as its own executable here, so there is no worker mode.
pub fn run_worker_if_asked(_args: &[String]) {}
