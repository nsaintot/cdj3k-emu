// SPDX-License-Identifier: MIT OR Apache-2.0
/*
 * jog_shm.c - IVSHMEM publish path for the jog LCD.
 *
 * Owns: PCI BAR2 discovery, the seqlock-style header init, the per-flip
 * 1280×240 → 320×240 horizontal-stride extract, the framebuffer registry
 * (jog_register_fb), and PRIME-export + MAP_DUMB plumbing for the host-
 * side jog frame texture (export_jog_prime).
 *
 * Cross-TU surface (declared in shim.h):
 *   - publish_frame(const void *src_1280)
 *   - export_jog_prime(int drm_fd, uint32_t gem_handle, uint32_t fb_id)
 * Everything else is file-scope (static).
 */

#include "ep122_shim.h"
#include <dirent.h>


/* Register a jog framebuffer handle so we can later snoop EP122's MAP_DUMB
 * and mmap() calls for it. */
static void jog_register_fb(uint32_t fb_id, uint32_t gem_handle)
{
    /* Avoid duplicate registrations (ADDFB2 can be called multiple times) */
    for (int i = 0; i < g_jog_fb_count; i++) {
        if (g_jog_fbs[i].fb_id == fb_id) return;
    }
    if (g_jog_fb_count >= JOG_MAX_FBS) return;
    g_jog_fbs[g_jog_fb_count].fb_id       = fb_id;
    g_jog_fbs[g_jog_fb_count].handle      = gem_handle;
    g_jog_fbs[g_jog_fb_count].mmap_offset = 0;
    g_jog_fbs[g_jog_fb_count].dma_ptr     = NULL;
    g_jog_fb_count++;
    fprintf(stderr, "[ep122_shim] jog fb_id=%u handle=%u registered (waiting for EP122 mmap)\n",
            fb_id, gem_handle);
}

/* Extract the visible 320×240 pixels from the 1280×240 stretched DRM buffer and
 * write them sequentially into dst (JOG_OUT_BYTES = 307,200 bytes).
 *
 * EP122 writes a 4:1 horizontally-stretched buffer where logical pixel C occupies
 * physical columns C*4…C*4+3.  The buffer also wraps: logical col 0 starts at
 * physical col 1024.  The visible region therefore splits into two contiguous
 * source segments per row:
 *   right segment: logical cols   0.. 63 → physical cols 1024..1279 (64  pixels)
 *   left  segment: logical cols  64..319 → physical cols    0..1019 (256 pixels)
 * Both segments are stride-4 reads, which the compiler auto-vectorises with NEON. */
static void extract_to_shm(const void *src_1280, void *dst_320)
{
    const uint8_t *src = (const uint8_t *)src_1280;
    uint32_t      *out = (uint32_t *)dst_320;

    for (int row = 0; row < (int)JOG_OUT_H; row++) {
        const uint32_t *row32 = (const uint32_t *)(src + (size_t)row * (JOG_OUT_W * 4u * 4u));
        /* Right segment: logical cols 0..63 from physical cols 1024..1276 */
        const uint32_t *right = row32 + 1024;
        for (int i = 0; i < 64; i++)  out[i]      = right[i * 4];
        /* Left segment: logical cols 64..319 from physical cols 0..1020 */
        for (int j = 0; j < 256; j++) out[64 + j] = row32[j * 4];
        out += JOG_OUT_W;
    }
}

/* Locate the ivshmem PCI device (Red Hat vendor 0x1af4, device 0x1110) by
 * scanning /sys/bus/pci/devices and mmap its BAR2 (resource2). BAR0/BAR1 are
 * the doorbell/MSI region; BAR2 is the shared memory region for ivshmem-plain.
 * Returns mmap'd base on success, NULL on failure. */
static void *open_ivshmem_bar(void)
{
    DIR *d = opendir("/sys/bus/pci/devices");
    if (!d) return NULL;
    struct dirent *de;
    char path[320], buf[16];
    int fd, n;
    void *map = NULL;
    while ((de = readdir(d)) != NULL) {
        if (de->d_name[0] == '.') continue;

        snprintf(path, sizeof(path), "/sys/bus/pci/devices/%s/vendor", de->d_name);
        fd = open(path, O_RDONLY); if (fd < 0) continue;
        n = (int)read(fd, buf, sizeof(buf) - 1); close(fd);
        if (n <= 0) continue;
        buf[n] = 0;
        if (strtoul(buf, NULL, 0) != 0x1af4u) continue;

        snprintf(path, sizeof(path), "/sys/bus/pci/devices/%s/device", de->d_name);
        fd = open(path, O_RDONLY); if (fd < 0) continue;
        n = (int)read(fd, buf, sizeof(buf) - 1); close(fd);
        if (n <= 0) continue;
        buf[n] = 0;
        if (strtoul(buf, NULL, 0) != 0x1110u) continue;

        snprintf(path, sizeof(path), "/sys/bus/pci/devices/%s/resource2", de->d_name);
        fd = open(path, O_RDWR | O_SYNC);
        if (fd < 0) {
            fprintf(stderr, "[ep122_shim] open %s: %s\n", path, strerror(errno));
            continue;
        }
        map = mmap(NULL, JOG_SHM_BAR_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
        close(fd);
        if (map == MAP_FAILED) {
            fprintf(stderr, "[ep122_shim] mmap ivshmem BAR2: %s\n", strerror(errno));
            map = NULL;
            continue;
        }
        fprintf(stderr, "[ep122_shim] ivshmem BAR2 mapped at %p (PCI %s)\n", map, de->d_name);
        break;
    }
    closedir(d);
    return map;
}

/* Initialize the ivshmem region header. Idempotent. The host polls `seq`
 * directly from the shared region, so no wake channel is required. */
static void jog_shm_init_once(void)
{
    if (g_jog_shm_base) return;
    void *base = open_ivshmem_bar();
    if (!base) {
        fprintf(stderr, "[ep122_shim] ivshmem device not found - jog stream disabled\n");
        return;
    }
    /* Initialize header. Use seqlock-style: even = stable, odd = mid-write. */
    volatile uint32_t *p32 = (volatile uint32_t *)base;
    volatile uint16_t *p16 = (volatile uint16_t *)base;
    p32[JOG_SHM_OFF_SEQ / 4] = 0;
    p16[JOG_SHM_OFF_W   / 2] = (uint16_t)JOG_OUT_W;
    p16[JOG_SHM_OFF_H   / 2] = (uint16_t)JOG_OUT_H;
    p32[JOG_SHM_OFF_FMT / 4] = JOG_SHM_FMT_XRGB8888;
    __atomic_thread_fence(__ATOMIC_RELEASE);
    p32[JOG_SHM_OFF_MAGIC / 4] = JOG_SHM_MAGIC;

    g_jog_shm_pixels = (uint8_t *)base + JOG_SHM_PIXELS_OFF;
    __atomic_store_n(&g_jog_shm_base, base, __ATOMIC_RELEASE);
    fprintf(stderr, "[ep122_shim] jog ivshmem ready: pixels @ %p (%ux%u XRGB8888)\n",
            g_jog_shm_pixels, JOG_OUT_W, JOG_OUT_H);
}

/* Publish a freshly extracted frame into ivshmem with seqlock semantics so the
 * host can detect torn writes. Host polls `seq` - no wake channel needed. */
void publish_frame(const void *src_1280)
{
    void *base = __atomic_load_n(&g_jog_shm_base, __ATOMIC_ACQUIRE);
    if (!base || !g_jog_shm_pixels) return;
    volatile uint32_t *seq = (volatile uint32_t *)((uint8_t *)base + JOG_SHM_OFF_SEQ);
    uint32_t s = *seq;
    /* odd → write in progress */
    __atomic_store_n((uint32_t *)seq, s | 1u, __ATOMIC_RELEASE);
    extract_to_shm(src_1280, g_jog_shm_pixels);
    __atomic_store_n((uint32_t *)seq, (s | 1u) + 1u, __ATOMIC_RELEASE);
}

/* On the first 1280×240 jog framebuffer: PRIME-export it (logging only - proves
 * the handle is a valid dma-buf), MAP_DUMB+mmap it inside EP122 so we can read
 * pixels on every flip, then bring up the ivshmem publish path. */
void export_jog_prime(int drm_fd, uint32_t gem_handle, uint32_t fb_id)
{
    /* Keep PRIME export for logging (also proves the handle is a valid dma-buf). */
    struct { uint32_t handle; uint32_t flags; int fd; } prime;
    prime.handle = gem_handle;
    prime.flags  = 0x00000002u;  /* DRM_RDWR */
    prime.fd     = -1;
    if (sys_ioctl(drm_fd, 0xC00C642Du, &prime) < 0)
        fprintf(stderr, "[ep122_shim] PRIME_HANDLE_TO_FD(handle=%u): %s (non-fatal)\n",
                gem_handle, strerror(errno));
    else {
        if (g_jog_prime_fd >= 0) sys_close(g_jog_prime_fd);
        g_jog_prime_fd = prime.fd;
        fprintf(stderr, "[ep122_shim] jog PRIME fd=%d (handle=%u fb_id=%u)\n",
                prime.fd, gem_handle, fb_id);
    }

    /* Register this handle, then immediately MAP_DUMB + mmap it ourselves.
     * We are running inside EP122's process (the DRM master), so this is safe -
     * the previous kernel crash only occurred when called from a non-master process. */
    jog_register_fb(fb_id, gem_handle);
    {
        fake_mode_map_dumb_t md;
        __builtin_memset(&md, 0, sizeof(md));
        md.handle = gem_handle;
        if (sys_ioctl(drm_fd, 0xC01064B3u /* DRM_IOCTL_MODE_MAP_DUMB = _IOWR('d',0xB3,16) */, &md) == 0) {
            g_jog_fbs[g_jog_fb_count - 1].mmap_offset = md.offset;
            fprintf(stderr, "[ep122_shim] MAP_DUMB(master) handle=%u → offset=0x%llx\n",
                    gem_handle, (unsigned long long)md.offset);
            void *ptr = sys_mmap(NULL, JOG_DMA_SIZE, PROT_READ | PROT_WRITE,
                                 MAP_SHARED, drm_fd, (off_t)md.offset);
            if (ptr != MAP_FAILED) {
                g_jog_fbs[g_jog_fb_count - 1].dma_ptr = ptr;
                fprintf(stderr, "[ep122_shim] mmap(master) jog handle=%u → %p\n", gem_handle, ptr);
            } else {
                fprintf(stderr, "[ep122_shim] mmap(master) jog handle=%u: %s\n", gem_handle, strerror(errno));
            }
        } else {
            fprintf(stderr, "[ep122_shim] MAP_DUMB(master) handle=%u: %s\n", gem_handle, strerror(errno));
        }
    }

    /* On first framebuffer: locate ivshmem BAR + open wake vport. */
    jog_shm_init_once();
}
