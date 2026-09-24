// SPDX-License-Identifier: MIT OR Apache-2.0
/* main.c: startup, signal handling, and the poll(2) pump.  See bridge.h for
 * the overall data path and the translation-unit map. */

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

#include "bridge.h"

volatile sig_atomic_t g_stop;
static void on_signal(int sig) { (void)sig; g_stop = 1; }

int main(void)
{
    /* Line-buffer stderr so journald sees each log line promptly. */
    setvbuf(stderr, NULL, _IOLBF, 0);

    struct sigaction sa = { .sa_handler = on_signal };
    sigemptyset(&sa.sa_mask);
    sigaction(SIGTERM, &sa, NULL);
    sigaction(SIGINT,  &sa, NULL);
    signal(SIGPIPE, SIG_IGN);

    fprintf(stderr, "pc-link-bridge: starting\n");

    int vport_fd  = wait_open(VPORT_PATH,  0);
    if (vport_fd  < 0) return 0;
    int hidraw_fd = wait_open(HIDRAW_PATH, O_NONBLOCK);
    if (hidraw_fd < 0) return 0;
    /* MIDI is best-effort: without the guest-side USB-Audio card there is
     * nothing to bridge and `midi_fd = -1` short-circuits the MIDI legs of
     * the pump.  snd-usb-audio probes after the gadget binds, so give it a
     * few seconds. */
    int midi_fd   = open_usb_midi(5000);

    fprintf(stderr, "pc-link-bridge: vport=%d hidraw=%d midi=%d — pumping\n",
            vport_fd, hidraw_fd, midi_fd);

    /* Signal USB-B connect (SOURCE CONTROL MODE row) and force PC mode. */
    usbg_open();
    set_usb_connected(1);
    force_pc_mode();

    /* Poll up to three fds for input.  When MIDI is unavailable (EBUSY at
     * open), the third slot is skipped; poll(fd=-1) is a documented no-op
     * but we'd rather not feed -1 to the kernel, so we collapse the array. */
    while (!g_stop) {
        struct pollfd pfds[3] = {
            { .fd = vport_fd,  .events = POLLIN },
            { .fd = hidraw_fd, .events = POLLIN },
            { .fd = midi_fd >= 0 ? midi_fd : -1, .events = POLLIN },
        };
        nfds_t nfds = (midi_fd >= 0) ? 3 : 2;
        int rc = poll(pfds, nfds, 1000);
        if (rc < 0) {
            if (errno == EINTR) continue;
            fprintf(stderr, "pc-link-bridge: poll failed: %s\n", strerror(errno));
            break;
        }
        if (rc == 0) continue;

        /* host → guest */
        if (pfds[0].revents & POLLIN) {
            if (recv_and_dispatch(vport_fd, hidraw_fd, midi_fd) < 0) {
                /* read_exact returned EOF or hard error.  Treat as
                 * "host disconnected"; back off and keep polling rather
                 * than exit, so the host's next PcLink::start lands on a
                 * live daemon (avoids a systemd-restart race where the
                 * bridge isn't yet open when the host tries to connect). */
                msleep(200);
                continue;
            }
        }
        if (pfds[0].revents & (POLLHUP | POLLERR)) {
            /* virtio_console raises POLLHUP whenever the host port has no
             * client attached.  Don't exit; back off briefly so we don't
             * spin on the immediate-return poll, then re-poll for a host
             * to (re)connect.  systemd's Restart=always would otherwise
             * tear us down + bring us back on a 2s loop; that race is
             * exactly how the host's PcLink::start ends up connecting to
             * a moment when no guest reader is present. */
            msleep(200);
            continue;
        }

        /* An endpoint that hangs up keeps poll() returning immediately, so a
         * dead hidraw or MIDI fd is dropped from the set rather than spun on.
         * Unbinding the gadget from its UDC does this. */
        for (int i = 1; i <= 2; i++) {
            if (pfds[i].fd >= 0 && (pfds[i].revents & (POLLERR | POLLHUP | POLLNVAL))) {
                fprintf(stderr, "pc-link-bridge: %s endpoint hung up; dropping it\n",
                        i == 1 ? "hidraw" : "MIDI");
                close(pfds[i].fd);
                pfds[i].fd = -1;
                if (i == 1) hidraw_fd = -1; else midi_fd = -1;
            }
        }
        if (pfds[1].fd < 0 && pfds[2].fd < 0) {
            fprintf(stderr, "pc-link-bridge: no endpoints left; exiting\n");
            break;
        }

        /* guest → host: drain whatever's ready, framing per read. */
        uint8_t buf[MAX_PAYLOAD];
        if (pfds[1].revents & POLLIN) {
            ssize_t n = read_some(hidraw_fd, buf, sizeof buf);
            if (n > 0) {
                if (send_frame(vport_fd, FRAME_HID, buf, (size_t)n) < 0) {
                    fprintf(stderr, "pc-link-bridge: HID frame send failed\n");
                    break;
                }
            } else if (n < 0) {
                fprintf(stderr, "pc-link-bridge: hidraw read failed: %s\n",
                        strerror(errno));
            }
        }
        if (midi_fd >= 0 && (pfds[2].revents & POLLIN)) {
            ssize_t n = read_some(midi_fd, buf, sizeof buf);
            if (n > 0) {
                if (send_frame(vport_fd, FRAME_MIDI, buf, (size_t)n) < 0) {
                    fprintf(stderr, "pc-link-bridge: MIDI frame send failed\n");
                    break;
                }
            } else if (n < 0) {
                fprintf(stderr, "pc-link-bridge: midi read failed: %s\n",
                        strerror(errno));
            }
        }
    }

    set_usb_connected(0);
    /* Let the detector read the disconnect (1 Hz poll) before unlinking. */
    msleep(1200);
    usbg_close();
    unforce_pc_mode();
    close(vport_fd);
    close(hidraw_fd);
    if (midi_fd >= 0) close(midi_fd);
    fprintf(stderr, "pc-link-bridge: exiting\n");
    return 0;
}
