// SPDX-License-Identifier: MIT OR Apache-2.0
/* io.c: endpoint discovery (hidraw, USB-Audio rawmidi) and the cdj3k.usb-link
 * wire protocol.  See bridge.h for the frame layout. */

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include "bridge.h"

void msleep(int ms)
{
    struct timespec ts = { .tv_sec = ms / 1000, .tv_nsec = (long)(ms % 1000) * 1000000L };
    nanosleep(&ts, NULL);
}

/* Block until `path` exists and is openable for O_RDWR, or until SIGTERM. */
int wait_open(const char *path, int extra_flags)
{
    for (;;) {
        if (g_stop) return -1;
        int fd = open(path, O_RDWR | O_CLOEXEC | extra_flags);
        if (fd >= 0) return fd;
        if (errno != ENOENT && errno != EACCES) {
            fprintf(stderr, "pc-link-bridge: open(%s) failed: %s\n",
                    path, strerror(errno));
        }
        msleep(500);
    }
}

/* Find the rawmidi char device for the gadget as seen from the guest's own USB
 * host side.
 *
 * Two ALSA cards expose the same gadget.  The f_midi card is the *gadget*
 * side and the app owns it: its ALSA sequencer client subscribes to f_midi,
 * which holds the rawmidi output substream, so opening that one returns
 * EBUSY.  The snd-usb-audio card is the *host* side of the dummy_hcd bus -
 * the MIDI counterpart of /dev/hidraw0 - and nothing else claims it.
 *
 * /proc/asound/cards names the driver after the card index:
 *   2 [CDJ3000        ]: USB-Audio - CDJ-3000
 * so the card carrying "USB-Audio" is the one to open.  Returns the card
 * number, or -1 when it has not appeared yet.
 */
static int find_usb_midi_card(void)
{
    FILE *f = fopen(ASOUND_CARDS, "r");
    if (!f) return -1;
    char line[256];
    int last = -1, found = -1;
    while (fgets(line, sizeof line, f)) {
        int n;
        if (sscanf(line, " %d [", &n) == 1) last = n;
        if (last >= 0 && strstr(line, "USB-Audio")) { found = last; break; }
    }
    fclose(f);
    return found;
}

/* Wait up to `timeout_ms` for that card, then open its first rawmidi device.
 * Returns the fd, or -1 to run HID-only. */
int open_usb_midi(int timeout_ms)
{
    for (int waited = 0; waited <= timeout_ms; waited += 200) {
        if (g_stop) return -1;
        int card = find_usb_midi_card();
        if (card >= 0) {
            char path[64];
            snprintf(path, sizeof path, "/dev/snd/midiC%dD0", card);
            int fd = open(path, O_RDWR | O_CLOEXEC | O_NONBLOCK);
            if (fd >= 0) {
                fprintf(stderr, "pc-link-bridge: MIDI on %s (card %d)\n", path, card);
                return fd;
            }
            /* The card registers before devtmpfs creates its rawmidi node,
             * so a missing node is worth waiting out.  Anything else (EBUSY,
             * EACCES) will not clear within the budget and would only delay
             * the HID pump, which starts once this returns. */
            if (errno != ENOENT && errno != ENODEV) {
                fprintf(stderr, "pc-link-bridge: open(%s) failed: %s\n",
                        path, strerror(errno));
                return -1;
            }
            if (waited == 0)
                fprintf(stderr, "pc-link-bridge: %s not there yet - waiting\n", path);
        }
        msleep(200);
    }
    fprintf(stderr, "pc-link-bridge: no USB-Audio card after %d ms"
                    " - continuing HID-only\n", timeout_ms);
    return -1;
}

/* Write exactly `len` bytes (handles short writes + EAGAIN spin-waits).
 * Returns 0 on success, -1 on hard error or stop. */
static int write_all(int fd, const void *buf, size_t len)
{
    const uint8_t *p = buf;
    size_t left = len;
    while (left > 0) {
        ssize_t n = write(fd, p, left);
        if (n > 0) {
            p += n;
            left -= (size_t)n;
            continue;
        }
        if (n < 0 && (errno == EAGAIN || errno == EINTR)) {
            if (g_stop) return -1;
            /* short pause so we don't pin a CPU on a slow consumer */
            msleep(1);
            continue;
        }
        return -1;
    }
    return 0;
}

/* Read at most `cap` bytes from a non-blocking fd. Returns bytes read, 0 if
 * nothing available, -1 on error. */
ssize_t read_some(int fd, void *buf, size_t cap)
{
    ssize_t n = read(fd, buf, cap);
    if (n >= 0) return n;
    if (errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR) return 0;
    return -1;
}

/* Frame and send a payload out the virtio-serial port. */
int send_frame(int vport, uint8_t type, const uint8_t *payload, size_t len)
{
    if (len > MAX_PAYLOAD) len = MAX_PAYLOAD;
    uint8_t hdr[3];
    hdr[0] = type;
    hdr[1] = (uint8_t)((len >> 8) & 0xff);
    hdr[2] = (uint8_t)(len & 0xff);
    if (write_all(vport, hdr, sizeof hdr) < 0) return -1;
    if (len > 0 && write_all(vport, payload, len) < 0) return -1;
    return 0;
}

/* Read exactly `len` bytes from a blocking fd (handles short reads). Returns
 * 0 on success, -1 on EOF/error. */
static int read_exact(int fd, void *buf, size_t len)
{
    uint8_t *p = buf;
    size_t left = len;
    while (left > 0) {
        if (g_stop) return -1;
        ssize_t n = read(fd, p, left);
        if (n > 0) { p += n; left -= (size_t)n; continue; }
        if (n == 0) return -1;  /* EOF (host closed) */
        if (errno == EINTR) continue;
        return -1;
    }
    return 0;
}

/* Receive one full frame from the virtio-serial port and dispatch it to
 * the right gadget-side fd.  Returns 0 on success, -1 on EOF/error. */
int recv_and_dispatch(int vport, int hidraw_fd, int midi_fd)
{
    uint8_t hdr[3];
    if (read_exact(vport, hdr, sizeof hdr) < 0) return -1;
    uint16_t len = ((uint16_t)hdr[1] << 8) | hdr[2];
    if (len > MAX_PAYLOAD) {
        fprintf(stderr, "pc-link-bridge: oversized frame len=%u type=0x%02x — draining\n",
                (unsigned)len, hdr[0]);
        /* Drain the bad frame so the stream stays aligned. */
        uint8_t scratch[1024];
        while (len > 0) {
            size_t chunk = len < sizeof scratch ? len : sizeof scratch;
            if (read_exact(vport, scratch, chunk) < 0) return -1;
            len -= (uint16_t)chunk;
        }
        return 0;
    }
    uint8_t payload[MAX_PAYLOAD];
    if (len > 0 && read_exact(vport, payload, len) < 0) return -1;

    if (hdr[0] == FRAME_HELLO) {
        char id[MAX_PAYLOAD];
        int n = gadget_identity(id, sizeof id);
        if (n < 0) return 0;
        if (send_frame(vport, FRAME_IDENTITY, (const uint8_t *)id, (size_t)n) < 0) return -1;
        fprintf(stderr, "pc-link-bridge: identity sent (%d bytes)\n", n);
        return 0;
    }

    int dst_fd;
    const char *what;
    switch (hdr[0]) {
        case FRAME_HID:  dst_fd = hidraw_fd; what = "HID";  break;
        case FRAME_MIDI: dst_fd = midi_fd;   what = "MIDI"; break;
        default:
            fprintf(stderr, "pc-link-bridge: unknown frame type 0x%02x — dropping\n",
                    hdr[0]);
            return 0;
    }
    /* Silently drop frames for endpoints we couldn't open (e.g. MIDI when
     * the app holds the raw char device exclusive). */
    if (dst_fd < 0) return 0;
    if (len == 0) return 0;

    if (hdr[0] == FRAME_HID) {
        /* hidraw_write() reads the first byte as the report number.  The
         * Pioneer descriptors declare no report IDs, so that byte is 0 and
         * is not part of the report; the wire payload carries the report
         * alone, in both directions. */
        uint8_t out[1 + MAX_PAYLOAD];
        out[0] = 0x00;
        memcpy(out + 1, payload, len);
        if (write_all(dst_fd, out, (size_t)len + 1) < 0) {
            fprintf(stderr, "pc-link-bridge: write to %s failed: %s\n",
                    what, strerror(errno));
        }
        return 0;
    }

    if (write_all(dst_fd, payload, len) < 0) {
        fprintf(stderr, "pc-link-bridge: write to %s failed: %s\n",
                what, strerror(errno));
    }
    return 0;
}
