// SPDX-License-Identifier: MIT OR Apache-2.0
/*
 * syscalls_ioctl.c - interposed ioctl(2). Dispatches DRM master/render-node
 * fds to handle_drm_ioctl (jog.c) and the subucom virtual SPI fd to its own
 * handler; everything else falls through to the real libc ioctl.
 */

#include "ep122_shim.h"
#include "mods/core/cdj3k_mods.h"

int ioctl(int fd, unsigned long request, ...) {
    va_list ap; va_start(ap, request);
    void *arg = va_arg(ap, void *);
    va_end(ap);

    /* DRM fd */
    if (is_drm_fd(fd))
        return handle_drm_ioctl(fd, request, arg);

    /* GPIODRV fd - accept any ioctl silently (e.g. GPIO pin configure) */
    if (is_gpiodrv_fd(fd)) {
        DBG("gpiodrv ioctl fd=%d request=0x%lx arg=%p\n", fd, (unsigned long)request, arg);
        return 0;
    }

    /* HIDG fd - accept any ioctl silently.
     *   f_hid's char device returns ENOTTY for HID-class ioctls (it only
     *   recognises a small set).  EP122's USB gadget manager interprets
     *   ENOTTY as a gadget initialisation failure and may show
     *   "USB Error. Remove the device."  Reads, writes, and poll all
     *   pass through to the real f_hid device; only ioctl is masked. */
    if (is_hidg_fd(fd))
        return 0;

    /* All subucom ioctls pass through to subucom_virt.ko */
    return sys_ioctl(fd, request, arg);
}

/* ------------------------------------------------------------------ */
/* read                                                               */
/* ------------------------------------------------------------------ */
ssize_t read(int fd, void *buf, size_t count) {
    /* --- DRM fd: synthesise a DRM_EVENT_FLIP_COMPLETE when a flip is pending.
     *   EP122's render loop calls drmHandleEvent(fd, ctx) which internally calls
     *   read() expecting a 32-byte drm_event_vblank.  The real card0 fd has no
     *   actual flip events because SETPLANE is faked.  We return a synthetic
     *   packet when inject_drm_flip_event() has been called (counter > 0);
     *   otherwise EAGAIN (non-blocking, caller should poll first). */
    if (is_drm_fd(fd)) {
        /* Per-fd flip event: only synthesise for fds that have a flip slot. */
        jog_flip_slot_t *rslot = flip_slot_for(fd);
        if (!rslot) { errno = EAGAIN; return -1; }
        int pending = __atomic_exchange_n(&rslot->pending, 0, __ATOMIC_ACQUIRE);
        if (!pending || count < 32) { errno = EAGAIN; return -1; }
        static uint32_t g_rd_seq = 0;
        uint8_t ev[32];
        memset(ev, 0, sizeof(ev));
        uint32_t type = 2u, length = 32u; /* DRM_EVENT_FLIP_COMPLETE */
        uint32_t seq  = __atomic_fetch_add(&g_rd_seq, 1, __ATOMIC_RELAXED);
        uint64_t ud   = __atomic_load_n(&rslot->user_data, __ATOMIC_ACQUIRE);
        memcpy(ev + 0,  &type,   4);
        memcpy(ev + 4,  &length, 4);
        memcpy(ev + 8,  &ud,     8);
        memcpy(ev + 24, &seq,    4);
        memcpy(buf, ev, 32);
        return 32;
    }

    /* --- GPIODRV fd: passthrough to pipe + log result for shutdown tracing --- */
    if (is_gpiodrv_fd(fd)) {
        ssize_t r = sys_read(fd, buf, count);
        if (g_debug && r > 0) {
            unsigned char b0 = count > 0 ? ((unsigned char *)buf)[0] : 0;
            unsigned char b1 = count > 1 ? ((unsigned char *)buf)[1] : 0;
            unsigned char b2 = count > 2 ? ((unsigned char *)buf)[2] : 0;
            unsigned char b3 = count > 3 ? ((unsigned char *)buf)[3] : 0;
            DBG("gpiodrv read fd=%d count=%zu r=%zd bytes[0..3]=%02x %02x %02x %02x\n",
                fd, count, r, b0, b1, b2, b3);
        }
        return r;
    }

    /* --- All other fds (including subucom_virt.ko): passthrough --- */
    return sys_read(fd, buf, count);
}

/* ------------------------------------------------------------------ */
/* write - passthrough, with a db_watch tap                           */
/* ------------------------------------------------------------------ */
/* HIDG and seq writes reach the real f_hid / f_midi char devices,    */
/* which fan out as USB input reports / MIDI events on the gadget bus.*/
ssize_t write(int fd, const void *buf, size_t count) {
    /* The caller, not just the fact. A return address is what turns "the
     * library file changed" into "this code changed it". */
    db_watch_write(fd, count, (uintptr_t)__builtin_return_address(0));
    return sys_write(fd, buf, count);
}
