/* SPDX-License-Identifier: MIT OR Apache-2.0 */
/* ------------------------------------------------------------------ */
/* Active fd table helpers                                            */
/* ------------------------------------------------------------------ */
/*
 * Lockless. Reads happen on every intercepted syscall (3+ JUCE threads at
 * 96 kHz audio cadence) so a shared spinlock here serialised the entire
 * audio path and was the dominant contributor to guest-side spinning under
 * load. Slots only change on open()/close(), so we use atomic load/store
 * with acquire/release ordering for lookups, and CAS to claim a free slot.
 *
 * Race tolerance:
 *  - fdset_has may miss a concurrent add and return 0 - the wrapper falls
 *    through to the unintercepted libc syscall, which is correct: the fd
 *    isn't fully registered yet.
 *  - fdset_has may see a stale fd while a remove is in flight - the
 *    intercept fires once more for an fd we've just closed; harmless.
 *  - fdset_add uses CAS to ensure two concurrent open()s can't claim the
 *    same slot.
 */

#include "deck_shim.h"

#define LOAD(p)        __atomic_load_n((p), __ATOMIC_ACQUIRE)
#define STORE(p, v)    __atomic_store_n((p), (v), __ATOMIC_RELEASE)

static inline int try_claim(int *slot, int fd) {
    int expected = -1;
    return __atomic_compare_exchange_n(slot, &expected, fd,
                                       /* weak */ 0,
                                       __ATOMIC_RELEASE,
                                       __ATOMIC_RELAXED);
}

/* ---- Generic set operations; model code builds its own sets on these ---- */

int fdset_has(int *set, int n, int fd) {
    if (fd < 0) return 0;
    for (int i = 0; i < n; i++) {
        if (LOAD(&set[i]) == fd) return 1;
    }
    return 0;
}

int fdset_add(int *set, int n, int fd) {
    for (int i = 0; i < n; i++) {
        if (try_claim(&set[i], fd)) return 0;
    }
    return -1;
}

void fdset_remove(int *set, int n, int fd) {
    for (int i = 0; i < n; i++) {
        if (LOAD(&set[i]) == fd) { STORE(&set[i], -1); break; }
    }
}

/* ---- The core's own sets ---- */

int  is_drm_fd(int fd)      { return fdset_has(g_drm_active, MAX_ACTIVE_FDS, fd); }
int  add_drm_fd(int fd)     { return fdset_add(g_drm_active, MAX_ACTIVE_FDS, fd); }
void remove_drm_fd(int fd)  { fdset_remove(g_drm_active, MAX_ACTIVE_FDS, fd); }

int  is_hidg_fd(int fd)     { return fdset_has(g_hidg_active, MAX_HIDG_FDS, fd); }
int  add_hidg_fd(int fd)    { return fdset_add(g_hidg_active, MAX_HIDG_FDS, fd); }
void remove_hidg_fd(int fd) { fdset_remove(g_hidg_active, MAX_HIDG_FDS, fd); }

int  is_gpiodrv_fd(int fd)     { return fdset_has(g_gpiodrv_active, MAX_GPIODRV_FDS, fd); }
int  add_gpiodrv_fd(int fd)    { return fdset_add(g_gpiodrv_active, MAX_GPIODRV_FDS, fd); }
void remove_gpiodrv_fd(int fd) { fdset_remove(g_gpiodrv_active, MAX_GPIODRV_FDS, fd); }
