// SPDX-License-Identifier: MIT OR Apache-2.0
/* usbg.c: the /tmp/usbg1 FIFO.
 *
 * The app's meow::UsbHostPcConnectDetector, in EP122 and EP145 alike, polls
 * /proc/udev_usbg1, which the deck_shim open() hook redirects here, and reads
 * one event per poll. "connect" and "disconnect" change the SOURCE CONTROL
 * MODE row; anything else is ignored. The detector has no edge detection, so
 * the bridge uses a FIFO, which delivers one token per read. The bridge holds
 * the FIFO open O_RDWR so that the detector's open always succeeds. The shim
 * forces O_NONBLOCK on that open, so an empty FIFO returns EAGAIN and does
 * not block the detector between tokens.
 *
 * A FIFO is a byte stream: two tokens written close together can arrive in
 * one read, and the detector compares the whole buffer, so it matches
 * neither. */

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "bridge.h"

static int g_usbg_fd = -1;   /* The FIFO, held open while the bridge runs. */

int usbg_open(void)
{
    if (mkfifo(USBG_PATH, 0644) != 0 && errno != EEXIST) {
        fprintf(stderr, "pc-link-bridge: mkfifo %s failed: %s\n", USBG_PATH, strerror(errno));
        return -1;
    }
    g_usbg_fd = open(USBG_PATH, O_RDWR | O_NONBLOCK | O_CLOEXEC);
    if (g_usbg_fd < 0) {
        fprintf(stderr, "pc-link-bridge: open %s failed: %s\n", USBG_PATH, strerror(errno));
        return -1;
    }
    return 0;
}

void usbg_close(void)
{
    if (g_usbg_fd >= 0) { close(g_usbg_fd); g_usbg_fd = -1; }
    unlink(USBG_PATH);
}

/* Write one connect or disconnect event as the bare token without a newline,
 * because the detector compares the exact string. The FIFO delivers it in a
 * single read. */
void set_usb_connected(int on)
{
    if (g_usbg_fd < 0) return;
    const char *tok = on ? "connect" : "disconnect";
    size_t len = on ? 7 : 10;
    if (write(g_usbg_fd, tok, len) != (ssize_t)len)
        fprintf(stderr, "pc-link-bridge: write %s failed: %s\n", USBG_PATH, strerror(errno));
    else
        fprintf(stderr, "pc-link-bridge: sent the USB-B %s event through %s\n", tok, USBG_PATH);
}
