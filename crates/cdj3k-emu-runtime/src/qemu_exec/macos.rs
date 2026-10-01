//! QEMU as a dylib linked into this binary, run by a re-exec of it.

use std::io;
use std::process::Command;

use super::QemuCommand;

/// How to start QEMU on this host: a re-exec of this binary, whose worker
/// runs the QEMU linked into it. `keep_fd` is inherited by the child.
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

/// When this process is the worker [`qemu_command`] starts, run QEMU in it
/// and exit; otherwise return.
///
/// The worker ends itself when its parent goes, since launchd adopts an
/// orphan.
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
