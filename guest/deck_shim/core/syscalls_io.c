// SPDX-License-Identifier: MIT OR Apache-2.0
/*
 * syscalls_io.c - the ioctl(2) and read(2) hooks. ioctl sends DRM fds to
 * handle_drm_ioctl (jog.c), answers the gpiodrv and hidg stubs itself, and
 * passes every other fd, including the subucom device, to the kernel.
 */

#include "deck_shim.h"

int ioctl(int fd, unsigned long request, ...) {
    va_list ap; va_start(ap, request);
    void *arg = va_arg(ap, void *);
    va_end(ap);

    if (is_drm_fd(fd))
        return handle_drm_ioctl(fd, request, arg);

    /* Accept every ioctl on a GPIODRV fd, such as a pin configuration. */
    if (is_gpiodrv_fd(fd)) {
        DBG("gpiodrv ioctl fd=%d request=0x%lx arg=%p\n", fd, (unsigned long)request, arg);
        return 0;
    }

    /* Accept every ioctl on a HIDG fd. f_hid's char device recognises only
     * a small set and returns ENOTTY for the other HID-class ioctls. The
     * app's USB gadget manager treats ENOTTY as a gadget initialisation
     * failure and may show "USB Error. Remove the device." Reads and writes
     * pass through to the real f_hid device; poll() reports the fd as
     * always writable (see syscalls.c). */
    if (is_hidg_fd(fd))
        return 0;

    /* Every other ioctl, including those for subucom_virt.ko, passes through. */
    return sys_ioctl(fd, request, arg);
}

/* ------------------------------------------------------------------ */
/* read                                                               */
/* ------------------------------------------------------------------ */
ssize_t read(int fd, void *buf, size_t count) {
    /* --- DRM fd: synthesise a DRM_EVENT_FLIP_COMPLETE when a flip is pending.
     *   The app's render loop calls drmHandleEvent(fd, ctx), which calls read()
     *   and expects a 32-byte drm_event_vblank. The real card0 fd has no flip
     *   events because the shim fakes SETPLANE. Return a synthetic event when
     *   inject_drm_flip_event() has left a flip pending; otherwise fail with
     *   EAGAIN, because the read is non-blocking and the caller polls first. */
    if (is_drm_fd(fd)) {
        /* Only an fd with a flip slot can have a pending flip. */
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

    /* --- GPIODRV fd: a pin-state read answers at once and never blocks. The
     *   backing pipe only provides the fd and stays empty, so poll() never
     *   reports it readable. Reads are paced to 1 kHz, so a tight polling loop
     *   in the app cannot spin. EP122 and EP145 poll the pins with plain
     *   4-byte reads: EP122's GpioThread reads each input port every 2 ms,
     *   and EP145's TestMode reads UPD-Port from its message thread. A
     *   blocking read would stall either thread for good. --- */
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
        DBG("gpiodrv read fd=%d count=%zu: returning 0x%02x\n",
            fd, count, GPIODRV_STATE_BYTE);
        return (ssize_t)count;
    }

    /* --- All other fds, including the subucom_virt.ko device, pass
     *   through. A sub-CPU frame also carries touch coordinates; on a model
     *   that reads a touch device, deck_touch_publish writes them as evdev
     *   records to the touch stub. --- */
    ssize_t r = sys_read(fd, buf, count);
    if (r > 0 && fd == __atomic_load_n(&g_subucom_fd, __ATOMIC_ACQUIRE)
        && deck_touch_publish)
        deck_touch_publish((const unsigned char *)buf, (size_t)r);
    return r;
}
