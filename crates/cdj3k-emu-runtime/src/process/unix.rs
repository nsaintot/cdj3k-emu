//! POSIX signal handling. The signal handler and `exit_now` call only
//! async-signal-safe functions.

use std::sync::atomic::{AtomicI32, Ordering};

/// The write end of the pipe that hands a caught signal to [`watch_signals`],
/// or -1 before it exists.
static SIGNAL_PIPE: AtomicI32 = AtomicI32::new(-1);

/// Install an `atexit` hook and a handler for the signals that end a desktop
/// app. SIGINT and SIGTERM close the window the way Cmd+Q does, so the guest
/// powers off first. A second signal, or one with no window up, ends the app
/// at once, as do SIGQUIT and SIGHUP: after a hangup the terminal is gone,
/// and writing a log line to it would panic.
pub fn install_exit_hooks() {
    extern "C" fn at_exit() {
        crate::cleanup_runtime_files();
    }
    extern "C" fn on_signal(sig: libc::c_int) {
        let fd = SIGNAL_PIPE.load(Ordering::Relaxed);
        if sig == libc::SIGQUIT || sig == libc::SIGHUP || fd < 0 {
            exit_now();
        }
        let byte = sig as u8;
        // SAFETY: `write` is async-signal-safe, and `byte` outlives the call.
        unsafe { libc::write(fd, (&raw const byte).cast(), 1) };
    }
    let mut fds = [-1; 2];
    // SAFETY: `fds` has room for the two descriptors `pipe` writes.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } == 0 {
        for fd in fds {
            // SAFETY: `fcntl` touches only the descriptor table. The child
            // processes must not inherit the pipe.
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        }
        let read_end = fds[0];
        let spawned = std::thread::Builder::new()
            .name("exit-signals".into())
            .spawn(move || watch_signals(read_end));
        if spawned.is_ok() {
            SIGNAL_PIPE.store(fds[1], Ordering::Relaxed);
        }
    }
    // SAFETY: both handlers are `extern "C"` functions that live forever.
    unsafe {
        libc::atexit(at_exit);
        for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
            libc::signal(sig, on_signal as *const () as libc::sighandler_t);
        }
    }
}

/// Take each caught signal off the pipe. The first one asks the window to
/// quit; the next one, or one with no window to ask, ends the app at once.
fn watch_signals(read_end: i32) {
    let mut asked = false;
    loop {
        let mut sig = 0u8;
        // SAFETY: `sig` has room for the one byte read.
        let n = unsafe { libc::read(read_end, (&raw mut sig).cast(), 1) };
        if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if n <= 0 {
            return;
        }
        if !asked && cdj3k_emu_platform::desktop::request_quit() {
            asked = true;
            eprintln!(
                "cdj3k-emu: Signal {sig} received; asking the window to quit. Send it again to quit at once."
            );
            continue;
        }
        exit_now();
    }
}

/// Kill QEMU, remove the runtime files and end the process. Every call it
/// makes is async-signal-safe.
fn exit_now() -> ! {
    cdj3k_emu_platform::menu_state::APP_SHUTDOWN.store(true, Ordering::Relaxed);
    crate::kill_qemu_child_now();
    crate::cleanup_runtime_files();
    // SAFETY: `_exit` is async-signal-safe.
    unsafe { libc::_exit(0) }
}

/// Start `command` in a process group of its own, so that a Ctrl+C in the
/// terminal reaches only the app, which powers the guest off first.
pub fn own_group(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

/// Ask the process to exit (`SIGTERM`).
pub fn terminate(pid: i32) {
    // SAFETY: `kill` has no memory preconditions.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
}

/// End the process now (`SIGKILL`).
pub fn kill(pid: i32) {
    // SAFETY: as above.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
}

/// Do nothing: the exit hooks reach the child by its pid.
pub fn adopt(_pid: u32) {}

/// Do nothing: `terminate` signals the pid and needs no monitor session.
pub fn set_quit_channel(_monitor: &std::net::TcpStream) {}

/// Clear `FD_CLOEXEC` on `fd`, for a forked child about to exec.
pub fn keep_across_exec(fd: i32) {
    // SAFETY: `fcntl` touches only the descriptor table.
    unsafe { libc::fcntl(fd, libc::F_SETFD, 0) };
}

pub fn is_alive(pid: i32) -> bool {
    // SAFETY: as above; signal 0 only checks that the pid exists.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}
