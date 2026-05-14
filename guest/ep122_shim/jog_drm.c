// SPDX-License-Identifier: MIT OR Apache-2.0
/*
 * jog_drm.c - libdrm-style API surface (drmGetVersion, drmModeGet*Connector,
 * drmModeGetEncoder, and the matching free helpers). Pure plumbing - these
 * fabricate DRM objects matching what the EP122 jog assembly expects.
 *
 * Statics _get_connector_real / _get_encoder_real (if present) live here too
 * since they're only called from within this TU.
 */

#include "ep122_shim.h"

drm_api_version_t *drmGetVersion(int fd) {
    (void)fd;
    drm_api_version_t *v = (drm_api_version_t *)calloc(1, sizeof(*v));
    if (!v) return NULL;
    v->version_major      = 1;
    v->version_minor      = 4;
    v->version_patchlevel = 0;
    v->name     = (char *)calloc(9,  1); if (v->name) { memcpy(v->name, "rockchip", 8); v->name_len = 8; }
    v->date     = (char *)calloc(11, 1); if (v->date) { memcpy(v->date, "2023-01-01", 10); v->date_len = 10; }
    v->desc     = (char *)calloc(7,  1); if (v->desc) { memcpy(v->desc, "RK DRM", 6);  v->desc_len = 6; }
    DBG("drmGetVersion(fd=%d) → rockchip v1.4.0\n", fd);
    return v;
}

void drmFreeVersion(drm_api_version_t *v) {
    if (!v) return;
    free(v->name); free(v->date); free(v->desc);
    free(v);
}

/* ------------------------------------------------------------------ */
/* drmModeGetConnector / drmModeGetConnectorCurrent                   */
/* ------------------------------------------------------------------ */

#define DRM_API_MODE_CONNECTED    1

/* DRM_IOCTL_MODE_GETCONNECTOR = _IOWR('d', 0xa7, fake_mode_connector_t)
 * size = sizeof(fake_mode_connector_t) = 80 = 0x50 */
#define EP122_SHIM_GETCONNECTOR_IOC 0xC05064A7u

static drm_api_connector_t *_get_connector_real(int fd, uint32_t id) {
    fake_mode_connector_t kc;
    memset(&kc, 0, sizeof(kc));
    kc.connector_id = id;

    if (sys_ioctl(fd, EP122_SHIM_GETCONNECTOR_IOC, &kc) < 0)
        return NULL;

    uint32_t nm = kc.count_modes, ne = kc.count_encoders, np = kc.count_props;

    drm_api_modeinfo_t *modes = (drm_api_modeinfo_t *)calloc(nm ? nm : 1, sizeof(*modes));
    uint32_t           *encs  = (uint32_t *)calloc(ne ? ne : 1, sizeof(uint32_t));
    uint32_t           *props = (uint32_t *)calloc(np ? np : 1, sizeof(uint32_t));
    uint64_t           *pvals = (uint64_t *)calloc(np ? np : 1, sizeof(uint64_t));
    if (!modes || !encs || !props || !pvals) {
        free(modes); free(encs); free(props); free(pvals);
        return NULL;
    }

    kc.modes_ptr       = (uint64_t)(uintptr_t)modes;
    kc.encoders_ptr    = (uint64_t)(uintptr_t)encs;
    kc.props_ptr       = (uint64_t)(uintptr_t)props;
    kc.prop_values_ptr = (uint64_t)(uintptr_t)pvals;

    if (sys_ioctl(fd, EP122_SHIM_GETCONNECTOR_IOC, &kc) < 0) {
        free(modes); free(encs); free(props); free(pvals);
        return NULL;
    }

    drm_api_connector_t *c = (drm_api_connector_t *)calloc(1, sizeof(*c));
    if (!c) { free(modes); free(encs); free(props); free(pvals); return NULL; }

    c->connector_id      = kc.connector_id;
    c->encoder_id        = kc.encoder_id;
    if (c->encoder_id == 0 && ne > 0)
        c->encoder_id = encs[0];
    c->connector_type    = kc.connector_type;
    c->connector_type_id = kc.connector_type_id;
    c->connection        = DRM_API_MODE_CONNECTED;
    c->mmWidth           = kc.mm_width;
    c->mmHeight          = kc.mm_height;
    c->subpixel          = kc.subpixel;
    c->count_modes       = (int)nm;
    c->modes             = modes;
    c->count_props       = (int)np;
    c->props             = props;
    c->prop_values       = pvals;
    c->count_encoders    = (int)ne;
    c->encoders          = encs;

#define DRM_MODE_CONNECTOR_DSI_TYPE 16u
    if (c->connector_type == DRM_MODE_CONNECTOR_DSI_TYPE &&
        c->connector_type_id == 2u &&
        c->count_modes == 0) {
        drm_api_modeinfo_t *syn = (drm_api_modeinfo_t *)calloc(1, sizeof(*syn));
        if (syn) {
            memcpy(syn, &g_fake_mode, sizeof(fake_modeinfo_t));
            free(modes);
            c->modes      = syn;
            c->count_modes = 1;
            fprintf(stderr,
                    "[ep122_shim] drmModeGetConnector(id=%u): synthesized "
                    "1280x240 preferred mode for jog DSI-2\n", id);
        }
    }

    DBG("drmModeGetConnector(fd=%d, id=%u) type=%u type_id=%u → forced connected\n",
        fd, id, c->connector_type, c->connector_type_id);
    return c;
}

drm_api_connector_t *drmModeGetConnectorCurrent(int fd, uint32_t connector_id) {
    return _get_connector_real(fd, connector_id);
}

drm_api_connector_t *drmModeGetConnector(int fd, uint32_t connector_id) {
    return _get_connector_real(fd, connector_id);
}

void drmModeFreeConnector(drm_api_connector_t *ptr) {
    if (!ptr) return;
    free(ptr->modes);
    free(ptr->props);
    free(ptr->prop_values);
    free(ptr->encoders);
    free(ptr);
}

/* ------------------------------------------------------------------ */
/* drmModeGetEncoder                                                  */
/* ------------------------------------------------------------------ */
/* DRM_IOCTL_MODE_GETENCODER = _IOWR('d', 0xa6, fake_mode_encoder_t)
 * sizeof(fake_mode_encoder_t) = 20 = 0x14 */
#define EP122_SHIM_GETENCODER_IOC   0xC01464A6u

/* DRM_IOCTL_MODE_GETRESOURCES = _IOWR('d', 0xa0, ...)
 * sizeof(struct drm_mode_card_res) = 64 = 0x40 */
#define EP122_SHIM_GETRESOURCES_IOC 0xC04064A0u

drm_api_encoder_t *drmModeGetEncoder(int fd, uint32_t encoder_id) {
    fake_mode_encoder_t ke;
    memset(&ke, 0, sizeof(ke));
    ke.encoder_id = encoder_id;

    if (sys_ioctl(fd, EP122_SHIM_GETENCODER_IOC, &ke) < 0) {
        DBG("drmModeGetEncoder(fd=%d, id=%u): GETENCODER failed\n", fd, encoder_id);
        return NULL;
    }

    drm_api_encoder_t *e = (drm_api_encoder_t *)calloc(1, sizeof(*e));
    if (!e) return NULL;

    e->encoder_id     = ke.encoder_id;
    e->encoder_type   = ke.encoder_type;
    e->crtc_id        = ke.crtc_id;
    e->possible_crtcs = ke.possible_crtcs;
    e->possible_clones= ke.possible_clones;

    if (e->crtc_id == 0 && (e->possible_crtcs & 2u)) {
        fake_mode_res_t res;
        memset(&res, 0, sizeof(res));

        if (sys_ioctl(fd, EP122_SHIM_GETRESOURCES_IOC, &res) == 0 &&
            res.count_crtcs >= 2) {
            uint32_t *crtc_ids = (uint32_t *)calloc(res.count_crtcs, sizeof(uint32_t));
            if (crtc_ids) {
                res.crtc_id_ptr = (uint64_t)(uintptr_t)crtc_ids;
                res.fb_id_ptr        = (uint64_t)(uintptr_t)calloc(res.count_fbs        ? res.count_fbs        : 1, sizeof(uint32_t));
                res.connector_id_ptr = (uint64_t)(uintptr_t)calloc(res.count_connectors ? res.count_connectors : 1, sizeof(uint32_t));
                res.encoder_id_ptr   = (uint64_t)(uintptr_t)calloc(res.count_encoders   ? res.count_encoders   : 1, sizeof(uint32_t));

                if (sys_ioctl(fd, EP122_SHIM_GETRESOURCES_IOC, &res) == 0) {
                    e->crtc_id = crtc_ids[1];
                } else {
                    fprintf(stderr,
                            "[ep122_shim] drmModeGetEncoder: GETRESOURCES phase2 failed: %s\n",
                            strerror(errno));
                }
                free((void *)(uintptr_t)res.fb_id_ptr);
                free((void *)(uintptr_t)res.connector_id_ptr);
                free((void *)(uintptr_t)res.encoder_id_ptr);
                free(crtc_ids);
            }
        } else if (e->crtc_id == 0) {
            fprintf(stderr,
                    "[ep122_shim] drmModeGetEncoder: GETRESOURCES phase1 failed or <2 CRTCs: %s\n",
                    strerror(errno));
        }
    }

    fprintf(stderr,
            "[ep122_shim] drmModeGetEncoder(id=%u) type=%u crtc_id=%u possible=0x%x\n",
            encoder_id, e->encoder_type, e->crtc_id, e->possible_crtcs);
    return e;
}

void drmModeFreeEncoder(drm_api_encoder_t *ptr) {
    free(ptr);
}
