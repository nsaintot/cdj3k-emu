// SPDX-License-Identifier: MIT OR Apache-2.0
/*
 * jog_shm.c - publishes jog LCD frames to the host through IVSHMEM.
 *
 * This file finds the ivshmem PCI BAR2, initialises the seqlock-style header,
 * extracts the 320×240 image from the 1280×240 buffer on each flip, keeps the
 * framebuffer registry (jog_register_fb), and maps each jog framebuffer with
 * MAP_DUMB so its pixels can be read (export_jog_prime).
 *
 * Only these functions are visible to other files (declared in deck_shim.h):
 *   - publish_frame(const void *src_1280)
 *   - export_jog_prime(int drm_fd, uint32_t gem_handle, uint32_t fb_id)
 */

#include "deck_shim.h"
#include <dirent.h>


/* Register a jog framebuffer handle, so that the shim can match the app's
 * later MAP_DUMB and mmap() calls for it. */
static void jog_register_fb(uint32_t fb_id, uint32_t gem_handle)
{
    /* ADDFB2 can run several times for one framebuffer; register it once. */
    for (int i = 0; i < g_jog_fb_count; i++) {
        if (g_jog_fbs[i].fb_id == fb_id) return;
    }
    if (g_jog_fb_count >= JOG_MAX_FBS) return;
    g_jog_fbs[g_jog_fb_count].fb_id       = fb_id;
    g_jog_fbs[g_jog_fb_count].handle      = gem_handle;
    g_jog_fbs[g_jog_fb_count].mmap_offset = 0;
    g_jog_fbs[g_jog_fb_count].dma_ptr     = NULL;
    g_jog_fb_count++;
    fprintf(stderr, "[deck_shim] jog fb_id=%u handle=%u registered (waiting for the app mmap)\n",
            fb_id, gem_handle);
}

/* Extract the visible 320×240 pixels from the stretched 1280×240 DRM buffer and
 * write them in order into dst (JOG_OUT_BYTES, 307,200 bytes).
 *
 * The app stretches the image 4:1 horizontally: logical pixel C occupies
 * physical columns C*4 to C*4+3. The buffer also wraps, so logical column 0
 * starts at physical column 1024. Each row therefore comes from two source
 * segments:
 *   | Logical columns | Physical columns | Pixels |
 *   |-----------------|------------------|--------|
 *   | 0-63            | 1024-1279        | 64     |
 *   | 64-319          | 0-1023           | 256    |
 * Both segments are stride-4 reads, which the compiler vectorises with NEON. */
static void extract_to_shm(const void *src_1280, void *dst_320)
{
    const uint8_t *src = (const uint8_t *)src_1280;
    uint32_t      *out = (uint32_t *)dst_320;

    for (int row = 0; row < (int)JOG_OUT_H; row++) {
        const uint32_t *row32 = (const uint32_t *)(src + (size_t)row * (JOG_OUT_W * 4u * 4u));
        /* Logical columns 0-63 start at physical columns 1024-1276. */
        const uint32_t *right = row32 + 1024;
        for (int i = 0; i < 64; i++)  out[i]      = right[i * 4];
        /* Logical columns 64-319 start at physical columns 0-1020. */
        for (int j = 0; j < 256; j++) out[64 + j] = row32[j * 4];
        out += JOG_OUT_W;
    }
}

/* Find the ivshmem PCI device (Red Hat vendor 0x1af4, device 0x1110) in
 * /sys/bus/pci/devices and mmap its BAR2 (resource2). BAR0 and BAR1 hold the
 * doorbell and MSI registers; BAR2 is the shared memory of ivshmem-plain.
 * Returns the mapped base, or NULL on failure. */
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
            fprintf(stderr, "[deck_shim] Cannot open %s: %s\n", path, strerror(errno));
            continue;
        }
        map = mmap(NULL, JOG_SHM_BAR_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
        close(fd);
        if (map == MAP_FAILED) {
            fprintf(stderr, "[deck_shim] Cannot mmap the ivshmem BAR2: %s\n", strerror(errno));
            map = NULL;
            continue;
        }
        fprintf(stderr, "[deck_shim] ivshmem BAR2 mapped at %p (PCI %s)\n", map, de->d_name);
        break;
    }
    closedir(d);
    return map;
}

/* Initialise the ivshmem header once. The host polls `seq` in the shared
 * region, so no wake channel is needed. */
static void jog_shm_init_once(void)
{
    if (g_jog_shm_base) return;
    void *base = open_ivshmem_bar();
    if (!base) {
        fprintf(stderr, "[deck_shim] The ivshmem device was not found; the jog stream is disabled.\n");
        return;
    }
    /* `seq` works as a seqlock: an even value means the frame is stable, and
     * an odd value means a write is in progress. */
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
    fprintf(stderr, "[deck_shim] The jog ivshmem region is ready: pixels at %p (%ux%u XRGB8888)\n",
            g_jog_shm_pixels, JOG_OUT_W, JOG_OUT_H);
}

/* Publish a newly extracted frame into ivshmem under the seqlock, so the host
 * can detect a torn frame. */
void publish_frame(const void *src_1280)
{
    void *base = __atomic_load_n(&g_jog_shm_base, __ATOMIC_ACQUIRE);
    if (!base || !g_jog_shm_pixels) return;
    volatile uint32_t *seq = (volatile uint32_t *)((uint8_t *)base + JOG_SHM_OFF_SEQ);
    uint32_t s = *seq;
    __atomic_store_n((uint32_t *)seq, s | 1u, __ATOMIC_RELEASE);
    extract_to_shm(src_1280, g_jog_shm_pixels);
    __atomic_store_n((uint32_t *)seq, (s | 1u) + 1u, __ATOMIC_RELEASE);
}

/* For each 1280×240 jog framebuffer, PRIME-export it to log that the handle is a
 * valid dma-buf, then map it with MAP_DUMB and mmap inside the app so the shim
 * can read its pixels on every flip. The first call also sets up the ivshmem
 * region. */
void export_jog_prime(int drm_fd, uint32_t gem_handle, uint32_t fb_id)
{
    struct { uint32_t handle; uint32_t flags; int fd; } prime;
    prime.handle = gem_handle;
    prime.flags  = 0x00000002u;  /* DRM_RDWR */
    prime.fd     = -1;
    if (sys_ioctl(drm_fd, 0xC00C642Du, &prime) < 0)
        fprintf(stderr, "[deck_shim] PRIME_HANDLE_TO_FD(handle=%u) failed: %s (not fatal)\n",
                gem_handle, strerror(errno));
    else {
        if (g_jog_prime_fd >= 0) sys_close(g_jog_prime_fd);
        g_jog_prime_fd = prime.fd;
        fprintf(stderr, "[deck_shim] jog PRIME fd=%d (handle=%u fb_id=%u)\n",
                prime.fd, gem_handle, fb_id);
    }

    /* Register this handle, then map it with MAP_DUMB and mmap on the app's own
     * DRM fd, inside the app process. */
    jog_register_fb(fb_id, gem_handle);
    {
        fake_mode_map_dumb_t md;
        __builtin_memset(&md, 0, sizeof(md));
        md.handle = gem_handle;
        if (sys_ioctl(drm_fd, 0xC01064B3u /* DRM_IOCTL_MODE_MAP_DUMB = _IOWR('d',0xB3,16) */, &md) == 0) {
            g_jog_fbs[g_jog_fb_count - 1].mmap_offset = md.offset;
            fprintf(stderr, "[deck_shim] MAP_DUMB handle=%u returned offset=0x%llx\n",
                    gem_handle, (unsigned long long)md.offset);
            void *ptr = sys_mmap(NULL, JOG_DMA_SIZE, PROT_READ | PROT_WRITE,
                                 MAP_SHARED, drm_fd, (off_t)md.offset);
            if (ptr != MAP_FAILED) {
                g_jog_fbs[g_jog_fb_count - 1].dma_ptr = ptr;
                fprintf(stderr, "[deck_shim] mmap mapped jog handle=%u at %p\n", gem_handle, ptr);
            } else {
                fprintf(stderr, "[deck_shim] mmap of jog handle=%u failed: %s\n", gem_handle, strerror(errno));
            }
        } else {
            fprintf(stderr, "[deck_shim] MAP_DUMB handle=%u failed: %s\n", gem_handle, strerror(errno));
        }
    }

    /* On the first framebuffer, find and map the ivshmem BAR. */
    jog_shm_init_once();
}
