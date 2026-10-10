// SPDX-License-Identifier: MIT OR Apache-2.0
/*
 * pc_link_bridge copies HID and MIDI traffic in both directions between the
 * USB gadget that dummy_hcd attaches inside this guest and the cdj3k.usb-link
 * virtio-serial port, which leads to cdj3k-emu-runtime on the host.
 *
 * The HID data path; MIDI works the same way:
 *
 *   Guest to host: the app writes a report to /dev/hidg0, and f_hid on
 *   dummy_hcd delivers it to /dev/hidraw0. This daemon reads it and sends it
 *   as a frame over cdj3k.usb-link to the host's virtual HID device.
 *
 *   Host to guest: this daemon receives a frame from cdj3k.usb-link and
 *   writes the report to /dev/hidraw0; dummy_hcd delivers it, and the app
 *   reads it from /dev/hidg0.
 *
 * The wire payload is the bare HID report in both directions. io.c adds the
 * leading report-number byte that hidraw_write() expects, next to the fd that
 * needs it.
 *
 * The wire frame is the same in both directions:
 *   | Bytes | Field                                                        |
 *   |-------|--------------------------------------------------------------|
 *   | 0     | type: 0x01 HID, 0x02 MIDI, 0x03 HELLO, 0x04 IDENTITY         |
 *   | 1-2   | payload length, big-endian u16                               |
 *   | 3-    | payload                                                      |
 *
 * The host opens every connection with an empty HELLO and waits for the
 * IDENTITY reply (gadget.c) before it registers its endpoints, so the host's
 * devices carry the identity that the firmware gave the gadget.
 *
 * HID reports are as long as the gadget's report descriptor says (64 bytes on
 * the CDJ-3000, up to 1024 on the CDJ-3000X) and USB-MIDI uses 4-byte event
 * packets, so a 4 KiB framing buffer covers both.
 *
 * The systemd unit, which initramfs patch 29 installs, is not enabled by
 * default. cfgd starts or stops it when the host toggles `pc_link`, so the
 * bridge stays silent until the user plugs in the virtual cable.
 *
 * Source files:
 *   io.c      endpoint discovery and the virtio-serial wire protocol
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

/* SIGTERM and SIGINT set this; every blocking wait loop checks it. */
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

/* usbg.c: the /tmp/usbg1 FIFO that gates the app's SOURCE CONTROL MODE row. */
int  usbg_open(void);
void usbg_close(void);
void set_usb_connected(int on);

#endif /* PC_LINK_BRIDGE_H */
