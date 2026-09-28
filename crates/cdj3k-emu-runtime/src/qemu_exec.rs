//! Starting QEMU, which is not the same operation on every host.
//!
//! On macOS QEMU is linked into this binary as a dylib, so it runs in a
//! re-exec of ourselves with `--qemu-worker`: a fresh process per launch keeps
//! QEMU's global state clean, and the worker hands its arguments straight to
//! QEMU's `main`. On Linux QEMU is an ordinary executable shipped beside us.

use std::io;
use std::process::Command;

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

/// How to start QEMU on this host: a re-exec of this binary, whose worker
/// runs the QEMU linked into it. `keep_fd` is inherited by the child.
#[cfg(target_os = "macos")]
pub fn qemu_command(keep_fd: Option<i32>) -> io::Result<QemuCommand> {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(std::env::current_exe()?);
    command.arg("--qemu-worker");
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        command.pre_exec(move || {
            if let Some(fd) = keep_fd {
                crate::process::keep_across_exec(fd);
            }
            // USER_INTERACTIVE keeps QEMU's threads, which inherit it, on the
            // performance cores.
            extern "C" {
                fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
            }
            const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;
            pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0);
            Ok(())
        });
    }
    Ok(QemuCommand {
        command,
        wants_argv0: true,
    })
}

/// How to start QEMU on this host: the executable shipped beside us.
/// `keep_fd` is inherited by the child.
#[cfg(target_os = "linux")]
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

/// The executable a host adapter would ship beside us; no descriptor is
/// passed down.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn qemu_command(_keep_fd: Option<i32>) -> io::Result<QemuCommand> {
    Ok(QemuCommand {
        command: Command::new(cdj3k_emu_platform::bundled::tool("qemu-system-aarch64")),
        wants_argv0: false,
    })
}

/// When this process is the worker [`qemu_command`] starts, run QEMU in it
/// and exit; otherwise return.
///
/// The worker ends itself when its parent goes, since launchd adopts an
/// orphan.
#[cfg(target_os = "macos")]
pub fn run_worker_if_asked(args: &[String]) {
    use std::ffi::CString;
    if args.get(1).map(String::as_str) != Some("--qemu-worker") {
        return;
    }
    // SAFETY: `getppid` has no preconditions.
    let parent = unsafe { libc::getppid() };
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        if !crate::process::is_alive(parent) {
            eprintln!("cdj3k-emu-worker: parent gone, aborting");
            // SAFETY: ends the process; QEMU's threads go with it.
            unsafe {
                crate::ffi::cdj3k_emu_qemu_abort();
                libc::_exit(0)
            };
        }
    });

    let qemu_args = &args[2..];
    eprintln!("cdj3k-emu-worker: {}", qemu_args.join(" "));
    let c_strings: Vec<CString> = qemu_args
        .iter()
        .map(|s| CString::new(s.as_str()).expect("argv NUL"))
        .collect();
    let c_ptrs: Vec<*const libc::c_char> = c_strings.iter().map(|cs| cs.as_ptr()).collect();
    // SAFETY: `c_ptrs` points into `c_strings`, which outlives the call.
    let code = unsafe { crate::ffi::cdj3k_emu_qemu_run(c_ptrs.len() as libc::c_int, c_ptrs.as_ptr()) };
    // QEMU returns through a longjmp out of `exit`, leaving its threads
    // running; `_exit` ends them all at once.
    unsafe { libc::_exit(code) };
}

/// QEMU runs as its own executable here, so there is no worker mode.
#[cfg(target_os = "linux")]
pub fn run_worker_if_asked(_args: &[String]) {}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn run_worker_if_asked(_args: &[String]) {}

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
