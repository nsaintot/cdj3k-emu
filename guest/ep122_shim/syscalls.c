/* SPDX-License-Identifier: MIT OR Apache-2.0 */
/* ------------------------------------------------------------------ */
/* Shared open logic - used by open(), open64(), openat()             */
/* ------------------------------------------------------------------ */

#include "ep122_shim.h"

static int do_open(const char *pathname, int flags, mode_t mode) {
    if (pathname) {
        /* /sys/module/rockchipdrm/parameters/vsync_time - Pioneer DRM module
         * parameter: monotonic timestamp (ms) updated on every vsync interrupt.
         * EP122's MainDisplayUpdateTimer polls it at ~30 Hz to pace rendering.
         * On vanilla kernel there is no rockchipdrm module; synthesise the file
         * by returning a memfd pre-filled with the current monotonic time in ms.
         * Each open() produces a fresh fd so repeated reads get fresh values. */
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

        /* Log /dev/ and /sys/ opens. */
        if (strncmp(pathname, "/dev/", 5) == 0 ||
            strncmp(pathname, "/sys/", 5) == 0) {
            DBG("open(%s, 0x%x)\n", pathname, flags);
        }

        /* /dev/dri/card* or /dev/dri/renderD* or /dev/mali* - open real driver,
         * track in g_drm_active[] so VERSION ioctl can be intercepted below. */
        if (strncmp(pathname, DRM_DEV_PREFIX, sizeof(DRM_DEV_PREFIX) - 1) == 0 ||
            strncmp(pathname, "/dev/mali", 9) == 0) {
            long fd = syscall(SYS_openat, AT_FDCWD, pathname, flags, (long)mode);
            if (fd < 0) { return -1; } /* errno already set by glibc syscall() */
            if (add_drm_fd((int)fd) < 0) {
                syscall(SYS_close, fd);
                errno = EMFILE; return -1;
            }
            DBG("open(%s) → real DRM fd %ld\n", pathname, fd);
            return (int)fd;
        }
        /* ALSA control/PCM: pass through to virtio_snd.ko (card 0, real kernel device). */
        /* ALSA sequencer: /dev/snd/seq
         * EP122 opens this O_RDWR|O_NONBLOCK to send MIDI to the USB host
         * via f_midi (client 24:0).  No f_midi gadget exists in QEMU, so we
         * stub the sequencer client lifecycle and discard all MIDI output. */
        if (strcmp(pathname, ALSA_SEQ_PATH) == 0) {
            if (g_seq_rd < 0) { errno = ENODEV; return -1; }
            int fd = sys_dup(g_seq_rd);
            if (fd < 0) return -1;
            if (add_seq_fd(fd) < 0) { sys_close(fd); errno = EMFILE; return -1; }
            DBG("open(%s) → fake ALSA seq fd %d\n", pathname, fd);
            return fd;
        }
        /* /dev/hidg0 - USB HID gadget: EP122 opens read+write separately.
         * We always give the pipe read-end; write() is intercepted below.
         * Reads return EAGAIN (no USB host); poll() forces POLLOUT for writes. */
        if (strcmp(pathname, HIDG_PATH) == 0) {
            if (g_hidg_rd < 0) { errno = ENODEV; return -1; }
            int fd = sys_dup(g_hidg_rd);
            if (fd < 0) return -1;
            if (add_hidg_fd(fd) < 0) { sys_close(fd); errno = EMFILE; return -1; }
            DBG("open(%s, 0x%x) → hidg stub fd %d\n", pathname, flags, fd);
            return fd;
        }
        /* /dev/gpiodrv - Pioneer GPIO device used to detect USB presence.
         * In QEMU virt the GPIO hardware does not exist → read() EFAULT.
         * Stub returns zeros (no GPIO pins active = no USB device). */
        if (strcmp(pathname, GPIODRV_PATH) == 0) {
            if (g_gpiodrv_rd < 0) { errno = ENODEV; return -1; }
            int fd = sys_dup(g_gpiodrv_rd);
            if (fd < 0) return -1;
            if (add_gpiodrv_fd(fd) < 0) { sys_close(fd); errno = EMFILE; return -1; }
            DBG("open(%s, 0x%x) → gpiodrv stub fd %d\n", pathname, flags, fd);
            return fd;
        }
        /* /sys/bus/usb/drivers/usb-storage/bind   (write "1-1:1.0" to bind)
         * /sys/bus/usb/drivers/usb-storage/unbind (write "1-1:1.0" to unbind)
         * /sys/bus/usb/drivers/usb/unbind          (for safe-eject)
         * EP122 writes the USB interface name to these sysfs entries to bind or
         * unbind the USB storage driver.  In QEMU virt there is no xHCI
         * controller so the real sysfs write would fail ENODEV causing EP122
         * to show "USB Error."  Return a /dev/null fd so writes succeed silently.
         * NOTE: never put glob chars (star slash) in comments - they close the block. */
        if (strncmp(pathname, "/sys/bus/usb/drivers/", 21) == 0 &&
            (strstr(pathname, "/bind") != NULL ||
             strstr(pathname, "/unbind") != NULL)) {
            int fd = sys_openat("/dev/null", O_WRONLY, 0);
            DBG("open(%s, 0x%x) → /dev/null stub (USB sysfs bind) fd %d\n",
                pathname, flags, fd);
            return fd;
        }
        /* /sys/class/thermal/thermal_zone0/temp - fake 40°C (40000 milli-°C).
         * EP122's OverheatingWatcher reads this periodically; without it EP122
         * logs an error every tick.  Return a one-shot pipe pre-seeded with
         * "40000\n" - EP122 opens a new fd each read, so re-seeding is correct. */
        if (strcmp(pathname, "/sys/class/thermal/thermal_zone0/temp") == 0) {
            int pfd[2];
            if (syscall(SYS_pipe2, pfd, O_CLOEXEC) < 0) return -1;
            sys_write(pfd[1], "40000\n", 6);
            syscall(SYS_close, pfd[1]);
            DBG("open(%s) → fake thermal temp fd %d\n", pathname, pfd[0]);
            return pfd[0];
        }
        /* /sys/class/backlight/ brightness writes - discard silently.
         * EP122 sets screen brightness; no backlight hw exists in QEMU. */
        if (strncmp(pathname, "/sys/class/backlight/", 21) == 0 &&
            strstr(pathname, "/brightness") != NULL) {
            int fd = sys_openat("/dev/null", O_WRONLY, 0);
            DBG("open(%s, 0x%x) → /dev/null stub (backlight) fd %d\n",
                pathname, flags, fd);
            return fd;
        }
    }
    return sys_openat(pathname, flags, mode);
}

/* ------------------------------------------------------------------ */
/* open / open64 / openat / openat64                                  */
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
    /* For absolute paths, dirfd is ignored - handle same as open() */
    if (pathname && pathname[0] == '/') {
        mode_t mode = 0;
        if (flags & O_CREAT) {
            va_list ap; va_start(ap, flags);
            mode = va_arg(ap, mode_t); va_end(ap);
        }
        return do_open(pathname, flags, mode);
    }
    /* Relative path - pass through */
    mode_t mode = 0;
    if (flags & O_CREAT) {
        va_list ap; va_start(ap, flags);
        mode = va_arg(ap, mode_t); va_end(ap);
    }
    long r = syscall(SYS_openat, dirfd, pathname, flags, mode);
    if (r < 0) { errno = (int)-r; return -1; }
    return (int)r;
}

int openat64(int dirfd, const char *pathname, int flags, ...) __attribute__((alias("openat")));
#pragma GCC diagnostic pop

/* ------------------------------------------------------------------ */
/* stat / lstat / fstatat / access / faccessat                        */
/* JUCE File::existsAsFile() calls stat() before open().  Intercept   */
/* vsync_time here so existsAsFile() returns true, then open() above  */
/* returns the memfd with the live timestamp.                         */
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
    if (r < 0) { errno = (int)-r; return -1; }
    return 0;
}

int lstat(const char *path, struct stat *buf) {
    if (path && strstr(path, "vsync_time") != NULL) return vsync_fake_stat(buf);
    long r = syscall(SYS_fstatat, AT_FDCWD, path, buf, AT_SYMLINK_NOFOLLOW);
    if (r < 0) { errno = (int)-r; return -1; }
    return 0;
}

int fstatat(int dirfd, const char *path, struct stat *buf, int flags) {
    if (path && strstr(path, "vsync_time") != NULL) return vsync_fake_stat(buf);
    long r = syscall(SYS_fstatat, dirfd, path, buf, flags);
    if (r < 0) { errno = (int)-r; return -1; }
    return 0;
}

int access(const char *path, int mode) {
    if (path && strstr(path, "vsync_time") != NULL) return 0;
    long r = syscall(SYS_faccessat, AT_FDCWD, path, mode, 0);
    if (r < 0) { errno = (int)-r; return -1; }
    return 0;
}

int faccessat(int dirfd, const char *path, int mode, int flags) {
    if (path && strstr(path, "vsync_time") != NULL) return 0;
    long r = syscall(SYS_faccessat, dirfd, path, mode, flags);
    if (r < 0) { errno = (int)-r; return -1; }
    return 0;
}

/* ------------------------------------------------------------------ */
/* close                                                              */
/* ------------------------------------------------------------------ */
/* Defined in ep122_shim_link.c. Returns 1 if the FD has a Pro DJ Link
 * packet pending in the delayed-send queue (in which case we must NOT
 * close yet - the link worker will close it after firing the packet). */
extern int ep122_link_intercept_close(int fd);

int close(int fd) {
    if (is_drm_fd(fd)) {
        remove_drm_fd(fd);
        flip_slot_free(fd);  /* release per-fd flip event state */
        DBG("close(DRM fd %d)\n", fd);
        return sys_close(fd);
    }
    if (is_seq_fd(fd)) {
        remove_seq_fd(fd);
        DBG("close(ALSA seq fd %d)\n", fd);
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
    /* Pro DJ Link delayed-send: defer the kernel close until our worker
     * has fired the queued packet on this FD. */
    if (ep122_link_intercept_close(fd)) {
        return 0;   /* tell EP122 close succeeded; worker will close later */
    }
    return sys_close(fd);
}

/* ------------------------------------------------------------------ */
/* ioctl                                                              */
/* ------------------------------------------------------------------ */
void *mmap(void *addr, size_t length, int prot, int flags, int fd, off_t offset) {
    void *ret = sys_mmap(addr, length, prot, flags, fd, offset);
    /* Log every MAP_SHARED mmap on a DRM fd so we can see ALL of EP122's buffer maps. */
    if (is_drm_fd(fd) && (flags & MAP_SHARED) && offset != 0) {
        DBG("mmap(drm_fd=%d offset=0x%llx len=%zu) → %s%p\n",
            fd, (unsigned long long)(uint64_t)(uintptr_t)offset, length,
            ret == MAP_FAILED ? "FAILED errno=" : "", ret);
    }
    /* Snoop EP122's DRM mmap: when it maps a jog GEM buffer (matched by mmap_offset
     * recorded in MAP_DUMB handler), record the returned ptr for PAGE_FLIP copies. */
    if (ret != MAP_FAILED && is_drm_fd(fd) && offset != 0) {
        for (int i = 0; i < g_jog_fb_count; i++) {
            if (g_jog_fbs[i].mmap_offset != 0 &&
                g_jog_fbs[i].mmap_offset == (uint64_t)(uintptr_t)offset) {
                g_jog_fbs[i].dma_ptr = ret;
                DBG("mmap jog fb_id=%u handle=%u offset=0x%llx → %p\n",
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
/* poll - force POLLIN|POLLOUT for PCM fds; intercept HIDG/seq        */
/* ------------------------------------------------------------------ */
/*
 * /dev/subucom_spi1.0 is now handled by subucom_virt.ko - poll on it
 * goes through the kernel's char device poll handler which always returns
 * POLLIN (no .poll handler = DEFAULT_POLLMASK).  No stub interception needed.
 *
 * NOTE: aarch64 (asm-generic syscall table) does NOT have SYS_poll.
 *       Only SYS_ppoll (= 73) is available.  raw_poll() abstracts this.
 */

/* raw_poll - portable poll-syscall wrapper (SYS_ppoll on aarch64) */
static long raw_poll(struct pollfd *fds, nfds_t nfds, int timeout_ms) {
#ifdef SYS_poll
    return syscall(SYS_poll, fds, nfds, timeout_ms);
#else
    /* aarch64: use ppoll(fds, nfds, tsp, NULL, 0) */
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
    /* Separate counts: fds that need FORCED readiness vs hidg fds.
     *
     * subucom/pcm: must be forced POLLIN|POLLOUT so EP122's tight
     *   850 Hz loop doesn't time out waiting for a real pipe event.
     *
     * hidg: must NOT be forced.  Forcing POLLOUT makes EP122's HID
     *   event loop spin at full CPU (poll returns immediately every
     *   iteration).  Instead, let the kernel's natural poll() block
     *   for the caller's timeout on the empty pipe - this simulates
     *   "no USB host connected, waiting for one."
     *
     * drm: the DRM pipe gets synthetic flip events written to g_drm_wr from
     *   inject_drm_flip_event() after each SETPLANE/PAGE_FLIP.  raw_poll()
     *   on the read-end returns POLLIN naturally when there is data.
     *   If the pipe is empty (e.g., first frame, or event consumed before
     *   poll), we fall through to raw_poll with a 33 ms cap so EP122
     *   retries rather than blocking indefinitely. */
    int hidg_n   = 0;
    int seq_n    = 0;   /* ALSA seq: POLLOUT-only (MIDI write, no incoming) */
    int drm_n    = 0;   /* DRM fds waiting POLLIN for flip events */
    if (fds)
        for (nfds_t i = 0; i < nfds; i++) {
            if (is_hidg_fd(fds[i].fd))         hidg_n++;
            else if (is_seq_fd(fds[i].fd))     seq_n++;
            else if (is_drm_fd(fds[i].fd) && (fds[i].events & POLLIN)) drm_n++;
        }

    int stub_n = hidg_n + seq_n + drm_n;

    /* Pure passthrough when no stub fds involved */
    if (!stub_n) {
        long r = raw_poll(fds, nfds, timeout);
        if (r < 0) { errno = (int)-r; return -1; }
        return (int)r;
    }

    /* DRM POLLIN poll: synthesise flip events while also polling all other fds.
     *
     * IMPORTANT: EP122's JUCE Message Thread polls the DRM fd AND the X11
     * socket in the same poll() call.  If we return synthetic DRM POLLIN via
     * an early return (skipping raw_poll), the X11 socket never becomes ready
     * from JUCE's perspective → JUCE never receives X11 paint events → main
     * LCD goes black after the first frame and the jog GEM buffer stays zero.
     *
     * Fix: always call raw_poll (with 0 ms timeout when a flip is already
     * pending so we don't block), then overlay synthetic DRM POLLIN on top. */
    if (drm_n && !hidg_n && !seq_n) {
        /* If any DRM fd has a pending flip, use 0-ms timeout so we don't
         * block waiting for real kernel events - we'll return immediately. */
        int flip_pending = 0;
        for (nfds_t i = 0; i < nfds; i++) {
            if (!is_drm_fd(fds[i].fd)) continue;
            jog_flip_slot_t *ps = flip_slot_for(fds[i].fd);
            if (ps && __atomic_load_n(&ps->pending, __ATOMIC_ACQUIRE))
                { flip_pending = 1; break; }
        }
        int cap = flip_pending ? 0 : ((timeout < 0 || timeout > 33) ? 33 : timeout);
        long r = raw_poll(fds, nfds, cap);
        if (r < 0) { errno = (int)-r; return -1; }
        /* Overlay synthetic DRM POLLIN for fds that have a pending flip.
         * OR-into revents so we don't clear real events from other fds. */
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

    /* hidg / seq stub branch:
     * - If any fd requests POLLOUT: pretend the USB gadget / MIDI port is
     *   always write-ready.  Rate-limit to ~1 kHz with a 1 ms sleep so we
     *   don't burn CPU spinning.
     * - If only POLLIN is requested: block on the real pipe (no host data
     *   expected - pipe stays empty).                                    */
    int has_pollout = 0;
    for (nfds_t i = 0; i < nfds; i++)
        if (is_hidg_fd(fds[i].fd) && (fds[i].events & POLLOUT))
            has_pollout = 1;

    if (has_pollout) {
        /* Rate-limit: 1 ms sleep ≈ 1 kHz USB HID max poll rate */
        if (timeout != 0) {
            struct timespec ts = { 0, 1000000L };
            syscall(SYS_nanosleep, &ts, NULL);
        }
        int cnt = 0;
        for (nfds_t i = 0; i < nfds; i++) {
            if (is_hidg_fd(fds[i].fd)) {
                fds[i].revents = (fds[i].events & POLLOUT) ? POLLOUT : 0;
                if (fds[i].revents) cnt++;
            }
        }
        return cnt;
    }
    /* Pure POLLIN: block for real timeout - no host data expected */
    long r = raw_poll(fds, nfds, timeout);
    if (r < 0) { errno = (int)-r; return -1; }
    for (nfds_t i = 0; i < nfds; i++)
        if (is_hidg_fd(fds[i].fd)) fds[i].revents = 0;
    return (int)r;
}
