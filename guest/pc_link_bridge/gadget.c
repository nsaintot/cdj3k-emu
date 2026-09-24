// SPDX-License-Identifier: MIT OR Apache-2.0
/* gadget.c: the gadget identity, read back from configfs.
 *
 * The firmware's usb_gadget.sh (patch 28 keeps its identity lines) is the one
 * place a model's USB identity is defined.  The host builds its virtual HID
 * device and MIDI device from what this reports, so both match the gadget the
 * app talks to.
 *
 * Payload: one `key=value` line per field, report_desc as lowercase hex.
 *   idVendor=0x2b73
 *   idProduct=0x004e
 *   bcdDevice=0x0140
 *   manufacturer=AlphaTheta Corporation
 *   product=CDJ-3000X
 *   serialnumber=DJMP000004EH
 *   report_desc=06a0ff0901... */

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

#include "bridge.h"

/* Read `path` into `buf` (at most `cap` bytes).  Returns the byte count, or
 * -1 when the file cannot be read. */
static ssize_t read_file(const char *path, void *buf, size_t cap)
{
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) return -1;
    size_t got = 0;
    while (got < cap) {
        ssize_t n = read(fd, (char *)buf + got, cap - got);
        if (n > 0) { got += (size_t)n; continue; }
        if (n < 0 && errno == EINTR) continue;
        if (n < 0) { close(fd); return -1; }
        break;
    }
    close(fd);
    return (ssize_t)got;
}

/* Append `key=<file contents, trailing newline stripped>\n`.  Returns the new
 * length, or -1 if the file is missing or the buffer is full. */
static int add_text(char *out, size_t cap, int len, const char *key, const char *file)
{
    char path[160], val[128];
    snprintf(path, sizeof path, "%s/%s", GADGET_DIR, file);
    ssize_t n = read_file(path, val, sizeof val - 1);
    if (n < 0) return -1;
    val[n] = 0;
    val[strcspn(val, "\n")] = 0;
    int w = snprintf(out + len, cap - (size_t)len, "%s=%s\n", key, val);
    if (w < 0 || (size_t)w >= cap - (size_t)len) return -1;
    return len + w;
}

int gadget_identity(char *out, size_t cap)
{
    static const struct { const char *key, *file; } k_text[] = {
        { "idVendor",     "idVendor" },
        { "idProduct",    "idProduct" },
        { "bcdDevice",    "bcdDevice" },
        { "manufacturer", "strings/0x409/manufacturer" },
        { "product",      "strings/0x409/product" },
        { "serialnumber", "strings/0x409/serialnumber" },
    };
    int len = 0;
    for (size_t i = 0; i < sizeof k_text / sizeof k_text[0]; i++) {
        len = add_text(out, cap, len, k_text[i].key, k_text[i].file);
        if (len < 0) {
            fprintf(stderr, "pc-link-bridge: gadget %s unreadable\n", k_text[i].file);
            return -1;
        }
    }

    unsigned char desc[512];
    ssize_t n = read_file(GADGET_DIR "/functions/hid.usb0/report_desc", desc, sizeof desc);
    if (n <= 0) {
        fprintf(stderr, "pc-link-bridge: gadget report_desc unreadable\n");
        return -1;
    }
    int w = snprintf(out + len, cap - (size_t)len, "report_desc=");
    if (w < 0 || (size_t)w >= cap - (size_t)len) return -1;
    len += w;
    if ((size_t)len + (size_t)n * 2 + 2 > cap) return -1;
    for (ssize_t i = 0; i < n; i++)
        len += snprintf(out + len, cap - (size_t)len, "%02x", desc[i]);
    out[len++] = '\n';
    return len;
}
