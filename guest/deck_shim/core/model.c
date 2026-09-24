/* SPDX-License-Identifier: MIT OR Apache-2.0 */
/* ------------------------------------------------------------------ */
/* Which deck the shim is loaded into                                 */
/* ------------------------------------------------------------------ */
/*
 * The model comes from subucom_virt.ko, which the rootfs patch loads with
 * `model=cdj3k` or `model=cdj3kx` and exports read-only under /sys/module.
 * It is read once, on the first call, and cached: the answer cannot change
 * for the life of the process, and callers sit on hot paths (read() of
 * every sub-CPU frame).
 */

#include "deck_shim.h"

static int g_model;   /* 0 until resolved, then a deck_model_t value */

deck_model_t deck_model(void) {
    int cached = __atomic_load_n(&g_model, __ATOMIC_ACQUIRE);
    if (cached) return (deck_model_t)cached;

    int model = DECK_MODEL_CDJ3K;
    int fd = sys_openat(DECK_MODEL_PARAM, O_RDONLY, 0);
    if (fd >= 0) {
        char buf[16];
        ssize_t r = sys_read(fd, buf, sizeof(buf) - 1);
        sys_close(fd);
        if (r > 0) {
            buf[r] = '\0';
            if (strncmp(buf, "cdj3kx", 6) == 0) model = DECK_MODEL_CDJ3KX;
            else if (strncmp(buf, "cdj3k", 5) == 0) model = DECK_MODEL_CDJ3K;
            else DBG("model: %s says %s, using %d\n", DECK_MODEL_PARAM, buf, model);
        }
    } else {
        DBG("model: %s unreadable, using %d\n", DECK_MODEL_PARAM, model);
    }

    __atomic_store_n(&g_model, model, __ATOMIC_RELEASE);
    DBG("model: deck is %s\n", model == DECK_MODEL_CDJ3KX ? "cdj3kx" : "cdj3k");
    return (deck_model_t)model;
}
