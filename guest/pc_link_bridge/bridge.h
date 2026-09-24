// SPDX-License-Identifier: MIT OR Apache-2.0
/*
 * pc_link_bridge: bidirectional HID + MIDI pump between the dummy_hcd-
 * attached USB gadget (inside this guest) and the cdj3k.usb-link virtio-
 * serial port (out to cdj3k-emu-runtime on the Mac).
 *
 * Direction summary (HID; MIDI is symmetric):
 *
 *   app    →  /dev/hidg0  ─[ f_hid + dummy_hcd ]─→  /dev/hidraw0
 *                                                   └─ this daemon reads
 *                                                        and frames out to
 *                                                        cdj3k.usb-link
 *                                                        → host CoreHID
 *                                                          (virtual HID dev)
 *
 *   host CoreHID ← virtual HID dev → cdj3k.usb-link → this daemon
 *      writes to /dev/hidraw0 ─[ dummy_hcd ]→ /dev/hidg0 ← app read
 *
 * The wire payload is the bare HID report in both directions.  The leading
 * report-number byte hidraw_write() expects is added in io.c, next to the fd
 * that needs it.
 *
 * Wire frame (over virtio-serial; both directions identical):
 *   byte 0       : type    (0x01=HID, 0x02=MIDI, 0x03=HELLO, 0x04=IDENTITY)
 *   bytes 1..2   : length  (big-endian u16, payload only)
 *   bytes 3..    : payload
 *
 * The host opens every connection with an empty HELLO and waits for the
 * IDENTITY reply (gadget.c) before registering its endpoints, so the macOS
 * devices carry the identity the firmware gave the gadget.
 *
 * HID reports are as long as the gadget's report descriptor says (64 bytes on
 * the CDJ-3000, up to 1024 on the CDJ-3000X) and USB-MIDI uses 4-byte event
 * packets, so a 4 KiB framing buffer covers both.
 *
 * The systemd unit (installed by initramfs patch 29) is NOT enabled by
 * default; cfgd starts/stops it when the host toggles `pc_link` on/off, which
 * keeps the bridge silent until the user plugs the "virtual cable" in.
 *
 * Translation units:
 *   io.c      endpoint discovery + the virtio-serial wire protocol
 *   gadget.c  the gadget identity read back from configfs
 *   pcmode.c  force PC mode on apps that cannot enter it themselves
 *   usbg.c    the /tmp/usbg1 FIFO that gates the SOURCE CONTROL MODE row
 *   main.c    startup, signal handling, the poll(2) pump
 */
#ifndef PC_LINK_BRIDGE_H
#define PC_LINK_BRIDGE_H

#include <signal.h>
#include <stddef.h>
#include <stdint.h>
#include <sys/types.h>

#define VPORT_PATH    "/dev/virtio-ports/cdj3k.usb-link"
#define HIDRAW_PATH   "/dev/hidraw0"
#define USBG_PATH     "/tmp/usbg1"
#define ASOUND_CARDS  "/proc/asound/cards"
#ifndef GADGET_DIR
#define GADGET_DIR    "/sys/kernel/config/usb_gadget/g1"
#endif

#define FRAME_HID   0x01
#define FRAME_MIDI  0x02
#define FRAME_HELLO    0x03
#define FRAME_IDENTITY 0x04
#define MAX_PAYLOAD 4096

/* Set by SIGTERM/SIGINT; every blocking wait loop polls it. */
extern volatile sig_atomic_t g_stop;

/* io.c: endpoint discovery and the virtio-serial wire protocol. */
void    msleep(int ms);
int     wait_open(const char *path, int extra_flags);
int     open_usb_midi(int timeout_ms);
ssize_t read_some(int fd, void *buf, size_t cap);
int     send_frame(int vport, uint8_t type, const uint8_t *payload, size_t len);
int     recv_and_dispatch(int vport, int hidraw_fd, int midi_fd);

/* gadget.c: the gadget identity as `key=value` lines; returns the length, or
 * -1 when configfs cannot be read or `cap` is too small. */
int gadget_identity(char *out, size_t cap);

/* pcmode.c: force PC mode on apps that cannot enter it themselves. */
void force_pc_mode(void);
void unforce_pc_mode(void);

/* usbg.c: the /tmp/usbg1 FIFO gating the app's SOURCE CONTROL MODE row. */
int  usbg_open(void);
void usbg_close(void);
void set_usb_connected(int on);

#endif /* PC_LINK_BRIDGE_H */
