/*
 * SPDX-License-Identifier: GPL-2.0-or-later
 *
 * cdj3k_emu_shim.c - QEMU embedding shim for libcdj3k-emu-qemu.dylib
 *
 * Linked into qemu-system-aarch64 / libcdj3k-emu-qemu.dylib alongside QEMU
 * objects (which are GPL-2.0-or-later).  As a derivative of QEMU it inherits
 * the same license.
 *
 * Exposes cdj3k_emu_qemu_run(argc, argv) as a callable C function that blocks
 * until QEMU exits.  Intended to run on a dedicated background thread from
 * Rust (cdj3k-emu-runtime).  The caller stops QEMU by sending {"execute":"quit"}
 * via the QMP TCP port; qemu_cleanup() then calls exit() which is intercepted
 * here via Mach-O dylib interposing and converted to a longjmp.
 *
 * The shm display backend (ui/shm-display.c) is assumed - this disables the
 * macOS CFRunLoop path so QEMU runs headless on the calling thread.
 */

#include <setjmp.h>
#include <stdlib.h>
#include <stdio.h>
#include <unistd.h>

/* ── Forward declarations of QEMU internals ────────────────────────────────
 * Full headers pull in too many QEMU-internal types.  We only need the
 * function signatures; the linker resolves the bodies from the linked objects.
 */
void  qemu_init(int argc, char **argv);
int   qemu_main_loop(void);
void  qemu_cleanup(int exitcode);
/* bql_lock() is a macro in QEMU 10.x: bql_lock_impl(__FILE__, __LINE__) */
void  bql_lock_impl(const char *file, int line);
void  bql_unlock(void);
void  replay_mutex_lock(void);
void  replay_mutex_unlock(void);

/* Function pointer set to os_darwin_cfrunloop_main on macOS by default.
 * We set it to NULL so the main() path falls through to qemu_default_main,
 * which runs qemu_main_loop() on the calling thread (headless mode).
 * Declared as data in system/main.c; visible across the linked objects. */
extern int (*qemu_main)(void);

/* ── exit() interpose ──────────────────────────────────────────────────────
 * QEMU calls exit() at the end of qemu_cleanup().  In an embedded dylib that
 * would terminate the host process.  We intercept it with Mach-O interposing
 * (applies only within this dylib image) and longjmp back to cdj3k_emu_qemu_run.
 */

static jmp_buf  _cdj3k_emu_exit_buf;
static volatile int _cdj3k_emu_in_run = 0;

static void _cdj3k_emu_exit_interpose(int status)
{
    if (_cdj3k_emu_in_run) {
        _cdj3k_emu_in_run = 0;
        longjmp(_cdj3k_emu_exit_buf, status + 1);
    }
    /* Not inside cdj3k_emu_qemu_run - pass through to the real exit.
     * Use _exit() to avoid re-entering our own interpose (we only hooked
     * exit, not _exit).  This also covers the normal host-process shutdown
     * path: Mach-O __interpose patches all images in the process, so the
     * Rust runtime's final exit(0) lands here too. */
    _exit(status);
}

/* Mach-O dylib interpose section - replaces exit() for all code linked
 * into this dylib image (i.e. all QEMU code that calls exit()).         */
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
 * Called by the host process on clean application quit.  Skips longjmp and
 * calls _exit() directly so QEMU background threads (RCU, etc.) are killed
 * by the OS rather than crashing on freed state.
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
 * Run QEMU on the calling thread.  Blocks until QEMU exits (clean shutdown
 * via QMP "quit" → qemu_cleanup → exit() → longjmp here).
 * Returns QEMU's exit code (0 on clean stop).
 *
 * Thread safety: call at most once at a time; restart by calling again after
 * the previous call has returned.
 */
__attribute__((visibility("default")))
int cdj3k_emu_qemu_run(int argc, char **argv)
{
    /* Disable the Cocoa CFRunLoop path - we run headless (shm backend). */
    qemu_main = NULL;

    int jmp_val = setjmp(_cdj3k_emu_exit_buf);
    if (jmp_val != 0) {
        /* exit() was intercepted; jmp_val = exit_status + 1. */
        return jmp_val - 1;
    }

    _cdj3k_emu_in_run = 1;

    /* Mirrors the logic in system/main.c::main() with qemu_main == NULL:
     *   qemu_init() → bql_unlock/replay_unlock → qemu_default_main() inline */
    qemu_init(argc, argv);

    /* qemu_init returns with BQL and replay_mutex held. Release them so
     * the QEMU worker threads (TCG, device I/O) can proceed. */
    bql_unlock();
    replay_mutex_unlock();

    /* Run the main event loop (same as qemu_default_main on a thread). */
    replay_mutex_lock();
    bql_lock_impl(__FILE__, __LINE__);
    int status = qemu_main_loop();
    qemu_cleanup(status);   /* → exit() → _cdj3k_emu_exit_interpose → longjmp */

    /* Unreachable - longjmp exits before we get here. */
    return status;
}
