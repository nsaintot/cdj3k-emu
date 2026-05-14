// Raw FFI bindings to libcdj3k-emu-qemu.dylib.
// cdj3k_emu_qemu_run blocks until QEMU exits (intended to run on a dedicated thread).
// Stop via QMP "quit" → qemu_cleanup() → exit() → dylib interpose → longjmp → return.
#[cfg(target_os = "macos")]
extern "C" {
    pub fn cdj3k_emu_qemu_run(argc: libc::c_int, argv: *const *const libc::c_char) -> libc::c_int;
    /// Hard-quit: clears the longjmp flag and calls _exit(0).
    /// Use on application shutdown to avoid QEMU background threads crashing
    /// on freed state after longjmp.
    pub fn cdj3k_emu_qemu_abort();
}
