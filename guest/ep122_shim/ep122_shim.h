/* SPDX-License-Identifier: MIT OR Apache-2.0 */
#ifndef EP122_SHIM_H
#define EP122_SHIM_H

/* ------------------------------------------------------------------ */
/* System includes                                                    */
/* ------------------------------------------------------------------ */

#define _GNU_SOURCE
#include <stdint.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
/* musl declares ioctl(int, int, ...) but we need unsigned long for request.
 * Suppress the prototype so our definition below does not conflict. */
#define ioctl __musl_ioctl_hidden
#include <sys/ioctl.h>
#undef ioctl
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/types.h>
#ifndef SYS_pipe2
#define SYS_pipe2 59   /* aarch64 Linux - not in macOS headers */
#endif
#ifndef SYS_memfd_create
#define SYS_memfd_create 279  /* aarch64 Linux */
#endif
#ifndef SYS_fstatat
#define SYS_fstatat 79   /* aarch64 Linux: newfstatat */
#endif
#ifndef SYS_faccessat
#define SYS_faccessat 48  /* aarch64 Linux */
#endif
#include <time.h>
#include <poll.h>
#include <sched.h>

/* ------------------------------------------------------------------ */
/* Configuration macros                                               */
/* ------------------------------------------------------------------ */

#define DRM_DEV_PREFIX    "/dev/dri/"

/* Maximum simultaneous opens we track */
#define MAX_ACTIVE_FDS    8

/* ------------------------------------------------------------------ */
/* Debug macro                                                        */
/* ------------------------------------------------------------------ */

#define DBG(...) do { \
    if (g_debug) { \
        fprintf(stderr, "[ep122_shim] " __VA_ARGS__); \
        fflush(stderr); \
    } \
} while(0)

/* ------------------------------------------------------------------ */
/* Known ioctl codes                                                  */
/* ------------------------------------------------------------------ */

#define IOCTL_TIMER_STATUS_READ   0x80027003u  /* _IOR('p',3,u16) */
#define IOCTL_TIMER_STATUS_WRITE  0x40027003u  /* _IOW('p',3,u16) - EP122 SubCpu::setBits */
#define IOCTL_BITS_PER_WORD_READ  0x80017001u  /* _IOR('p',1,u8)  */
#define IOCTL_BITS_PER_WORD_WRITE 0x40017001u  /* _IOW('p',1,u8)  */
#define IOCTL_MOSI_TRANSFER       0x40107000u  /* _IOW('p',0,16bytes) */
#define IOCTL_RX_BYTES_READ       0x80047004u  /* _IOR('p',4,u32) */
#define IOCTL_RX_BYTES_WRITE      0x40047004u  /* _IOW('p',4,u32) */
#define IOCTL_INTERVAL_READ       0x80047002u  /* _IOR('p',2,u32) */
#define IOCTL_INTERVAL_WRITE      0x40047002u  /* _IOW('p',2,u32) */

#define DRM_IOCTL_TYPE            0x64u
#define DRM_CMD_VERSION           0x00u
#define DRM_CMD_SET_CLIENT_CAP    0x0Du

/* DRM KMS modesetting commands (cmd byte of ioctl request) */
#define DRM_CMD_MODE_GETRESOURCES 0xA0u
#define DRM_CMD_MODE_GETCRTC      0xA1u
#define DRM_CMD_MODE_SETCRTC      0xA2u
#define DRM_CMD_MODE_GETENCODER   0xA6u
#define DRM_CMD_MODE_GETCONNECTOR 0xA7u
#define DRM_CMD_MODE_ADDFB        0xAEu
#define DRM_CMD_MODE_RMFB         0xAFu
#define DRM_CMD_MODE_PAGE_FLIP    0xB0u
#define DRM_CMD_MODE_CREATE_DUMB  0xB2u
#define DRM_CMD_MODE_MAP_DUMB     0xB3u
#define DRM_CMD_MODE_ADDFB2       0xB8u
#define DRM_CMD_MODE_ATOMIC       0xBCu

/* Fake KMS object IDs */
#define FAKE_CRTC_ID      1u
#define FAKE_CONNECTOR_ID 1u
#define FAKE_ENCODER_ID   1u
#define FAKE_FB_ID        1u
#define FAKE_DUMB_HANDLE  1u

/* CDJ-3000 jog LCD (DSI-2): 1280×240 @ 71 Hz */
#define FAKE_W         1280u
#define FAKE_H         240u
#define FAKE_BPP       32u
#define FAKE_PITCH     (FAKE_W * (FAKE_BPP / 8))  /* 5120 bytes/row */
#define FAKE_CLK_KHZ   25844u
#define FAKE_HTOTAL    1400u
#define FAKE_VTOTAL    260u

#define DRM_MODE_LEN   32

/* Jog LCD framebuffer sharing */
#define JOG_FB_MAGIC    "JOGFBUF\x00"   /* 8 bytes - scanner target */
#define JOG_FB_HDR_SIZE 16u             /* 8 magic + 8 VA of pixel region */
#define JOG_FB_PX_SIZE  (FAKE_W * FAKE_H * (FAKE_BPP / 8u))  /* 1,228,800 bytes */

#define JOG_DMA_SIZE   (FAKE_W * FAKE_H * 4u)   /* 1,228,800 bytes - source DRM buffer */
#define JOG_OUT_W      320u
#define JOG_OUT_H      240u
#define JOG_OUT_BYTES  (JOG_OUT_W * JOG_OUT_H * 4u)  /* 307,200 bytes - extracted XRGB8888 */
#define JOG_MAX_FBS    4

/* ivshmem (cdj3k_jog) shared-memory layout - must match host crates/cdj3k-emu-streams.
 *   0x0000  u32 magic    'JOG1' little-endian = 0x31474F4A
 *   0x0004  u32 seq      seqlock counter (odd = write in progress)
 *   0x0008  u16 width
 *   0x000A  u16 height
 *   0x000C  u32 format   1 = XRGB8888
 *   0x1000  pixels       width*height*4 bytes
 */
#define JOG_SHM_BAR_SIZE     (1u << 20)   /* 1 MiB ivshmem BAR2 */
#define JOG_SHM_MAGIC        0x31474F4Au
#define JOG_SHM_FMT_XRGB8888 1u
#define JOG_SHM_PIXELS_OFF   0x1000u
#define JOG_SHM_OFF_MAGIC    0x0000u
#define JOG_SHM_OFF_SEQ      0x0004u
#define JOG_SHM_OFF_W        0x0008u
#define JOG_SHM_OFF_H        0x000Au
#define JOG_SHM_OFF_FMT      0x000Cu

typedef struct {
    uint32_t fb_id;
    uint32_t handle;
    uint64_t mmap_offset;
    void    *dma_ptr;
} jog_fb_entry_t;

/* ------------------------------------------------------------------ */
/* ALSA sequencer stub (audio PCM/CTL paths use the kernel virtio_snd  */
/* driver directly - no shim interception.)                             */
/* ------------------------------------------------------------------ */

#define ALSA_SEQ_PATH    "/dev/snd/seq"

#define ALSA_SEQ_IOCTL_PVERSION    0x80045300u
#define ALSA_SEQ_IOCTL_CLIENT_ID   0x80045301u

#define MAX_ALSA_FDS  16

/* ------------------------------------------------------------------ */
/* USB HID Gadget stub (/dev/hidg0)                                   */
/* ------------------------------------------------------------------ */

#define HIDG_PATH    "/dev/hidg0"
#define MAX_HIDG_FDS 8

/* ------------------------------------------------------------------ */
/* GPIO device stub (/dev/gpiodrv)                                    */
/* ------------------------------------------------------------------ */

#define GPIODRV_PATH    "/dev/gpiodrv"
#define MAX_GPIODRV_FDS 4

/* ------------------------------------------------------------------ */
/* Syscall helpers - no dlsym, no libdl, GLIBC_2.17 only              */
/* ------------------------------------------------------------------ */

static inline int sys_openat(const char *path, int flags, mode_t mode) {
    long r = syscall(SYS_openat, AT_FDCWD, path, flags, mode);
    if (r < 0) { errno = (int)-r; return -1; }
    return (int)r;
}
static inline int sys_close(int fd) {
    long r = syscall(SYS_close, fd);
    if (r < 0) { errno = (int)-r; return -1; }
    return 0;
}
static inline int sys_ioctl(int fd, unsigned long req, void *arg) {
    long r = syscall(SYS_ioctl, fd, req, arg);
    if (r < 0) { errno = (int)-r; return -1; }
    return (int)r;
}
static inline ssize_t sys_read(int fd, void *buf, size_t n) {
    long r = syscall(SYS_read, fd, buf, n);
    if (r < 0) { errno = (int)-r; return -1; }
    return (ssize_t)r;
}
static inline ssize_t sys_write(int fd, const void *buf, size_t n) {
    long r = syscall(SYS_write, fd, buf, n);
    if (r < 0) { errno = (int)-r; return -1; }
    return (ssize_t)r;
}
static inline int sys_dup(int fd) {
    long r = syscall(SYS_dup, fd);
    if (r < 0) { errno = (int)-r; return -1; }
    return (int)r;
}
static inline void *sys_mmap(void *addr, size_t len, int prot, int flags,
                               int fd, off_t off) {
    long r = syscall(SYS_mmap, addr, len, prot, flags, fd, off);
    if (r < 0) { errno = (int)-r; return MAP_FAILED; }
    return (void *)r;
}

/* ------------------------------------------------------------------ */
/* Struct/typedef definitions                                         */
/* ------------------------------------------------------------------ */

typedef struct {
    uint32_t clock;
    uint16_t hdisplay, hsync_start, hsync_end, htotal, hskew;
    uint16_t vdisplay, vsync_start, vsync_end, vtotal, vscan;
    uint32_t vrefresh;
    uint32_t flags;
    uint32_t type;
    char     name[DRM_MODE_LEN];
} fake_modeinfo_t;

typedef struct {
    uint64_t fb_id_ptr;
    uint64_t crtc_id_ptr;
    uint64_t connector_id_ptr;
    uint64_t encoder_id_ptr;
    uint32_t count_fbs;
    uint32_t count_crtcs;
    uint32_t count_connectors;
    uint32_t count_encoders;
    uint32_t min_width,  max_width;
    uint32_t min_height, max_height;
} fake_mode_res_t;

typedef struct {
    uint64_t encoders_ptr;
    uint64_t modes_ptr;
    uint64_t props_ptr;
    uint64_t prop_values_ptr;
    uint32_t count_modes;
    uint32_t count_props;
    uint32_t count_encoders;
    uint32_t encoder_id;
    uint32_t connector_id;
    uint32_t connector_type;
    uint32_t connector_type_id;
    uint32_t connection;
    uint32_t mm_width, mm_height;
    uint32_t subpixel;
    uint32_t pad;
} fake_mode_connector_t;

typedef struct {
    uint32_t encoder_id;
    uint32_t encoder_type;
    uint32_t crtc_id;
    uint32_t possible_crtcs;
    uint32_t possible_clones;
} fake_mode_encoder_t;

typedef struct {
    uint64_t    set_connectors_ptr;
    uint32_t    count_connectors;
    uint32_t    crtc_id;
    uint32_t    fb_id;
    uint32_t    x, y;
    uint32_t    gamma_size;
    uint32_t    mode_valid;
    fake_modeinfo_t mode;
} fake_mode_crtc_t;

typedef struct {
    uint32_t height, width, bpp, flags;
    uint32_t handle, pitch;
    uint64_t size;
} fake_mode_create_dumb_t;

typedef struct {
    uint32_t handle, pad;
    uint64_t offset;
} fake_mode_map_dumb_t;

typedef struct {
    uint32_t fb_id;
    uint32_t width, height, pitch, bpp, depth;
    uint32_t handle;
} fake_mode_addfb_t;

/* Per-fd flip event state */
#define MAX_JOG_RENDER_FDS 4
typedef struct {
    int              fd;
    volatile int     pending;
    volatile uint64_t user_data;
} jog_flip_slot_t;

/* libdrm drmEventContext (fake_event_ctx_t) */
typedef struct {
    int version;
    void (*vblank_handler)(int, unsigned int, unsigned int, unsigned int, void *);
    void (*page_flip_handler)(int, unsigned int, unsigned int, unsigned int, void *);
    void (*page_flip_handler2)(int, unsigned int, unsigned int, unsigned int, unsigned int, void *);
    void (*sequence_handler)(int, uint64_t, uint64_t, uint64_t);
} fake_event_ctx_t;

/* libdrm API types */
typedef struct {
    int   version_major;
    int   version_minor;
    int   version_patchlevel;
    int   name_len;
    char *name;
    int   date_len;
    char *date;
    int   desc_len;
    char *desc;
} drm_api_version_t;

typedef struct {
    uint32_t clock;
    uint16_t hdisplay, hsync_start, hsync_end, htotal, hskew;
    uint16_t vdisplay, vsync_start, vsync_end, vtotal, vscan;
    uint32_t vrefresh, flags, type;
    char     name[32];
} drm_api_modeinfo_t;

typedef struct {
    uint32_t            connector_id;
    uint32_t            encoder_id;
    uint32_t            connector_type;
    uint32_t            connector_type_id;
    uint32_t            connection;
    uint32_t            mmWidth, mmHeight;
    uint32_t            subpixel;
    int                 count_modes;
    drm_api_modeinfo_t *modes;
    int                 count_props;
    uint32_t           *props;
    uint64_t           *prop_values;
    int                 count_encoders;
    uint32_t           *encoders;
} drm_api_connector_t;

typedef struct {
    uint32_t encoder_id;
    uint32_t encoder_type;
    uint32_t crtc_id;
    uint32_t possible_crtcs;
    uint32_t possible_clones;
} drm_api_encoder_t;

/* ------------------------------------------------------------------ */
/* Global variable extern declarations                                */
/* ------------------------------------------------------------------ */

extern int            g_debug;
extern int            g_jog_prime_fd;
extern jog_fb_entry_t g_jog_fbs[JOG_MAX_FBS];
extern int            g_jog_fb_count;
extern void          *g_jog_shm_base;        /* mmap of ivshmem BAR2 (1 MiB) */
extern void          *g_jog_shm_pixels;      /* g_jog_shm_base + JOG_SHM_PIXELS_OFF */
extern void          *g_jog_current_dma_ptr;
extern uint8_t       *g_jog_fb;

extern int            g_hidg_rd;
extern int            g_hidg_wr;
extern int            g_hidg_active[MAX_HIDG_FDS];

extern int            g_gpiodrv_rd;
extern int            g_gpiodrv_wr;
extern int            g_gpiodrv_active[MAX_GPIODRV_FDS];

extern int            g_drm_rd;
extern int            g_drm_wr;
extern int            g_seq_rd;
extern int            g_seq_wr;

extern int            g_drm_active[MAX_ACTIVE_FDS];
extern int            g_seq_active[MAX_ALSA_FDS];

/* ------------------------------------------------------------------ */
/* Shared sysfs paths and helper (clock.c + link.c)                   */
/* ------------------------------------------------------------------ */

/* Live host-side pipeline depth in ms, exposed by virtio_snd.ko as the
 * CSV `guest,host,total`. Both the slave-mode clock shim and the master-
 * mode delay-send shim use it as the auto offset. */
#define VSND_LATENCY_AUTO_PATH    "/sys/module/virtio_snd/parameters/audio_latency_ms"
/* Manual override - non-zero forces a fixed compensation value. */
#define VSND_LATENCY_MANUAL_PATH  "/sys/module/virtio_snd/parameters/link_pos_offset_ms"
/* Master switch for both shims; 0 = full passthrough. */
#define VSND_LATENCY_ENABLED_PATH "/sys/module/virtio_snd/parameters/audio_sync_enabled"
/* Re-read the above paths at most once per wall-clock second on the hot path. */
#define VSND_LATENCY_REFRESH_S    1

/* Parse the LAST integer in a CSV/space-separated line. For single-value
 * sysfs files (link_pos_offset_ms, audio_sync_enabled) this is just that
 * number; for the CSV `audio_latency_ms` output (guest,host,total) it
 * returns total. Returns -1 on parse error or empty file. */
long shim_read_sysfs_long(const char *path);

/* ------------------------------------------------------------------ */
/* Function prototypes - fd helpers (ep122_shim_fd.c)                 */
/* ------------------------------------------------------------------ */

int  is_drm_fd(int fd);
int  add_drm_fd(int fd);
void remove_drm_fd(int fd);

int  is_seq_fd(int fd);
int  add_seq_fd(int fd);
void remove_seq_fd(int fd);

int  is_hidg_fd(int fd);
int  add_hidg_fd(int fd);
void remove_hidg_fd(int fd);

int  is_gpiodrv_fd(int fd);
int  add_gpiodrv_fd(int fd);
void remove_gpiodrv_fd(int fd);

/* ------------------------------------------------------------------ */
/* Function prototypes - jog/DRM helpers (jog.c, jog_shm.c)           */
/* ------------------------------------------------------------------ */

jog_flip_slot_t *flip_slot_for(int fd);
void             flip_slot_free(int fd);
int              handle_drm_ioctl(int fd, unsigned long request, void *arg);

/* jog_shm.c - IVSHMEM publish path. Called from jog.c on each flip. */
void publish_frame(const void *src_1280);
void export_jog_prime(int drm_fd, uint32_t gem_handle, uint32_t fb_id);

/* jog.c - synthesised 1280x240@71 mode reused by the connector synthesis path. */
extern const fake_modeinfo_t g_fake_mode;

/* jog.c - find the DMA-mapped host pointer for a given DRM fb_id, or NULL if
 * EP122 hasn't mmap'd that buffer yet. Shared by all flip-path ioctls. */
void *jog_dma_for_fb(uint32_t fb_id);

#endif /* EP122_SHIM_H */
