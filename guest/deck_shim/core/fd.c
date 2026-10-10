/* SPDX-License-Identifier: MIT OR Apache-2.0 */
/* ------------------------------------------------------------------ */
/* Active fd table helpers                                            */
/* ------------------------------------------------------------------ */
/*
 * The sets are lockless because every intercepted syscall reads them (three
 * or more JUCE threads at the 96 kHz audio cadence), and a shared lock would
 * serialise the audio path. Slots change only on open() and close(), so
 * lookups use atomic loads and stores with acquire/release ordering, and a
 * CAS claims a free slot.
 *
 * Tolerated races:
 *  - fdset_has may miss a concurrent add and return 0. The wrapper then
 *    falls through to the plain syscall, which is correct because the fd is
 *    not fully registered yet.
 *  - fdset_has may see a stale fd while a remove is in flight. The intercept
 *    then runs once more for an fd that was just closed, which is harmless.
 *  - fdset_add uses a CAS, so two concurrent open() calls cannot claim the
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

/* ---- Generic set operations, which model code also uses for its own sets ---- */

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
