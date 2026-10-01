//! A host with no QEMU build of its own: the command names the executable a
//! host adapter would ship beside us.

use std::io;
use std::process::Command;

use super::QemuCommand;

pub fn qemu_command(_keep_fd: Option<i32>) -> io::Result<QemuCommand> {
    Ok(QemuCommand {
        command: Command::new(cdj3k_emu_platform::bundled::tool("qemu-system-aarch64")),
        wants_argv0: false,
    })
}

pub fn run_worker_if_asked(_args: &[String]) {}
