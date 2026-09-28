//! POSIX signals. Each call is async-signal-safe.

/// `atexit`, and a handler for the signals that end a desktop app, which
/// cleans up inline and exits: `atexit` does not run across an AppKit quit.
pub fn install_exit_hooks() {
    extern "C" fn at_exit() {
        crate::cleanup_runtime_files();
    }
    extern "C" fn on_signal(_sig: libc::c_int) {
        cdj3k_emu_platform::menu_state::APP_SHUTDOWN
            .store(true, std::sync::atomic::Ordering::Relaxed);
        crate::kill_qemu_child_now();
        crate::cleanup_runtime_files();
        // SAFETY: `_exit` is async-signal-safe.
        unsafe { libc::_exit(0) };
    }
    // SAFETY: both handlers are `extern "C"` functions that live forever.
    unsafe {
        libc::atexit(at_exit);
        for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
            libc::signal(sig, on_signal as *const () as libc::sighandler_t);
        }
    }
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

/// Clear `FD_CLOEXEC` on `fd`, for a forked child about to exec.
pub fn keep_across_exec(fd: i32) {
    // SAFETY: `fcntl` touches only the descriptor table.
    unsafe { libc::fcntl(fd, libc::F_SETFD, 0) };
}

pub fn is_alive(pid: i32) -> bool {
    // SAFETY: as above; signal 0 only checks that the pid exists.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}
