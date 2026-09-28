//! polkit, through `pkexec`. The desktop's own authentication agent draws
//! the prompt, so this matches whatever the user already sees for
//! privileged operations.

use std::io;
use std::process::Command;

pub fn run_elevated(sh_cmd: &str) -> io::Result<()> {
    if sh_cmd.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "shell command contains null byte",
        ));
    }
    let status = Command::new("pkexec")
        .arg("/bin/sh")
        .arg("-c")
        .arg(sh_cmd)
        .status()
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("pkexec not available ({e}); install polkit to grant privileges"),
            )
        })?;

    match status.code() {
        Some(0) => Ok(()),
        // pkexec: 126 when the dialog was dismissed, 127 when authorization
        // could not be obtained, as for an app started outside the graphical
        // session (no authentication agent).
        Some(126) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "admin elevation refused",
        )),
        Some(127) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "no one to ask for admin rights: this app is running outside \
             the desktop session (started over SSH or detached), where \
             polkit has no agent to show a prompt",
        )),
        Some(code) => Err(io::Error::other(format!(
            "elevated command failed with status {code}"
        ))),
        None => Err(io::Error::other("elevated command killed by a signal")),
    }
}
