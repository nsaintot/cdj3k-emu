/* SPDX-License-Identifier: MIT OR Apache-2.0 */
/* ------------------------------------------------------------------ */
/* Shared open logic for open(), open64() and openat()               */
/* ------------------------------------------------------------------ */

#include "deck_shim.h"

static int do_open(const char *pathname, int flags, mode_t mode) {
    if (pathname) {
        /* /sys/module/rockchipdrm/parameters/vsync_time is a parameter of Pioneer's
         * DRM module: a monotonic timestamp in ms, updated on every vsync interrupt.
         * The app's MainDisplayUpdateTimer polls it at about 30 Hz to pace rendering.
         * The vanilla kernel has no rockchipdrm module, so the shim synthesises the
         * file as a memfd that holds the current monotonic time in ms. Each open()
         * returns a new fd, so every read gets a current value. */
        if (strstr(pathname, "vsync_time") != NULL) {
            struct timespec ts;
            clock_gettime(CLOCK_MONOTONIC, &ts);
            uint64_t ms = (uint64_t)ts.tv_sec * 1000ULL +
                          (uint64_t)ts.tv_nsec / 1000000ULL;
            char buf[32];
            int len = snprintf(buf, sizeof(buf), "%llu\n", (unsigned long long)ms);
            int mfd = (int)syscall(SYS_memfd_create, "vsync_time", 0x0008 /*MFD_NOEXEC_SEAL*/);
            if (mfd >= 0) {
                syscall(SYS_write, mfd, buf, len);
                syscall(SYS_lseek, mfd, 0, SEEK_SET);
            }
            return mfd;
        }

        if (strncmp(pathname, "/dev/", 5) == 0 ||
            strncmp(pathname, "/sys/", 5) == 0) {
            DBG("open(%s, 0x%x)\n", pathname, flags);
        }

        /* Open /dev/dri/card*, /dev/dri/renderD* and /dev/mali* on the real
         * driver, and track the fd in g_drm_active[] so the ioctl hook can
         * answer DRM_IOCTL_VERSION. */
        if (strncmp(pathname, DRM_DEV_PREFIX, sizeof(DRM_DEV_PREFIX) - 1) == 0 ||
            strncmp(pathname, "/dev/mali", 9) == 0) {
            long fd = syscall(SYS_openat, AT_FDCWD, pathname, flags, (long)mode);
            if (fd < 0) { return -1; } /* glibc syscall() already set errno */
            if (add_drm_fd((int)fd) < 0) {
                syscall(SYS_close, fd);
                errno = EMFILE; return -1;
            }
            DBG("open(%s): opened the real DRM fd %ld\n", pathname, fd);
            return (int)fd;
        }
        /* ALSA control, PCM and sequencer devices reach the final sys_openat
         * unchanged. CONFIG_SND_SEQUENCER=y, so EP122's sequencer client
         * handshake reaches the real subsystem. */
        /* /dev/hidg0 is the USB HID gadget char device. The shim opens the real
         * f_hid device and registers the fd, so the ioctl hook can answer 0 instead
         * of f_hid's ENOTTY, which puts EP122's gadget manager into "USB Error". */
        if (strcmp(pathname, HIDG_PATH) == 0) {
            int fd = (int)sys_openat(pathname, flags, mode);
            if (fd < 0) return -1;
            if (add_hidg_fd(fd) < 0) {
                sys_close(fd);
                errno = EMFILE;
                return -1;
            }
            DBG("open(%s, 0x%x): opened the real f_hid fd %d and tracked it for the ioctl filter\n",
                pathname, flags, fd);
            return fd;
        }
        /* /proc/udev_usbg1 is Pioneer's USB-B connect node. Redirect it to the FIFO
         * that cdj3k-pc-link-bridge serves; EP122's detector reads connect and
         * disconnect tokens from it. The FIFO does not exist until the bridge runs,
         * so a failed open here means PC Link is off, and the detector retries on
         * its next poll. O_NONBLOCK is forced because the bridge holds the FIFO
         * open, and a blocking read would stall the detector between tokens. */
        if (strcmp(pathname, USBG_PROBE_PATH) == 0) {
            int fd = (int)sys_openat(USBG_FIFO_PATH, flags | O_NONBLOCK, mode);
            DBG("open(%s): redirected to %s, fd %d\n", pathname, USBG_FIFO_PATH, fd);
            return fd;
        }
        /* /dev/gpiodrv is Pioneer's GPIO device for detecting USB presence.
         * QEMU virt has no such hardware, so each open gets an fd backed by
         * the stub pipe, and read() reports GPIODRV_STATE_BYTE. */
        if (strcmp(pathname, GPIODRV_PATH) == 0) {
            if (g_gpiodrv_rd < 0) { errno = ENODEV; return -1; }
            int fd = sys_dup(g_gpiodrv_rd);
            if (fd < 0) return -1;
            if (add_gpiodrv_fd(fd) < 0) { sys_close(fd); errno = EMFILE; return -1; }
            DBG("open(%s, 0x%x): returning gpiodrv stub fd %d\n", pathname, flags, fd);
            return fd;
        }
        /* Some models read touch from an input node that QEMU virt does not
         * have; the model's touch code serves it from the sub-CPU frame. All
         * details of that device stay in the model directory. */
        if (deck_touch_owns_path && deck_touch_owns_path(pathname)) {
            int fd = deck_touch_open(flags);
            DBG("open(%s, 0x%x): returning touch stub fd %d\n", pathname, flags, fd);
            return fd;
        }
        /* subucom_virt.ko serves the sub-CPU device. Remember the fd so that
         * read() can take the touch coordinates out of the frames. */
        if (strcmp(pathname, SUBUCOM_SPI_PATH) == 0
            || strcmp(pathname, SUBUCOM_SPI_PATH_1500X) == 0) {
            int fd = sys_openat(pathname, flags, 0);
            if (fd >= 0) {
                __atomic_store_n(&g_subucom_fd, fd, __ATOMIC_RELEASE);
                DBG("open(%s, 0x%x): passed through, subucom fd %d\n",
                    pathname, flags, fd);
            }
            return fd;
        }
        /* /sys/bus/usb/drivers/usb-storage/bind   (write "1-1:1.0" to bind)
         * /sys/bus/usb/drivers/usb-storage/unbind (write "1-1:1.0" to unbind)
         * /sys/bus/usb/drivers/usb/unbind          (for safe eject)
         * The app writes the USB interface name to these sysfs entries to bind
         * or unbind the USB storage driver. QEMU virt has no xHCI controller,
         * so the real sysfs write would fail with ENODEV and the app would
         * show "USB Error." Return a /dev/null fd so the writes succeed.
         * Do not write a path glob such as a star followed by a slash in this
         * comment: it closes the comment. */
        if (strncmp(pathname, "/sys/bus/usb/drivers/", 21) == 0 &&
            (strstr(pathname, "/bind") != NULL ||
             strstr(pathname, "/unbind") != NULL)) {
            int fd = sys_openat("/dev/null", O_WRONLY, 0);
            DBG("open(%s, 0x%x): returning a /dev/null stub for the USB sysfs bind, fd %d\n",
                pathname, flags, fd);
            return fd;
        }
        /* /sys/class/thermal/thermal_zone0/temp reports a fixed 40 °C (40000
         * milli-°C). The app's OverheatingWatcher reads it periodically and
         * logs an error on every tick when it is missing. Return a one-shot
         * pipe that holds "40000\n"; the app opens a new fd for each read. */
        if (strcmp(pathname, "/sys/class/thermal/thermal_zone0/temp") == 0) {
            int pfd[2];
            if (syscall(SYS_pipe2, pfd, O_CLOEXEC) < 0) return -1;
            sys_write(pfd[1], "40000\n", 6);
            syscall(SYS_close, pfd[1]);
            DBG("open(%s): returning a fixed thermal temperature, fd %d\n", pathname, pfd[0]);
            return pfd[0];
        }
        /* Discard writes to the brightness file under /sys/class/backlight:
         * the app sets the screen brightness, and QEMU has no backlight. */
        if (strncmp(pathname, "/sys/class/backlight/", 21) == 0 &&
            strstr(pathname, "/brightness") != NULL) {
            int fd = sys_openat("/dev/null", O_WRONLY, 0);
            DBG("open(%s, 0x%x): returning a /dev/null stub for the backlight, fd %d\n",
                pathname, flags, fd);
            return fd;
        }
    }
    {
        return sys_openat(pathname, flags, mode);
    }
}

/* ------------------------------------------------------------------ */
/* open, open64, openat and openat64                                  */
/* ------------------------------------------------------------------ */
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wnonnull-compare"
int open(const char *pathname, int flags, ...) {
    mode_t mode = 0;
    if (flags & O_CREAT) {
        va_list ap; va_start(ap, flags);
        mode = va_arg(ap, mode_t); va_end(ap);
    }
    return do_open(pathname, flags, mode);
}

int open64(const char *pathname, int flags, ...) __attribute__((alias("open")));

int openat(int dirfd, const char *pathname, int flags, ...) {
    /* dirfd is ignored for an absolute path, so handle it like open(). */
    if (pathname && pathname[0] == '/') {
        mode_t mode = 0;
        if (flags & O_CREAT) {
            va_list ap; va_start(ap, flags);
            mode = va_arg(ap, mode_t); va_end(ap);
        }
        return do_open(pathname, flags, mode);
    }
    /* Pass a relative path through. */
    mode_t mode = 0;
    if (flags & O_CREAT) {
        va_list ap; va_start(ap, flags);
        mode = va_arg(ap, mode_t); va_end(ap);
    }
    long r = syscall(SYS_openat, dirfd, pathname, flags, mode);
    if (r < 0) return -1;   /* glibc syscall() already set errno */
    return (int)r;
}

int openat64(int dirfd, const char *pathname, int flags, ...) __attribute__((alias("openat")));
#pragma GCC diagnostic pop

/* ------------------------------------------------------------------ */
/* stat, lstat, fstatat, access and faccessat                         */
/* JUCE File::existsAsFile() calls stat() before open(). These hooks  */
/* report vsync_time as present, so existsAsFile() returns true and   */
/* open() above returns the memfd with the current timestamp.         */
/* ------------------------------------------------------------------ */

static int vsync_fake_stat(struct stat *buf) {
    memset(buf, 0, sizeof(*buf));
    buf->st_mode  = S_IFREG | 0444;
    buf->st_size  = 20;
    buf->st_nlink = 1;
    return 0;
}

int stat(const char *path, struct stat *buf) {
    if (path && strstr(path, "vsync_time") != NULL) return vsync_fake_stat(buf);
    long r = syscall(SYS_fstatat, AT_FDCWD, path, buf, 0);
    if (r < 0) return -1;   /* glibc syscall() already set errno */
    return 0;
}

int lstat(const char *path, struct stat *buf) {
    if (path && strstr(path, "vsync_time") != NULL) return vsync_fake_stat(buf);
    long r = syscall(SYS_fstatat, AT_FDCWD, path, buf, AT_SYMLINK_NOFOLLOW);
    if (r < 0) return -1;   /* glibc syscall() already set errno */
    return 0;
}

int fstatat(int dirfd, const char *path, struct stat *buf, int flags) {
    if (path && strstr(path, "vsync_time") != NULL) return vsync_fake_stat(buf);
    long r = syscall(SYS_fstatat, dirfd, path, buf, flags);
    if (r < 0) return -1;   /* glibc syscall() already set errno */
    return 0;
}

int access(const char *path, int mode) {
    if (path && strstr(path, "vsync_time") != NULL) return 0;
    long r = syscall(SYS_faccessat, AT_FDCWD, path, mode, 0);
    if (r < 0) return -1;   /* glibc syscall() already set errno */
    return 0;
}

int faccessat(int dirfd, const char *path, int mode, int flags) {
    if (path && strstr(path, "vsync_time") != NULL) return 0;
    long r = syscall(SYS_faccessat, dirfd, path, mode, flags);
    if (r < 0) return -1;   /* glibc syscall() already set errno */
    return 0;
}

/* ------------------------------------------------------------------ */
/* close                                                              */
/* ------------------------------------------------------------------ */
/* Defined in link.c. Returns 1 if the fd has a Pro DJ Link packet waiting in
 * the delayed-send queue. In that case the fd must stay open; the link worker
 * closes it after sending the packet. */
extern int shim_link_intercept_close(int fd);

int close(int fd) {
    if (is_drm_fd(fd)) {
        remove_drm_fd(fd);
        flip_slot_free(fd);
        DBG("close(DRM fd %d)\n", fd);
        return sys_close(fd);
    }
    if (is_hidg_fd(fd)) {
        remove_hidg_fd(fd);
        DBG("close(HIDG fd %d)\n", fd);
        return sys_close(fd);
    }
    if (is_gpiodrv_fd(fd)) {
        remove_gpiodrv_fd(fd);
        DBG("close(gpiodrv fd %d)\n", fd);
        return sys_close(fd);
    }
    if (deck_touch_is_fd && deck_touch_is_fd(fd)) {
        deck_touch_close(fd);
        DBG("close(touch fd %d)\n", fd);
        return sys_close(fd);
    }
    if (fd == __atomic_load_n(&g_subucom_fd, __ATOMIC_ACQUIRE))
        __atomic_store_n(&g_subucom_fd, -1, __ATOMIC_RELEASE);
    /* Defer the close until the link worker has sent the packet queued on
     * this fd. */
    if (shim_link_intercept_close(fd)) {
        return 0;   /* Report success; the worker closes the fd later. */
    }
    return sys_close(fd);
}

/* ------------------------------------------------------------------ */
/* mmap                                                               */
/* ------------------------------------------------------------------ */
void *mmap(void *addr, size_t length, int prot, int flags, int fd, off_t offset) {
    void *ret = sys_mmap(addr, length, prot, flags, fd, offset);
    /* Log every MAP_SHARED mmap on a DRM fd, to show all of the app's buffer mappings. */
    if (is_drm_fd(fd) && (flags & MAP_SHARED) && offset != 0) {
        DBG("mmap(drm_fd=%d offset=0x%llx len=%zu) returned %s%p\n",
            fd, (unsigned long long)(uint64_t)(uintptr_t)offset, length,
            ret == MAP_FAILED ? "FAILED errno=" : "", ret);
    }
    /* When the app maps a jog GEM buffer, matched by the mmap_offset that the
     * MAP_DUMB handler recorded, keep the returned pointer for the flip path. */
    if (ret != MAP_FAILED && is_drm_fd(fd) && offset != 0) {
        for (int i = 0; i < g_jog_fb_count; i++) {
            if (g_jog_fbs[i].mmap_offset != 0 &&
                g_jog_fbs[i].mmap_offset == (uint64_t)(uintptr_t)offset) {
                g_jog_fbs[i].dma_ptr = ret;
                DBG("mmap: jog fb_id=%u handle=%u offset=0x%llx mapped at %p\n",
                    g_jog_fbs[i].fb_id, g_jog_fbs[i].handle,
                    (unsigned long long)offset, ret);
            }
        }
    }
    return ret;
}

#ifdef __linux__
void *mmap64(void *addr, size_t length, int prot, int flags,
             int fd, off_t offset) __attribute__((alias("mmap")));
#endif

/* ------------------------------------------------------------------ */
/* poll: synthesise DRM flip readiness and hidg POLLOUT               */
/* ------------------------------------------------------------------ */
/*
 * subucom_virt.ko serves /dev/subucom_spi1.0, and its poll handler always
 * reports the device as readable. The shim does not intercept it.
 *
 * aarch64 uses the asm-generic syscall table, which has no SYS_poll, only
 * SYS_ppoll (73); raw_poll() hides the difference.
 */

/* A poll syscall wrapper that uses SYS_ppoll on aarch64. */
static long raw_poll(struct pollfd *fds, nfds_t nfds, int timeout_ms) {
#ifdef SYS_poll
    return syscall(SYS_poll, fds, nfds, timeout_ms);
#else
    /* aarch64: call ppoll(fds, nfds, tsp, NULL, 0). */
    if (timeout_ms < 0) {
        return syscall(SYS_ppoll, fds, nfds, NULL, NULL, 0);
    } else {
        struct timespec ts;
        ts.tv_sec  = timeout_ms / 1000;
        ts.tv_nsec = (timeout_ms % 1000) * 1000000L;
        return syscall(SYS_ppoll, fds, nfds, &ts, NULL, 0);
    }
#endif
}

int poll(struct pollfd *fds, nfds_t nfds, int timeout) {
    /* Count the hidg fds and the DRM fds that wait for POLLIN.
     *
     * hidg: POLLOUT is forced so EP122's writer never sees the gadget as
     *   stalled, and a 1 ms sleep bounds the loop at the USB HID report
     *   rate. POLLIN comes from the kernel: f_hid raises it when the host
     *   has queued an output report for /dev/hidg0.
     *
     * drm: inject_drm_flip_event() marks a flip as pending after an
     *   ATOMIC_COMMIT that asks for an event, or after such a PAGE_FLIP that
     *   the kernel rejects, and poll adds POLLIN for that fd. Without a pending flip, raw_poll runs with a
     *   33 ms cap, so the app retries instead of blocking indefinitely. */
    int hidg_n   = 0;
    int drm_n    = 0;   /* DRM fds that wait for POLLIN flip events */
    if (fds)
        for (nfds_t i = 0; i < nfds; i++) {
            if (is_hidg_fd(fds[i].fd))         hidg_n++;
            else if (is_drm_fd(fds[i].fd) && (fds[i].events & POLLIN)) drm_n++;
        }

    int stub_n = hidg_n + drm_n;

    /* Pass straight through when no stub fd is involved. */
    if (!stub_n) {
        long r = raw_poll(fds, nfds, timeout);
        if (r < 0) return -1;   /* glibc syscall() already set errno */
        return (int)r;
    }

    /* Synthesise flip events on the DRM fds and poll every other fd too.
     *
     * The app's JUCE message thread polls the DRM fd and the X11 socket in
     * the same poll() call. An early return with a synthetic DRM POLLIN would
     * hide the X11 socket from JUCE, so JUCE would receive no paint events,
     * the main LCD would go black after the first frame, and the jog GEM
     * buffer would stay zero. So always call raw_poll, with a 0 ms timeout
     * when a flip is already pending, then add the synthetic DRM POLLIN. */
    if (drm_n && !hidg_n) {
        /* If any DRM fd has a pending flip, poll with a 0 ms timeout so the
         * call returns at once. */
        int flip_pending = 0;
        for (nfds_t i = 0; i < nfds; i++) {
            if (!is_drm_fd(fds[i].fd)) continue;
            jog_flip_slot_t *ps = flip_slot_for(fds[i].fd);
            if (ps && __atomic_load_n(&ps->pending, __ATOMIC_ACQUIRE))
                { flip_pending = 1; break; }
        }
        int cap = flip_pending ? 0 : ((timeout < 0 || timeout > 33) ? 33 : timeout);
        long r = raw_poll(fds, nfds, cap);
        if (r < 0) return -1;   /* glibc syscall() already set errno */
        /* Add POLLIN for each DRM fd with a pending flip. OR it into
         * revents, so the real events stay. */
        int total = (int)r;
        for (nfds_t i = 0; i < nfds; i++) {
            if (!is_drm_fd(fds[i].fd)) continue;
            jog_flip_slot_t *ps = flip_slot_for(fds[i].fd);
            if (ps && __atomic_load_n(&ps->pending, __ATOMIC_ACQUIRE) &&
                (fds[i].events & POLLIN) && !(fds[i].revents & POLLIN)) {
                fds[i].revents |= POLLIN;
                total++;
            }
        }
        return total;
    }

    /* hidg fds:
     * - If any fd requests POLLOUT, report the USB gadget as always ready
     *   to write, and limit the loop to about 1 kHz with a 1 ms sleep.
     * - If only POLLIN is requested, poll with the caller's timeout; f_hid
     *   raises POLLIN when the host queues an output report.             */
    int has_pollout = 0;
    for (nfds_t i = 0; i < nfds; i++)
        if (is_hidg_fd(fds[i].fd) && (fds[i].events & POLLOUT))
            has_pollout = 1;

    /* Limit the writer loop: POLLOUT is synthesised below, so without this
     * sleep the caller would spin as fast as it can issue reports. 1 ms is
     * the maximum USB HID report rate. */
    if (has_pollout && timeout != 0) {
        struct timespec ts = { 0, 1000000L };
        syscall(SYS_nanosleep, &ts, NULL);
    }

    /* Let the kernel answer for every fd. With POLLOUT synthesised there is
     * nothing to wait for, so poll with 0 ms; otherwise use the caller's
     * timeout. f_hid raises POLLIN on /dev/hidg0 when the host has queued an
     * output report, which is the only signal EP122's HID receive thread
     * gets, so the kernel's revents pass through unchanged. */
    long r = raw_poll(fds, nfds, has_pollout ? 0 : timeout);
    if (r < 0) return -1;   /* glibc syscall() already set errno */
    int total = (int)r;

    /* The app's gadget manager treats a hidg fd that never reports POLLOUT as
     * a stalled gadget, so grant POLLOUT unconditionally. OR it into revents,
     * so a POLLIN that the kernel set in the same call stays. */
    if (has_pollout) {
        for (nfds_t i = 0; i < nfds; i++) {
            if (!is_hidg_fd(fds[i].fd)) continue;
            if (!(fds[i].events & POLLOUT)) continue;
            if (fds[i].revents == 0) total++;
            fds[i].revents |= POLLOUT;
        }
    }
    return total;
}
