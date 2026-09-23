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
    return dev;
}

/* Retire a device handed back by slot_detach.  Never called with the slot
 * lock held. */
void slot_retire(MIDIDeviceRef dev, uint32_t location) {
    if (!dev) return;
    os_log(g_log, "device down, location %u", (unsigned)location);
    MIDISetupRemoveDevice(dev);
}

/* Publish bytes arriving from one guest on that slot's source endpoint. */
void emit_from_slot(struct slot *sl, const unsigned char *buf, int n) {
    if (!sl->src || n <= 0) return;
    Byte pkt[1024];
    MIDIPacketList *pl = (MIDIPacketList *)pkt;
    MIDIPacket *p = MIDIPacketListInit(pl);
    p = MIDIPacketListAdd(pl, sizeof pkt, p, 0, n, buf);
    if (p) MIDIReceived(sl->src, pl);
}

