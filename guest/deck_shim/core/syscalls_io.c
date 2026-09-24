// SPDX-License-Identifier: MIT OR Apache-2.0
/*
 * syscalls_io.c - interposed ioctl(2) and read(2). ioctl dispatches DRM
 * master/render-node fds to handle_drm_ioctl (jog.c) and the subucom virtual
 * SPI fd to its own handler; everything else falls through to the real libc
 * ioctl.
 */

#include "deck_shim.h"

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
     *   recognises a small set).  The app's USB gadget manager interprets
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
     *   The app's render loop calls drmHandleEvent(fd, ctx) which internally calls
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

    /* --- GPIODRV fd: a pin-state read answers at once and never blocks.  The
     *   backing pipe is an fd placeholder, kept empty so poll() never reports
     *   it readable; reads are paced to 1 kHz so a tight polling loop in the
     *   app cannot spin.  Both apps poll the pins with plain 4-byte reads:
     *   EP122's GpioThread reads each input port every 2 ms, and EP145's
     *   TestMode reads UPD-Port from its message thread.  A blocking read
     *   parks either thread for good. --- */
    if (is_gpiodrv_fd(fd)) {
        static long last_ns;
        struct timespec now;
        clock_gettime(CLOCK_MONOTONIC, &now);
        long now_ns = (long)now.tv_sec * 1000000000L + now.tv_nsec;
        long prev = __atomic_exchange_n(&last_ns, now_ns, __ATOMIC_RELAXED);
        if (now_ns - prev < GPIODRV_MIN_READ_NS) {
            struct timespec pace = { 0, GPIODRV_MIN_READ_NS };
            nanosleep(&pace, NULL);
        }
        memset(buf, GPIODRV_STATE_BYTE, count);
        DBG("gpiodrv read fd=%d count=%zu -> 0x%02x\n",
            fd, count, GPIODRV_STATE_BYTE);
        return (ssize_t)count;
    }

    /* --- All other fds (including subucom_virt.ko): passthrough.  A frame
     *   off the sub-CPU also carries touch coordinates; on a model that
     *   reads a touch device they reach it as evdev records on its stub. --- */
    ssize_t r = sys_read(fd, buf, count);
    if (r == 64 && fd == __atomic_load_n(&g_subucom_fd, __ATOMIC_ACQUIRE)
        && deck_touch_publish)
        deck_touch_publish((const unsigned char *)buf, (size_t)r);
    return r;
}
