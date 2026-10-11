// SPDX-License-Identifier: MIT OR Apache-2.0
/*
 * deck_shim.c - the LD_PRELOAD shim for the DRM devices and the deck's
 * peripheral devices.
 *
 * The syscall interposers live in syscalls.c and syscalls_io.c, the clock
 * hooks in clock.c, the Pro DJ Link send hooks in link.c, and the libdrm entry
 * points in jog.c and jog_drm.c. The shim uses neither dlsym nor libdl, to
 * stay within GLIBC_2.17: passthroughs call syscall() directly, and clock.c
 * reads the clock through the vDSO. The fd tables use lockless atomics (see
 * fd.c).
 *
 * subucom_virt.ko serves /dev/subucom_spi1.0; the shim does not intercept it.
 *
 * The jog LCD setup in the app's JogLcdDRM constructor, and what the shim
 * returns for each call:
 *
 *   | Call                                   | Result                       |
 *   |----------------------------------------|------------------------------|
 *   | open("/dev/dri/card0", O_RDWR)         | the real DRM fd, now tracked |
 *   | ioctl(fd, DRM_IOCTL_VERSION, ...)      | "rockchip" v1.4              |
 *   | ioctl(fd, DRM_IOCTL_SET_CLIENT_CAP, .) | 0                            |
 *   | ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES) | the kernel's answer          |
 *
 * The MISO field map is in crates/cdj3k-emu-panel/src/miso_frame.rs (fields)
 * and cdj3k.rs (IDLE).
 */

#include "deck_shim.h"

/* ------------------------------------------------------------------ */
/* Global variable definitions                                        */
/* ------------------------------------------------------------------ */

int g_debug = 0;

int g_jog_prime_fd = -1;

jog_fb_entry_t g_jog_fbs[JOG_MAX_FBS];
int    g_jog_fb_count  = 0;
void  *g_jog_shm_base   = NULL;
void  *g_jog_shm_pixels = NULL;
void  *g_jog_current_dma_ptr = NULL;

uint8_t *g_jog_fb = NULL;

/* The open() interposer gives EP122 a real f_hid char-device fd. The shim
 * keeps this table so that ioctl() can answer 0 instead of f_hid's ENOTTY,
 * which EP122's gadget manager treats as a fatal init failure (see
 * syscalls_io.c), and so that poll() can report the fd as writable. */
int g_hidg_active[MAX_HIDG_FDS];

int g_gpiodrv_rd = -1;
int g_gpiodrv_wr = -1;
int g_gpiodrv_active[MAX_GPIODRV_FDS];

int g_subucom_fd = -1;

int g_drm_rd = -1;
int g_drm_wr = -1;

int g_drm_active[MAX_ACTIVE_FDS];

/* A sysfs reader for clock.c and link.c that uses raw syscalls, so stdio
 * stays out of the audio hot path. It returns the last integer in the line,
 * so it handles both `42\n` and the CSV `g,h,t\n`. */
long shim_read_sysfs_long(const char *path)
{
    long fd = syscall(SYS_openat, AT_FDCWD, path,
                      O_RDONLY | O_CLOEXEC, (long)0);
    if (fd < 0) return -1;
    char buf[16] = {0};
    long n = syscall(SYS_read, (long)fd, (long)buf, (long)(sizeof(buf) - 1));
    syscall(SYS_close, (long)fd);
    if (n <= 0) return -1;
    long v = -1, cur = 0;
    int in_num = 0;
    for (long i = 0; i < n; i++) {
        char c = buf[i];
        if (c >= '0' && c <= '9') {
            cur = cur * 10 + (c - '0');
            in_num = 1;
        } else if (c == ',' || c == ' ') {
            if (in_num) { v = cur; cur = 0; in_num = 0; }
        } else if (c == '\n' || c == '\0') {
            if (in_num) v = cur;
            break;
        } else {
            return -1;
        }
    }
    return v;
}

/* ------------------------------------------------------------------ */
/* Constructor: set up the fd tables and the DRM and GPIO pipes       */
/* ------------------------------------------------------------------ */
/* subucom_virt.ko provides /dev/subucom_spi1.0. The app opens it     */
/* directly, and the shim does not intercept it.                      */

static void fds_init(void) __attribute__((constructor));
static void fds_init(void) {
    /* SUBUCOM_DEBUG=1 enables verbose logging. */
    const char *dbg_env = getenv("SUBUCOM_DEBUG");
    if (dbg_env && dbg_env[0] == '1') g_debug = 1;

    for (int i = 0; i < MAX_ACTIVE_FDS; i++) {
        g_drm_active[i] = -1;
    }
    for (int i = 0; i < MAX_HIDG_FDS; i++) g_hidg_active[i] = -1;

    int pfds[2];

    /* The DRM pipe starts empty. */
    if (syscall(SYS_pipe2, pfds, O_NONBLOCK | O_CLOEXEC) == 0) {
        g_drm_rd = pfds[0];
        g_drm_wr = pfds[1];
        DBG("DRM pipe: rd=%d wr=%d\n", g_drm_rd, g_drm_wr);
    }

    /* The GPIODRV pipe only backs the fds that open("/dev/gpiodrv") returns.
     * read() answers from the pin state, so the pipe stays empty. */
    for (int i = 0; i < MAX_GPIODRV_FDS; i++) g_gpiodrv_active[i] = -1;
    if (syscall(SYS_pipe2, pfds, O_CLOEXEC) == 0) {
        g_gpiodrv_rd = pfds[0];
        g_gpiodrv_wr = pfds[1];
        DBG("gpiodrv pipe: rd=%d wr=%d\n", g_gpiodrv_rd, g_gpiodrv_wr);
    }

    /* Map the jog LCD framebuffer anonymously behind a magic header, so the
     * host can find it in guest RAM. */
    {
        size_t total = (size_t)JOG_FB_HDR_SIZE + (size_t)JOG_FB_PX_SIZE;
        void *p = sys_mmap(NULL, total, PROT_READ | PROT_WRITE,
                           MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        if (p != MAP_FAILED) {
            memcpy(p, JOG_FB_MAGIC, 8);
            uint64_t px_va = (uint64_t)(uintptr_t)p + JOG_FB_HDR_SIZE;
            memcpy((uint8_t *)p + 8, &px_va, 8);
            memset((uint8_t *)p + JOG_FB_HDR_SIZE, 0, (size_t)JOG_FB_PX_SIZE);
            g_jog_fb = (uint8_t *)p + JOG_FB_HDR_SIZE;
            DBG("jog FB: %u bytes at %p (magic hdr at %p)\n",
                JOG_FB_PX_SIZE, g_jog_fb, p);
        } else {
            DBG("jog FB: anon mmap failed errno=%d\n", errno);
        }
    }
}
