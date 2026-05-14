// SPDX-License-Identifier: MIT OR Apache-2.0
/*
 * ep122_shim.c - LD_PRELOAD shim for /dev/dri/card* and peripheral devices
 *
 * Intercepts: open, open64, close, ioctl, read, write, mmap, mmap64, poll
 * No dlsym/libdl - all passthroughs use raw syscall() (GLIBC_2.17).
 * No pthread - fd tables use lockless atomics (see ep122_shim_fd.c).
 *
 * --- subucom_spi ioctl sequence ---
 *
 *   open("/dev/subucom_spi1.0", O_RDWR)     → real dup'd pipe fd
 *   ioctl(fd, 0x80027003, ...)              → timer_status read (2 bytes)
 *   ioctl(fd, 0x80017001, ...)              → bits_per_word read (1 byte)
 *   ioctl(fd, 0x40017001, ...)              → bits_per_word write / timer start
 *   read(fd, buf, 64)                       → MISO idle frame × N
 *   close(fd)
 *
 * EP122 main loop (~850 Hz):
 *   ioctl(fd, 0x40107000, mosi_16bytes)     → MOSI data transfer (ignored)
 *   read(fd, buf, 64)                       → MISO frame (we return idle)
 *
 * --- DRM/Jog-LCD ioctl sequence (JogLcdDRM::JogLcdDRM constructor) ---
 *
 *   open("/dev/dri/card0", O_RDWR)          → real dup'd pipe fd
 *   ioctl(fd, DRM_IOCTL_VERSION, ...)       → return "rockchip" v1.4
 *   ioctl(fd, DRM_IOCTL_SET_CLIENT_CAP, .) → return 0
 *   ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES) → return 0 connectors
 *   close(fd)
 *
 * MISO idle frame (64 bytes) - confirmed from hardware RE:
 * NOTE: All byte offsets ("bXX") are DECIMAL.  b16 = byte 16, NOT 0x16 (22).
 *   b02-b04: 0x01 0x04 0x03  - header
 *   b05: transport (play/cue/search/track/beatJump)
 *   b06: tempo/sync (reset/masterTempo/range/keySync/beatSync/master)
 *   b07: loop (in/out/reloop/beatloop½/beatloop2x/slip)
 *   b08: memory (mem/del/callPrev/callNext/callDelete)
 *   b09: hot cues A-H (each bit = one cue)
 *   b10: menu nav (source/browse/tagList/playlist/search/menu) + jogMode
 *   b11: menu nav (back/tagTrack/filter/shortcut/rotaryPress) + quantize/timeMode
 *   b12: 0x81 = device state bitmask (0x80=power_on | 0x01=sdcard_closed)
 *   b14-b15: rotary encoder counter (16-bit LE)
 *   b16-b17: LCD touch X (16-bit LE, 0x0000=no touch)
 *   b18-b19: LCD touch Y (16-bit LE, 0x0000=no touch)
 *   b22-b23: tempo slider (16-bit BE, range 0x0000-0xcfff)
 *   b24: vinyl speed rotary (0x00-0xff)
 *   b26-b27: jog position counter (16-bit LE, wrapping)
 *   b28-b29: jog velocity (16-bit BE, inverse: 0xffff=slow, 0x0000=fast)
 *   b30: jog touch state (0x00=none, 0x03=press, 0x04=touch, 0x0c=turning)
 *   b34-b43: capacitive sensor baseline
 *   All buttons/axes: 0x00
 */

#include "ep122_shim.h"

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

int g_hidg_rd = -1;
int g_hidg_wr = -1;
int g_hidg_active[MAX_HIDG_FDS];

int g_gpiodrv_rd = -1;
int g_gpiodrv_wr = -1;
int g_gpiodrv_active[MAX_GPIODRV_FDS];

int g_drm_rd = -1;
int g_drm_wr = -1;
int g_seq_rd = -1;
int g_seq_wr = -1;

int g_drm_active[MAX_ACTIVE_FDS];
int g_seq_active[MAX_ALSA_FDS];

/* Shared by clock.c and link.c. Raw-syscall sysfs reader so we don't
 * pull stdio into the audio hot path. Parses the LAST integer in the
 * line (handles both `42\n` and CSV `g,h,t\n`). */
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
/* Constructor - create pipe pairs for DRM / ALSA / GPIO / HIDG       */
/* ------------------------------------------------------------------ */
/* NOTE: /dev/subucom_spi1.0 is provided by subucom_virt.ko.          */
/*       EP122 opens it directly; this stub no longer intercepts it.  */

static void fds_init(void) __attribute__((constructor));
static void fds_init(void) {
    /* Check SUBUCOM_DEBUG=1 to enable verbose logging */
    const char *dbg_env = getenv("SUBUCOM_DEBUG");
    if (dbg_env && dbg_env[0] == '1') g_debug = 1;

    /* Initialize active fd tables to "empty" (-1) */
    for (int i = 0; i < MAX_ACTIVE_FDS; i++) {
        g_drm_active[i] = -1;
    }
    for (int i = 0; i < MAX_ALSA_FDS; i++) {
        g_seq_active[i] = -1;
    }

    int pfds[2];

    /* DRM pipe - NOT pre-filled.  EP122 should not poll this after
     * receiving 0 connectors from DRM_IOCTL_MODE_GETRESOURCES. */
    if (syscall(SYS_pipe2, pfds, O_NONBLOCK) == 0) {
        g_drm_rd = pfds[0];
        g_drm_wr = pfds[1];
        DBG("DRM pipe: rd=%d wr=%d\n", g_drm_rd, g_drm_wr);
    }

    /* ALSA sequencer pipe - O_NONBLOCK; read→EAGAIN, write→discarded. */
    if (syscall(SYS_pipe2, pfds, O_NONBLOCK) == 0) {
        g_seq_rd = pfds[0];
        g_seq_wr = pfds[1];
        DBG("ALSA seq pipe: rd=%d wr=%d\n", g_seq_rd, g_seq_wr);
    }

    /* HIDG pipe - not pre-filled; read→0 (no host), write→discarded */
    for (int i = 0; i < MAX_HIDG_FDS; i++) g_hidg_active[i] = -1;
    if (syscall(SYS_pipe2, pfds, O_NONBLOCK) == 0) {
        g_hidg_rd = pfds[0];
        g_hidg_wr = pfds[1];
        DBG("HIDG pipe: rd=%d wr=%d (USB HID gadget stub ready)\n",
            g_hidg_rd, g_hidg_wr);
    }

    /* GPIODRV stub - blocking pipe (NO O_NONBLOCK), write-end kept open. */
    for (int i = 0; i < MAX_GPIODRV_FDS; i++) g_gpiodrv_active[i] = -1;
    if (syscall(SYS_pipe2, pfds, 0) == 0) {   /* 0 = blocking mode */
        g_gpiodrv_rd = pfds[0];
        g_gpiodrv_wr = pfds[1];
        static const uint8_t no_dev[4] = { 0xFF, 0xFF, 0xFF, 0xFF };
        for (int k = 0; k < 4; k++)
            syscall(SYS_write, g_gpiodrv_wr, no_dev, 4);
        DBG("gpiodrv pipe: rd=%d wr=%d (blocking, 4×0xFF seeds - no USB device)\n",
            g_gpiodrv_rd, g_gpiodrv_wr);
    }

    /* ---- Jog LCD framebuffer - tagged anon mmap visible in shared RAM ---- */
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
