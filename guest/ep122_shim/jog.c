/* SPDX-License-Identifier: MIT OR Apache-2.0 */
#include "ep122_shim.h"
#include <dirent.h>

/* ------------------------------------------------------------------ */
/* DRM ioctl handler                                                  */
/* ------------------------------------------------------------------ */
/*
 * struct drm_version on aarch64 (LP64) - 64 bytes total:
 *   int    version_major       [0]
 *   int    version_minor       [4]
 *   int    version_patchlevel  [8]
 *   char   _pad[4]             [12]  (alignment)
 *   size_t name_len            [16]
 *   char  *name               [24]
 *   size_t date_len            [32]
 *   char  *date               [40]
 *   size_t desc_len            [48]
 *   char  *desc               [56]
 */
typedef struct {
    int      version_major;
    int      version_minor;
    int      version_patchlevel;
    char     _pad[4];
    uint64_t name_len;
    char    *name;
    uint64_t date_len;
    char    *date;
    uint64_t desc_len;
    char    *desc;
} fake_drm_version_t;

_Static_assert(sizeof(fake_drm_version_t) == 64,
               "fake_drm_version_t must be 64 bytes on aarch64");

/* The 1280×240 @ 71 Hz mode we hand to EP122 (DSI-2, jog LCD).
 * Declared `extern` in shim.h - also consumed by jog_drm.c when synthesising
 * the connector mode list. */
const fake_modeinfo_t g_fake_mode = {
    .clock       = FAKE_CLK_KHZ,
    .hdisplay    = FAKE_W,
    .hsync_start = FAKE_W + 10,
    .hsync_end   = FAKE_W + 30,
    .htotal      = FAKE_HTOTAL,
    .hskew       = 0,
    .vdisplay    = FAKE_H,
    .vsync_start = FAKE_H + 4,
    .vsync_end   = FAKE_H + 8,
    .vtotal      = FAKE_VTOTAL,
    .vscan       = 0,
    .vrefresh    = 71,
    .flags       = 0,
    .type        = 6,   /* DRM_MODE_TYPE_PREFERRED | DRM_MODE_TYPE_DRIVER */
    .name        = "1280x240",
};


/* Per-fd flip event state. */
static jog_flip_slot_t g_jog_flip[MAX_JOG_RENDER_FDS] = {
    {-1,0,0}, {-1,0,0}, {-1,0,0}, {-1,0,0}
};

jog_flip_slot_t *flip_slot_for(int fd) {
    for (int i = 0; i < MAX_JOG_RENDER_FDS; i++)
        if (g_jog_flip[i].fd == fd) return &g_jog_flip[i];
    return NULL;
}
static jog_flip_slot_t *flip_slot_alloc(int fd) {
    for (int i = 0; i < MAX_JOG_RENDER_FDS; i++) {
        int expected = -1;
        if (__atomic_compare_exchange_n(&g_jog_flip[i].fd, &expected, fd,
                                        0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST))
            return &g_jog_flip[i];
    }
    return NULL;
}
void flip_slot_free(int fd) {
    for (int i = 0; i < MAX_JOG_RENDER_FDS; i++) {
        if (g_jog_flip[i].fd == fd) {
            g_jog_flip[i].pending   = 0;
            g_jog_flip[i].user_data = 0;
            __atomic_store_n(&g_jog_flip[i].fd, -1, __ATOMIC_SEQ_CST);
            return;
        }
    }
}

/* Last SETPLANE scanout state. */
static uint32_t g_setplane_crtc_id = 0;
static uint32_t g_setplane_fb_id   = 0;

/* Linear scan over the small (≤ JOG_MAX_FBS) registered-jog-fb table. Returns
 * the host-visible dma pointer for `fb_id`, or NULL if EP122 hasn't mmap'd it
 * yet. Centralises the lookup that PAGE_FLIP, SETCRTC, SETPLANE,
 * ATOMIC_COMMIT, and DIRTY_FB all need on every flip. */
void *jog_dma_for_fb(uint32_t fb_id)
{
    for (int i = 0; i < g_jog_fb_count; i++) {
        if (g_jog_fbs[i].fb_id == fb_id)
            return g_jog_fbs[i].dma_ptr;
    }
    return NULL;
}

static void inject_drm_flip_event(int fd, uint64_t user_data)
{
    jog_flip_slot_t *s = flip_slot_for(fd);
    if (!s) s = flip_slot_alloc(fd);
    if (!s) return;
    __atomic_store_n(&s->user_data, user_data, __ATOMIC_RELEASE);
    __atomic_fetch_add(&s->pending, 1, __ATOMIC_RELEASE);
}

int handle_drm_ioctl(int fd, unsigned long request, void *arg) {
    uint8_t cmd = (uint8_t)(request & 0xFFu);

    /* SET_MASTER (0x1e) / DROP_MASTER (0x1f): EP122 tries to become DRM master after
     * opening the device.  With Xorg already holding master on virtio_gpu this returns
     * EPERM, which EP122 treats as a fatal init error.  The shim handles all KMS calls
     * itself so EP122 never needs to be the real master - just pretend to succeed. */
    if (cmd == 0x1eu || cmd == 0x1fu) {
        DBG("drm ioctl(fd=%d, %s) → 0 (suppressed)\n", fd,
            cmd == 0x1eu ? "SET_MASTER" : "DROP_MASTER");
        return 0;
    }

    /* SET_CLIENT_CAP (0x0D): virtio-gpu rejects DRM_CLIENT_CAP_ATOMIC (cap=3) with
     * EOPNOTSUPP.  EP122 calls drmSetClientCap(ATOMIC) twice - once during init and
     * once from a deferred capability-check thread.  The second call crashes EP122.
     * Always return 0 regardless of capability so EP122 proceeds with atomic mode. */
    if (cmd == DRM_CMD_SET_CLIENT_CAP) {
        /* EP122 calls SET_CLIENT_CAP again during jog-LCD re-initialization, before
         * it rebuilds its callback context.  Any flip still pending at that point
         * would fire into a freed evctx → SIGSEGV.  Clear it now. */
        {
            jog_flip_slot_t *s = flip_slot_for(fd);
            if (s) __atomic_store_n(&s->pending, 0, __ATOMIC_SEQ_CST);
        }
        return 0;
    }

    if (cmd == DRM_CMD_VERSION) {
        /* Fake "rockchip" driver name so EP122's JogLcdDRM passes its driver check. */
        if (arg) {
            fake_drm_version_t *v = (fake_drm_version_t *)arg;
            v->version_major      = 1;
            v->version_minor      = 4;
            v->version_patchlevel = 0;
            if (v->name && v->name_len >= 8) memcpy(v->name, "rockchip", 8);
            v->name_len = 8;
            if (v->date && v->date_len >= 10) memcpy(v->date, "2023-01-01", 10);
            v->date_len = 10;
            if (v->desc && v->desc_len >= 6) memcpy(v->desc, "RK DRM", 6);
            v->desc_len = 6;
        }
        DBG("drm ioctl(fd=%d, VERSION) → rockchip v1.4.0\n", fd);
        return 0;
    }

    /* GETRESOURCES: pass through and log what the kernel returns. */
    if (cmd == DRM_CMD_MODE_GETRESOURCES) {
        int r = sys_ioctl(fd, request, arg);
        if (arg) {
            const uint32_t *p32 = (const uint32_t *)((const uint8_t *)arg + 32);
            uint32_t nfb = p32[0], nc = p32[1], nconn = p32[2], nenc = p32[3];
            DBG("GETRESOURCES rc=%d: fbs=%u crtcs=%u connectors=%u encoders=%u\n",
                r, nfb, nc, nconn, nenc);
        }
        return r;
    }

    /* CREATE_DUMB: log buffer allocation. */
    if (cmd == DRM_CMD_MODE_CREATE_DUMB) {
        int r = sys_ioctl(fd, request, arg);
        if (arg) {
            const uint32_t *p = (const uint32_t *)arg;
            uint32_t h = p[0], w = p[1], bpp = p[2];
            uint32_t handle = p[4], pitch = p[5];
            uint64_t size; memcpy(&size, p + 6, 8);
            DBG("CREATE_DUMB: %ux%u bpp=%u → handle=%u pitch=%u size=%llu rc=%d\n",
                w, h, bpp, handle, pitch, (unsigned long long)size, r);
        }
        return r;
    }

    /* ADDFB2: pass through, log dimensions, map + export if 1280×240. */
    if (cmd == DRM_CMD_MODE_ADDFB2) {
        int r = sys_ioctl(fd, request, arg);
        if (arg) {
            const uint32_t *p = (const uint32_t *)arg;
            uint32_t w = p[1], h = p[2], handle = p[5], fb_id = p[0];
            uint32_t pitch0  = p[9];
            uint32_t offset0 = p[13];
            DBG("ADDFB2: %ux%u handle=%u pitch=%u offset=%u → fb_id=%u rc=%d\n",
                w, h, handle, pitch0, offset0, fb_id, r);
            if (r == 0 && w == 1280 && h == 240 && handle != 0)
                export_jog_prime(fd, handle, fb_id);
        }
        return r;
    }

    /* Legacy ADDFB */
    if (cmd == DRM_CMD_MODE_ADDFB) {
        int r = sys_ioctl(fd, request, arg);
        if (arg) {
            const uint32_t *p = (const uint32_t *)arg;
            uint32_t w = p[1], h = p[2], handle = p[6], fb_id = p[0];
            DBG("ADDFB: %ux%u handle=%u → fb_id=%u rc=%d\n",
                w, h, handle, fb_id, r);
            if (r == 0 && w == 1280 && h == 240 && handle != 0)
                export_jog_prime(fd, handle, fb_id);
        }
        return r;
    }

    /* PAGE_FLIP (legacy KMS) */
    if (cmd == DRM_CMD_MODE_PAGE_FLIP) {
        void *flip_src = NULL;
        uint32_t flip_fb = 0, flip_flags = 0;
        uint64_t flip_user_data = 0;
        if (arg) {
            const uint32_t *p = (const uint32_t *)arg;
            flip_fb = p[1]; flip_flags = p[2];
            memcpy(&flip_user_data, p + 4, 8);
            flip_src = jog_dma_for_fb(flip_fb);
        }
        int r = sys_ioctl(fd, request, arg);
        if (flip_src) {
            __atomic_store_n(&g_jog_current_dma_ptr, flip_src, __ATOMIC_RELEASE);
            publish_frame(flip_src);

        }
        if (flip_flags & 0x1u) {
            if (r != 0) {
                inject_drm_flip_event(fd, flip_user_data);
                r = 0;
            }
        }
        return r;
    }

    /* SETCRTC (legacy) */
    if (cmd == DRM_CMD_MODE_SETCRTC) {
        int r = sys_ioctl(fd, request, arg);
        if (arg) {
            const fake_mode_crtc_t *crtc = (const fake_mode_crtc_t *)arg;
            void *src = jog_dma_for_fb(crtc->fb_id);
            if (src && g_jog_shm_pixels) {
                __atomic_store_n(&g_jog_current_dma_ptr, src, __ATOMIC_RELEASE);
                publish_frame(src);

                return 0;
            }
        }
        return r;
    }

    /* OBJ_GETPROPS (0xB9) */
    if (cmd == 0xB9u) {
        if (arg) {
            uint8_t *b = (uint8_t *)arg;
            uint64_t props_ptr, prop_values_ptr;
            uint32_t count_props, obj_id, obj_type;
            memcpy(&props_ptr,       b + 0,  8);
            memcpy(&prop_values_ptr, b + 8,  8);
            memcpy(&count_props,     b + 16, 4);
            memcpy(&obj_id,          b + 20, 4);
            memcpy(&obj_type,        b + 24, 4);
            if (obj_type == 0xeeeeeeeeu) {
                if (count_props == 0 || props_ptr == 0) {
                    uint32_t n = 2;
                    memcpy(b + 16, &n, 4);
                    DBG("OBJ_GETPROPS(1): plane=%u → count=2 (FB_ID,CRTC_ID)\n", obj_id);
                } else {
                    uint32_t ids[2]  = { 100u, 101u };
                    uint64_t vals[2] = { (uint64_t)g_setplane_fb_id,
                                         (uint64_t)g_setplane_crtc_id };
                    uint32_t n = 2;
                    if (count_props > 2) count_props = 2;
                    memcpy((void *)(uintptr_t)props_ptr,       ids,  count_props * 4);
                    memcpy((void *)(uintptr_t)prop_values_ptr, vals, count_props * 8);
                    memcpy(b + 16, &n, 4);
                    DBG("OBJ_GETPROPS(2): plane=%u → FB_ID=%u CRTC_ID=%u\n",
                        obj_id, g_setplane_fb_id, g_setplane_crtc_id);
                }
                return 0;
            }
        }
        return sys_ioctl(fd, request, arg);
    }

    /* GETPROPERTY (0xAA) */
    if (cmd == 0xAAu) {
        if (arg) {
            uint8_t *b = (uint8_t *)arg;
            uint32_t prop_id;
            memcpy(&prop_id, b + 16, 4);
            if (prop_id == 100u || prop_id == 101u) {
                const char *name = (prop_id == 100u) ? "FB_ID" : "CRTC_ID";
                uint32_t flags = 0x40u;
                uint32_t zero  = 0;
                memset(b + 24, 0, 32);
                strncpy((char *)(b + 24), name, 31);
                memcpy(b + 20, &flags, 4);
                memcpy(b + 56, &zero,  4);
                memcpy(b + 60, &zero,  4);
                DBG("GETPROPERTY: prop_id=%u → name=%s\n", prop_id, name);
                return 0;
            }
        }
        return sys_ioctl(fd, request, arg);
    }

    /* ATOMIC_COMMIT (0xBC) */
    if (cmd == DRM_CMD_MODE_ATOMIC) {
        if (arg) {
            const uint8_t *b = (const uint8_t *)arg;
            uint32_t atomic_flags, count_objs;
            uint64_t count_props_ptr, props_u64, prop_values_ptr, user_data;
            memcpy(&atomic_flags,     b + 0,  4);
            memcpy(&count_objs,       b + 4,  4);
            memcpy(&count_props_ptr,  b + 16, 8);
            memcpy(&props_u64,        b + 24, 8);
            memcpy(&prop_values_ptr,  b + 32, 8);
            memcpy(&user_data,        b + 48, 8);

            uint32_t new_fb_id = 0;
            if (count_objs && count_props_ptr && props_u64 && prop_values_ptr) {
                const uint32_t *cprops = (const uint32_t *)(uintptr_t)count_props_ptr;
                const uint32_t *pids   = (const uint32_t *)(uintptr_t)props_u64;
                const uint64_t *pvals  = (const uint64_t *)(uintptr_t)prop_values_ptr;
                uint32_t pidx = 0;
                for (uint32_t oi = 0; oi < count_objs; oi++) {
                    uint32_t np = cprops[oi];
                    for (uint32_t pi = 0; pi < np; pi++, pidx++) {
                        if (pids[pidx] == 100u)
                            new_fb_id = (uint32_t)pvals[pidx];
                    }
                }
            }

            void *flip_src = new_fb_id ? jog_dma_for_fb(new_fb_id) : NULL;
            if (flip_src && g_jog_shm_pixels) {
                __atomic_store_n(&g_jog_current_dma_ptr, flip_src, __ATOMIC_RELEASE);
                publish_frame(flip_src);

            }
            if (new_fb_id) g_setplane_fb_id = new_fb_id;

            if (atomic_flags & 0x1u)
                inject_drm_flip_event(fd, user_data);
        }
        return 0;
    }

    /* GETPLANERESOURCES (0xB5): virtio-gpu only exposes planes for CRTC 0 (main
     * screen); none have possible_crtcs & 0x2 (jog CRTC bit). Synthesize a
     * single plane 80 so EP122 finds a plane capable of driving the jog CRTC.
     *
     * struct drm_mode_get_plane_res: u64 plane_id_ptr [0..7], u32 count_planes [8..11] */
    if (cmd == 0xB5u) {
        if (!arg) return 0;
        uint64_t ptr;
        uint32_t count;
        memcpy(&ptr,   (const uint8_t *)arg + 0, 8);
        memcpy(&count, (const uint8_t *)arg + 8, 4);

        if (count == 0 || ptr == 0) {
            uint32_t one = 1;
            memcpy((uint8_t *)arg + 8, &one, 4);
            DBG("GETPLANERESOURCES: synthetic count=1 (plane 80)\n");
        } else {
            uint32_t pid = 80;
            memcpy((void *)(uintptr_t)ptr, &pid, sizeof(pid));
            uint32_t one = 1;
            memcpy((uint8_t *)arg + 8, &one, 4);
            DBG("GETPLANERESOURCES: wrote plane_ids=[80] count=1\n");
        }
        return 0;
    }

    /* GETPLANE (0xB6): virtio-gpu 4.4 returns ENOMEM which EP122 mishandles
     * (segfault via bad ptr deref). Return a synthetic valid response for plane 80
     * with possible_crtcs=0x3 so EP122 sees it as valid for both CRTCs (main + jog).
     *
     * struct drm_mode_get_plane: plane_id[0] crtc_id[1] fb_id[2] possible_crtcs[3]
     *   gamma_size[4] count_format_types[5] format_type_ptr[6..7] */
    if (cmd == 0xB6u) {
        if (arg) {
            uint32_t *p = (uint32_t *)arg;
            uint32_t plane_id = p[0];
            p[1] = (plane_id == 80) ? g_setplane_crtc_id : 0;
            p[2] = (plane_id == 80) ? g_setplane_fb_id   : 0;
            p[3] = (plane_id == 80) ? 0x3u : 0x1u;
            p[4] = 0;
            p[5] = 0;
            DBG("GETPLANE: plane=%u → crtc=%u fb=%u possible=0x%x (synthetic)\n",
                plane_id, p[1], p[2], p[3]);
        }
        return 0;
    }

    /* MAP_DUMB: snoop mmap offset for jog buffers. */
    if (cmd == DRM_CMD_MODE_MAP_DUMB) {
        int r = sys_ioctl(fd, request, arg);
        if (r == 0 && arg) {
            const fake_mode_map_dumb_t *md = (const fake_mode_map_dumb_t *)arg;
            for (int i = 0; i < g_jog_fb_count; i++) {
                if (g_jog_fbs[i].handle == md->handle) {
                    g_jog_fbs[i].mmap_offset = md->offset;
                    DBG("MAP_DUMB jog handle=%u → mmap_offset=0x%llx\n",
                        md->handle, (unsigned long long)md->offset);
                }
            }
        }
        return r;
    }

    /* GETENCODER: pass through, log, and patch crtc_id=0 for the jog encoder. */
    if (cmd == DRM_CMD_MODE_GETENCODER) {
        int r = sys_ioctl(fd, request, arg);
        if (r == 0 && arg) {
            fake_mode_encoder_t *ke = (fake_mode_encoder_t *)arg;
            if (ke->crtc_id == 0 && (ke->possible_crtcs & 2u)) {
                fake_mode_res_t res;
                memset(&res, 0, sizeof(res));
                if (sys_ioctl(fd, 0xC04064A0u, &res) == 0 && res.count_crtcs >= 2) {
                    uint32_t *cids = (uint32_t *)calloc(res.count_crtcs, sizeof(uint32_t));
                    uint32_t *fids = (uint32_t *)calloc(res.count_fbs    ? res.count_fbs    : 1, sizeof(uint32_t));
                    uint32_t *nids = (uint32_t *)calloc(res.count_connectors ? res.count_connectors : 1, sizeof(uint32_t));
                    uint32_t *eids = (uint32_t *)calloc(res.count_encoders   ? res.count_encoders   : 1, sizeof(uint32_t));
                    if (cids && fids && nids && eids) {
                        res.crtc_id_ptr      = (uint64_t)(uintptr_t)cids;
                        res.fb_id_ptr        = (uint64_t)(uintptr_t)fids;
                        res.connector_id_ptr = (uint64_t)(uintptr_t)nids;
                        res.encoder_id_ptr   = (uint64_t)(uintptr_t)eids;
                        if (sys_ioctl(fd, 0xC04064A0u, &res) == 0)
                            ke->crtc_id = cids[1];
                    }
                    free(cids); free(fids); free(nids); free(eids);
                }
                DBG("GETENCODER(id=%u) type=%u possible=0x%x → patched crtc_id=%u\n",
                    ke->encoder_id, ke->encoder_type, ke->possible_crtcs, ke->crtc_id);
            } else {
                DBG("GETENCODER(id=%u) type=%u crtc_id=%u possible=0x%x\n",
                    ke->encoder_id, ke->encoder_type, ke->crtc_id, ke->possible_crtcs);
            }
        }
        return r;
    }

    /* SETPLANE (0xB7) */
    if (cmd == 0xB7u) {
        uint32_t crtc_id = 0, fb_id = 0;
        void *flip_src = NULL;
        if (arg) {
            const uint32_t *p = (const uint32_t *)arg;
            crtc_id = p[1]; fb_id = p[2];
            flip_src = jog_dma_for_fb(fb_id);
        }

        if (flip_src) {
            __atomic_store_n(&g_jog_current_dma_ptr, flip_src, __ATOMIC_RELEASE);
            publish_frame(flip_src);

        }
        g_setplane_crtc_id = crtc_id;
        g_setplane_fb_id   = fb_id;
        return 0;
    }

    /* DIRTY_FB (0xB1) */
    if (cmd == 0xB1u) {
        /* No fb_id in the ioctl arg, so reuse the last published source. */
        void *src = __atomic_load_n(&g_jog_current_dma_ptr, __ATOMIC_ACQUIRE);
        if (src && g_jog_shm_pixels) { publish_frame(src); }
        return sys_ioctl(fd, request, arg);
    }

    /* WAIT_VBLANK (cmd=0x06) */
    if (cmd == 0x06u) {
        static uint32_t g_vblank_seq = 0;
        if (arg) {
            uint32_t *p = (uint32_t *)arg;
            p[0] = 0;
            p[1] = __atomic_fetch_add(&g_vblank_seq, 1, __ATOMIC_RELAXED);
            p[2] = 0; p[3] = 0;
        }
        struct timespec _ts = { .tv_sec = 0, .tv_nsec = 16666666L };
        syscall(SYS_nanosleep, &_ts, NULL);
        return 0;
    }

    /* Catch-all: pass through and log rc. */
    {
        int r = sys_ioctl(fd, request, arg);
        static uint8_t seen[256] = {0};
        if (!seen[cmd]) {
            seen[cmd] = 1;
            DBG("DRM cmd=0x%02X request=0x%lX rc=%d (first occurrence)\n",
                cmd, (unsigned long)request, r);
        }
        return r;
    }

}

/* ------------------------------------------------------------------ */
/* drmOpen / drmClose  - libdrm high-level API hooks                  */
/* ------------------------------------------------------------------ */

int drmOpen(const char *name, const char *busid) {
    (void)busid;
    long fd = syscall(SYS_openat, AT_FDCWD, "/dev/dri/card0",
                      O_RDWR | O_CLOEXEC, (long)0);
    if (fd < 0) { errno = (int)-fd; return -1; }
    if (add_drm_fd((int)fd) < 0) { syscall(SYS_close, fd); errno = EMFILE; return -1; }
    DBG("drmOpen(\"%s\") → real fd %ld\n", name ? name : "(null)", fd);
    return (int)fd;
}

int drmClose(int fd) {
    if (is_drm_fd(fd)) {
        remove_drm_fd(fd);
        DBG("drmClose(%d)\n", fd);
    }
    return sys_close(fd);
}

int drmIoctl(int fd, unsigned long request, void *arg) {
    return handle_drm_ioctl(fd, request, arg);
}

int drmHandleEvent(int fd, fake_event_ctx_t *evctx) {
    jog_flip_slot_t *s = flip_slot_for(fd);
    if (!s) return 0;

    int pending = __atomic_exchange_n(&s->pending, 0, __ATOMIC_ACQUIRE);
    if (!pending) return 0;

    if (!evctx) return 0;

    uint64_t user_data = __atomic_load_n(&s->user_data, __ATOMIC_ACQUIRE);
    static uint32_t seq = 0;
    uint32_t n = __atomic_fetch_add(&seq, 1, __ATOMIC_RELAXED);

    if (evctx->version >= 2 && evctx->page_flip_handler2)
        evctx->page_flip_handler2(fd, n, 0, 0, g_setplane_crtc_id, (void *)(uintptr_t)user_data);
    else if (evctx->page_flip_handler)
        evctx->page_flip_handler(fd, n, 0, 0, (void *)(uintptr_t)user_data);

    return 0;
}

int drmSetMaster(int fd) {
    (void)fd;
    DBG("drmSetMaster(%d) → 0 (suppressed - Xorg holds real master)\n", fd);
    return 0;
}

int drmDropMaster(int fd) {
    (void)fd;
    DBG("drmDropMaster(%d) → 0 (suppressed)\n", fd);
    return 0;
}

