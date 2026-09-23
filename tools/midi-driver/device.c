// SPDX-License-Identifier: MIT OR Apache-2.0
/* device.c: one MIDIDevice per connected slot, named and identified from that
 * instance's id. */
#include "driver.h"

/* CDJ-3000 identity: VID+PID (usbVendorProduct) + LocationID pair the MIDI
 * device with its HID sibling; the USB id selects the built-in CDJ-3000 map.
 * `location` comes from that instance's identity and equals the LocationID on
 * the HID device it registered. */
void configure_device(MIDIDeviceRef dev, const struct slot *sl) {
    uint32_t location = sl->location;
    CFStringRef name = sl->name;
    MIDIObjectSetIntegerProperty(dev, CFSTR("USBLocationID"), location);
    MIDIObjectSetIntegerProperty(dev, CFSTR("usbVendorProduct"), sl->usb_vp);
    MIDIObjectSetIntegerProperty(dev, CFSTR("USBVendorProduct"), sl->usb_vp);
    MIDIObjectSetStringProperty(dev, kMIDIPropertyName, name);
    for (ItemCount e = 0; e < MIDIDeviceGetNumberOfEntities(dev); e++) {
        MIDIEntityRef en = MIDIDeviceGetEntity(dev, e);
        MIDIObjectSetStringProperty(en, kMIDIPropertyName, name);
        MIDIObjectSetIntegerProperty(en, CFSTR("usbVendorProduct"), sl->usb_vp);
        MIDIObjectSetIntegerProperty(en, CFSTR("USBLocationID"), location);
        for (ItemCount k = 0; k < MIDIEntityGetNumberOfSources(en); k++) {
            MIDIEndpointRef src = MIDIEntityGetSource(en, k);
            MIDIObjectSetStringProperty(src, kMIDIPropertyName, name);
            MIDIObjectSetIntegerProperty(src, CFSTR("usbVendorProduct"), sl->usb_vp);
            MIDIObjectSetIntegerProperty(src, CFSTR("USBLocationID"), location);
        }
        for (ItemCount k = 0; k < MIDIEntityGetNumberOfDestinations(en); k++)
            MIDIObjectSetStringProperty(MIDIEntityGetDestination(en, k), kMIDIPropertyName, name);
    }
    MIDIObjectSetIntegerProperty(dev, kMIDIPropertyOffline, 0);
    MIDIObjectSetStringProperty(dev, kMIDIPropertyModel, name);
    MIDIObjectSetIntegerProperty(dev, CFSTR("UMP Enabled"), 0);
}

/* Remove any device we left in the setup from a previous load/crash. */
void remove_stale_devices(void) {
    /* Collect first: MIDISetupRemoveDevice shifts the list, so removing while
     * walking it by index skips the entry after each hit. */
    MIDIDeviceRef doomed[MAX_SLOTS * 2];
    int n = 0;
    for (ItemCount i = 0; i < MIDIGetNumberOfDevices() && n < (int)(sizeof doomed / sizeof *doomed); i++) {
        MIDIDeviceRef dev = MIDIGetDevice(i);
        SInt32 mark = 0;
        if (MIDIObjectGetIntegerProperty(dev, MARKER, &mark) == noErr && mark == 1)
            doomed[n++] = dev;
    }
    for (int i = 0; i < n; i++) MIDISetupRemoveDevice(doomed[i]);
    if (n) os_log(g_log, "removed %d stale device(s)", n);
}

/* Publish a fresh device for one slot when its link comes up (the HID device
 * is already registered): the device-added event makes a DJ app open the HID
 * and pair the two. */
int create_slot_device(struct slot *sl, int index) {
    if (!g_owner) return -1;
    MIDIDeviceRef dev = 0;
    OSStatus st = MIDIDeviceCreate(g_owner, sl->name, sl->maker, sl->name, &dev);
    if (st != noErr || !dev) {
        os_log_error(g_log, "MIDIDeviceCreate failed (%d)", (int)st);
        return -1;
    }
    MIDIEntityRef ent = 0;
    if (MIDIDeviceAddEntity(dev, sl->name, true, 1, 1, &ent) != noErr) {
        MIDIDeviceDispose(dev);
        return -1;
    }
    MIDIObjectSetIntegerProperty(dev, MARKER, 1);
    configure_device(dev, sl);
    st = MIDISetupAddDevice(dev);
    if (st != noErr) {
        os_log_error(g_log, "MIDISetupAddDevice failed (%d)", (int)st);
        MIDIDeviceDispose(dev);
        return -1;
    }
    sl->dev = dev;
    if (MIDIDeviceGetNumberOfEntities(dev) > 0) {
        MIDIEntityRef en = MIDIDeviceGetEntity(dev, 0);
        if (MIDIEntityGetNumberOfSources(en) > 0)
            sl->src = MIDIEntityGetSource(en, 0);
        /* Send() is handed the destination's refCons, which is how a packet
         * finds the slot it belongs to without a lookup table. */
        for (ItemCount k = 0; k < MIDIEntityGetNumberOfDestinations(en); k++)
            MIDIEndpointSetRefCons(MIDIEntityGetDestination(en, k),
                                   REF_MAKE(sl->gen, index), NULL);
    }
    os_log(g_log, "slot %d device up, location %u", index, (unsigned)sl->location);
    return 0;
}

/* Unpublish one slot's device when its link drops (like unplugging a CDJ). */
/* Detach the slot and hand back the device it published, if any.  The caller
 * retires that device with the slot lock released: MIDISetupRemoveDevice takes
 * MIDIServer's own setup lock, and MIDIServer holds that lock while calling
 * into this driver. */
MIDIDeviceRef slot_detach(struct slot *sl) {
    MIDIDeviceRef dev = sl->dev;
    if (sl->fd >= 0) close(sl->fd);
    if (sl->name)  { CFRelease(sl->name);  sl->name = NULL; }
    if (sl->maker) { CFRelease(sl->maker); sl->maker = NULL; }
    sl->fd = -1; sl->dev = 0; sl->src = 0; sl->path[0] = 0;
    sl->rs = 0; sl->msg[0] = 0; sl->have = 0; sl->need = 0; sl->in_sx = 0;
    return dev;
}

/* Retire a device handed back by slot_detach.  Never called with the slot
 * lock held. */
void slot_retire(MIDIDeviceRef dev, uint32_t location) {
    if (!dev) return;
    os_log(g_log, "device down, location %u", (unsigned)location);
    MIDISetupRemoveDevice(dev);
}

/* Data bytes that follow a status byte. */
static Byte data_len(Byte status) {
    switch (status & 0xf0) {
    case 0xc0: case 0xd0: return 1;
    case 0xf0: break;
    default:   return 2;
    }
    switch (status) {
    case 0xf1: case 0xf3: return 1;
    case 0xf2:            return 2;
    default:              return 0;
    }
}

struct emit { MIDIPacketList *pl; MIDIPacket *p; size_t cap; };

static void emit(struct emit *e, const Byte *b, int n) {
    if (!e->p || n <= 0) return;
    e->p = MIDIPacketListAdd(e->pl, e->cap, e->p, 0, (ByteCount)n, b);
}

/* Assemble bytes from one guest into whole messages in `pl`.  Running status
 * is expanded, a message split across reads is held in the slot until it
 * completes, and a SysEx is passed through in chunks.  Returns whether `pl`
 * holds any packet.  Called with the slot lock held. */
int slot_parse(struct slot *sl, const unsigned char *buf, int n,
               MIDIPacketList *pl, size_t cap) {
    struct emit e = { pl, MIDIPacketListInit(pl), cap };
    Byte sx[READ_CHUNK];
    int sxn = 0;
    for (int i = 0; i < n && i < READ_CHUNK; i++) {
        Byte b = buf[i];
        if (b >= 0xf8) {                         /* real-time, any position */
            emit(&e, sx, sxn); sxn = 0;
            emit(&e, &b, 1);
            continue;
        }
        if (b & 0x80) {
            if (sl->in_sx) {
                sl->in_sx = 0;
                if (b == 0xf7) { sx[sxn++] = b; emit(&e, sx, sxn); sxn = 0; continue; }
                emit(&e, sx, sxn); sxn = 0;      /* unterminated; status ends it */
            }
            if (b == 0xf0) { sl->in_sx = 1; sl->rs = 0; sl->msg[0] = 0; sx[sxn++] = b; continue; }
            if (b == 0xf7) continue;             /* stray EOX */
            sl->rs = b < 0xf0 ? b : 0;           /* system common clears it */
            sl->msg[0] = b; sl->have = 0; sl->need = data_len(b);
            if (sl->need == 0) { emit(&e, sl->msg, 1); sl->msg[0] = 0; }
            continue;
        }
        if (sl->in_sx) { sx[sxn++] = b; continue; }
        if (!sl->msg[0]) {
            if (!sl->rs) continue;               /* data with no status */
            sl->msg[0] = sl->rs; sl->have = 0; sl->need = data_len(sl->rs);
        }
        sl->msg[1 + sl->have++] = b;
        if (sl->have == sl->need) { emit(&e, sl->msg, 1 + sl->need); sl->msg[0] = 0; }
    }
    emit(&e, sx, sxn);                           /* SysEx continues next read */
    return pl->numPackets > 0;
}
