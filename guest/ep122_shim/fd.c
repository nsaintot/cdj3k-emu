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
 *  - is_*_fd may miss a concurrent add() and return 0 - the wrapper falls
 *    through to the unintercepted libc syscall, which is correct: the fd
 *    isn't fully registered yet.
 *  - is_*_fd may see a stale fd while a remove() is in flight - the
 *    intercept fires once more for an fd we've just closed; harmless.
 *  - add() uses CAS to ensure two concurrent open()s can't claim the same
 *    slot.
 */

#include "ep122_shim.h"

#define LOAD(p)        __atomic_load_n((p), __ATOMIC_ACQUIRE)
#define STORE(p, v)    __atomic_store_n((p), (v), __ATOMIC_RELEASE)

static inline int try_claim(int *slot, int fd) {
    int expected = -1;
    return __atomic_compare_exchange_n(slot, &expected, fd,
                                       /* weak */ 0,
                                       __ATOMIC_RELEASE,
                                       __ATOMIC_RELAXED);
}

int is_drm_fd(int fd) {
    if (fd < 0) return 0;
    for (int i = 0; i < MAX_ACTIVE_FDS; i++) {
        if (LOAD(&g_drm_active[i]) == fd) return 1;
    }
    return 0;
}

int add_drm_fd(int fd) {
    for (int i = 0; i < MAX_ACTIVE_FDS; i++) {
        if (try_claim(&g_drm_active[i], fd)) return 0;
    }
    return -1;
}

void remove_drm_fd(int fd) {
    for (int i = 0; i < MAX_ACTIVE_FDS; i++) {
        if (LOAD(&g_drm_active[i]) == fd) { STORE(&g_drm_active[i], -1); break; }
    }
}

/* ---- ALSA SEQ fd helpers ---- */
int is_seq_fd(int fd) {
    if (fd < 0) return 0;
    for (int i = 0; i < MAX_ALSA_FDS; i++) {
        if (LOAD(&g_seq_active[i]) == fd) return 1;
    }
    return 0;
}
int add_seq_fd(int fd) {
    for (int i = 0; i < MAX_ALSA_FDS; i++) {
        if (try_claim(&g_seq_active[i], fd)) return 0;
    }
    return -1;
}
void remove_seq_fd(int fd) {
    for (int i = 0; i < MAX_ALSA_FDS; i++) {
        if (LOAD(&g_seq_active[i]) == fd) { STORE(&g_seq_active[i], -1); break; }
    }
}

/* ---- HIDG fd helpers ---- */
int is_hidg_fd(int fd) {
    if (fd < 0) return 0;
    for (int i = 0; i < MAX_HIDG_FDS; i++) {
        if (LOAD(&g_hidg_active[i]) == fd) return 1;
    }
    return 0;
}
int add_hidg_fd(int fd) {
    for (int i = 0; i < MAX_HIDG_FDS; i++) {
        if (try_claim(&g_hidg_active[i], fd)) return 0;
    }
    return -1;
}
void remove_hidg_fd(int fd) {
    for (int i = 0; i < MAX_HIDG_FDS; i++) {
        if (LOAD(&g_hidg_active[i]) == fd) { STORE(&g_hidg_active[i], -1); break; }
    }
}

/* ---- GPIODRV fd helpers ---- */
int is_gpiodrv_fd(int fd) {
    if (fd < 0) return 0;
    for (int i = 0; i < MAX_GPIODRV_FDS; i++) {
        if (LOAD(&g_gpiodrv_active[i]) == fd) return 1;
    }
    return 0;
}
int add_gpiodrv_fd(int fd) {
    for (int i = 0; i < MAX_GPIODRV_FDS; i++) {
        if (try_claim(&g_gpiodrv_active[i], fd)) return 0;
    }
    return -1;
}
void remove_gpiodrv_fd(int fd) {
    for (int i = 0; i < MAX_GPIODRV_FDS; i++) {
        if (LOAD(&g_gpiodrv_active[i]) == fd) { STORE(&g_gpiodrv_active[i], -1); break; }
    }
}
