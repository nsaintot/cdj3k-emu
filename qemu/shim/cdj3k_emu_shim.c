/*
 * SPDX-License-Identifier: GPL-2.0-or-later
 *
 * cdj3k_emu_shim.c - QEMU embedding shim for libcdj3k-emu-qemu.dylib
 *
 * Linked into libcdj3k-emu-qemu.dylib with the QEMU objects
 * (GPL-2.0-or-later), so it has the same license.
 *
 * cdj3k_emu_qemu_run(argc, argv) runs QEMU and blocks until it exits.
 * cdj3k-emu-runtime calls it on a dedicated background thread. The caller
 * stops QEMU by sending {"execute":"quit"} through QMP; qemu_cleanup() then
 * calls exit(), which the Mach-O interpose below turns into a longjmp.
 *
 * The shim expects the shm display backend (ui/shm-display.c). It disables
 * the macOS CFRunLoop path, so QEMU runs headless on the calling thread.
 */

#include <setjmp.h>
#include <stdlib.h>
#include <stdio.h>
#include <unistd.h>

/* ── Forward declarations of QEMU internals ────────────────────────────────
 * The full headers pull in too many QEMU-internal types. Only the function
 * signatures are needed; the linker resolves them from the linked objects.
 */
void  qemu_init(int argc, char **argv);
int   qemu_main_loop(void);
void  qemu_cleanup(int exitcode);
/* bql_lock() is a macro that expands to bql_lock_impl(__FILE__, __LINE__). */
void  bql_lock_impl(const char *file, int line);
void  bql_unlock(void);
void  replay_mutex_lock(void);
void  replay_mutex_unlock(void);

/* On macOS, qemu_main points to os_darwin_cfrunloop_main by default.
 * cdj3k_emu_qemu_run sets it to NULL, so that the main() path falls through
 * to qemu_default_main, which runs qemu_main_loop() on the calling thread.
 * system/main.c defines it as data. */
extern int (*qemu_main)(void);

/* ── exit() interpose ──────────────────────────────────────────────────────
 * QEMU calls exit() at the end of qemu_cleanup(), which in an embedded dylib
 * would terminate the host process. Mach-O interposing, which applies to
 * every image in the process, replaces exit() and longjmps back to
 * cdj3k_emu_qemu_run.
 */

static jmp_buf  _cdj3k_emu_exit_buf;
static volatile int _cdj3k_emu_in_run = 0;

static void _cdj3k_emu_exit_interpose(int status)
{
    if (_cdj3k_emu_in_run) {
        _cdj3k_emu_in_run = 0;
        longjmp(_cdj3k_emu_exit_buf, status + 1);
    }
    /* Outside cdj3k_emu_qemu_run, end the process. _exit() does not go
     * through the interpose, which hooks only exit(). The interpose patches
     * every image in the process, so the Rust runtime's final exit(0) also
     * arrives here. */
    _exit(status);
}

/* Mach-O interpose entry for exit(). */
__attribute__((used))
static struct {
    const void *replacement;
    const void *replacee;
} _interpose_exit
__attribute__((section("__DATA,__interpose"))) = {
    (const void *)(unsigned long)&_cdj3k_emu_exit_interpose,
    (const void *)(unsigned long)&exit,
};

/* ── cdj3k_emu_qemu_abort ───────────────────────────────────────────────────────
 * The host process calls this when the application quits. It calls _exit()
 * directly, without the longjmp, so the OS ends QEMU's background threads
 * (RCU and others) before they can touch freed state.
 */
__attribute__((visibility("default")))
void cdj3k_emu_qemu_abort(void)
{
    _cdj3k_emu_in_run = 0;
    _exit(0);
}

/* ── Public API ────────────────────────────────────────────────────────────
 *
 * int cdj3k_emu_qemu_run(int argc, char **argv)
 *
 * Run QEMU on the calling thread and block until QEMU exits. On a clean
 * shutdown, a QMP "quit" makes qemu_cleanup call exit(), which the interpose
 * turns into a longjmp back here. Returns QEMU's exit code (0 on a clean
 * stop).
 *
 * Thread safety: only one call may run at a time. Call it again after the
 * previous call has returned to restart QEMU.
 */
__attribute__((visibility("default")))
int cdj3k_emu_qemu_run(int argc, char **argv)
{
    /* Disable the Cocoa CFRunLoop path; QEMU runs headless with the shm
     * backend. */
    qemu_main = NULL;

    int jmp_val = setjmp(_cdj3k_emu_exit_buf);
    if (jmp_val != 0) {
        /* The interpose caught exit(); jmp_val is the exit status + 1. */
        return jmp_val - 1;
    }

    _cdj3k_emu_in_run = 1;

    /* This follows main() in system/main.c with qemu_main set to NULL. It
     * calls qemu_init(), releases the BQL and the replay mutex, then runs
     * the body of qemu_default_main() inline. */
    qemu_init(argc, argv);

    /* qemu_init returns with BQL and replay_mutex held. Release them so
     * the QEMU worker threads (TCG, device I/O) can proceed. */
    bql_unlock();
    replay_mutex_unlock();

    /* Run the main event loop, as qemu_default_main does. */
    replay_mutex_lock();
    bql_lock_impl(__FILE__, __LINE__);
    int status = qemu_main_loop();
    qemu_cleanup(status);   /* calls exit(), which longjmps back to the setjmp above */

    /* Unreachable: the longjmp leaves before this line. */
    return status;
}
