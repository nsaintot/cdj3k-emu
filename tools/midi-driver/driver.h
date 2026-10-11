// SPDX-License-Identifier: MIT OR Apache-2.0
/* Shared declarations.
 *
 *   plugin.c  CFPlugIn factory and the MIDIDriverInterface entry points
 *   device.c  publishing one MIDIDevice per slot
 *   link.c    instance sockets, the identity, and the pump that owns the slots
 */
#ifndef CDJ3K_MIDI_DRIVER_H
#define CDJ3K_MIDI_DRIVER_H

#include <CoreMIDI/MIDIDriver.h>
#include <CoreMIDI/CoreMIDI.h>
#include <CoreFoundation/CoreFoundation.h>
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <stdarg.h>
#include <stdlib.h>
#include <time.h>
#include <pthread.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
#include <errno.h>
#include <poll.h>
#include <dirent.h>
#include <sys/time.h>

/* Link to the emulator: a raw MIDI byte stream over a unix socket.  Each
 * emulator instance binds its own at instance-<id>/midi-driver.sock and we
 * dial every one we find, because MIDIServer outlives any emulator run and
 * several instances can be up at once.  Each connection is one slot and one
 * MIDIDevice, and the device lives as long as the connection. */
#define MAX_SLOTS 8
/* SCAN_SECS is the rescan interval and how long an idle pump sleeps between
 * wakes.  It bounds
 * only how long a newly toggled slot waits to be noticed; MIDI arrives through
 * poll(), which returns the moment a byte lands. */
#define SCAN_SECS    2
#define POLL_TIMEOUT (SCAN_SECS * 1000)

struct slot {
    int             fd;       /* -1 when free */
    uint32_t        gen;      /* bumped on every reuse; encoded in the refCon */
    char            path[128];
    MIDIDeviceRef   dev;
    MIDIEndpointRef src;      /* guest to host */
    /* The identity the emulator sent: the guest gadget's USB identity and the
     * instance's LocationID, the same values that the matching HID device
     * carries. */
    uint32_t        location;
    uint32_t        usb_vp;   /* (vid << 16) | pid */
    CFStringRef     name;     /* product, released in slot_detach */
    CFStringRef     maker;    /* manufacturer, released in slot_detach */
    /* State for assembling guest-to-host bytes into whole messages.  The bytes
     * arrive as an unframed stream, and a MIDIPacket must carry whole
     * messages. */
    Byte            rs;       /* running status, 0 when none */
    Byte            msg[3];   /* message in progress; msg[0] == 0 when none */
    Byte            have;     /* data bytes held in msg[1..] */
    Byte            need;     /* data bytes msg[0] takes */
    Byte            in_sx;    /* inside a SysEx */
};
/* The refCon holds the generation in the high bits and the slot index plus
 * one in the low 8 bits.  A
 * Send() arriving on an endpoint whose slot has since been reused carries the
 * old generation and is dropped. */
#define REF_MAKE(gen, idx) ((void *)(uintptr_t)(((uint64_t)(gen) << 8) | ((idx) + 1)))
#define REF_IDX(r)         ((int)((uintptr_t)(r) & 0xff) - 1)
#define REF_GEN(r)         ((uint32_t)((uintptr_t)(r) >> 8))
/* MARKER tags this driver's devices, so cleanup removes only its own leftovers
 * and never real hardware. */
#define MARKER CFSTR("cdj3kEmuVirtual")

/* Identity the emulator sends on connect; the raw MIDI stream follows it. */
#define IDENTITY_MAGIC   0x43444A31u   /* 'CDJ1' */
#define IDENTITY_VERSION 1
struct pc_link_identity {
    uint32_t magic;
    uint16_t version;
    uint16_t instance;
    uint16_t vid, pid;
    uint32_t location;
    char     product[32];
    char     serial[32];
    char     manufacturer[32];
};

/* Defined in plugin.c.  MIDIServer gives the plugin no console, so log messages
 * go to Console and to `log stream --predicate 'subsystem == "com.cdj3k.emu"'`.
 * Lifecycle transitions log at the default level and failures at the error
 * level; nothing is logged per packet. */
#include <os/log.h>
extern os_log_t g_log;

extern MIDIDriverRef g_owner;
/* Whether the run that began at `epoch` is still current: false once Stop()
 * has been called, even if a Start() followed it. */
int driver_live(uint32_t epoch);
/* Blocks the pump until the driver is started; returns the run's epoch. */
uint32_t driver_wait_started(void);

/* device.c */
void configure_device(MIDIDeviceRef dev, const struct slot *sl);
void remove_stale_devices(void);
int  create_slot_device(struct slot *sl, int index);
MIDIDeviceRef slot_detach(struct slot *sl);
void slot_retire(MIDIDeviceRef dev, uint32_t location);
/* The number of bytes read per pump wake.  It bounds the packet list that
 * slot_parse fills. */
#define READ_CHUNK 256
/* This fits the worst case: one packet per input byte, each with up to 3 data
 * bytes. */
#define PARSE_LIST_BYTES 4096
int  slot_parse(struct slot *sl, const unsigned char *buf, int n,
                MIDIPacketList *pl, size_t cap);

/* link.c */
void *pump(void *arg);
void  link_send(int idx, uint32_t gen, const MIDIPacketList *pl);

#endif
