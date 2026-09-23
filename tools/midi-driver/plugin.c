// SPDX-License-Identifier: MIT OR Apache-2.0
/* plugin.c: the CFPlugIn factory MIDIServer loads, and the driver entry
 * points it calls. */
#include "driver.h"

/* Replaced with a subsystem handle in the factory; valid before that. */
os_log_t g_log = OS_LOG_DEFAULT;

MIDIDriverRef g_owner = 0;
volatile int  g_running = 0;
static pthread_t g_pump;
static int       g_pump_live = 0;

#define FACTORY_UUID CFUUIDGetConstantUUIDWithBytes(NULL, \
    0xAF,0xA6,0xCB,0x46,0x87,0x37,0x44,0xE1,0x88,0x5D,0x08,0x67,0x92,0x4B,0x22,0xF1)



typedef struct { MIDIDriverInterface *intf; CFUUIDRef factory; UInt32 refCount; } Driver;

static HRESULT QueryInterface(void *self, REFIID iid, LPVOID *ppv) {
    CFUUIDRef req = CFUUIDCreateFromUUIDBytes(NULL, iid);
    Driver *d = (Driver *)self;
    if (CFEqual(req, kMIDIDriverInterface2ID) || CFEqual(req, IUnknownUUID)) {
        CFRelease(req); d->refCount++; *ppv = self; return S_OK;
    }
    CFRelease(req); *ppv = NULL; return E_NOINTERFACE;
}
static ULONG AddRefD(void *self)  { return ++((Driver *)self)->refCount; }
static ULONG ReleaseD(void *self) {
    Driver *d = (Driver *)self;
    if (--d->refCount) return d->refCount;
    CFPlugInRemoveInstanceForFactory(d->factory); CFRelease(d->factory);
    free(d->intf); free(d); return 0;
}

static OSStatus Start(MIDIDriverRef self, MIDIDeviceListRef devList) {
    (void)devList;
    g_owner = self;
    /* No device up front: it is created only when the emu link is up (see
     * create_slot_device / the pump), so it appears together with the host HID and
     * djay pairs it.  Clear any device we left behind from a prior crash. */
    remove_stale_devices();
    if (!g_pump_live) {
        g_running = 1;
        if (pthread_create(&g_pump, NULL, pump, NULL) == 0) g_pump_live = 1;
        else g_running = 0;
    }
    return noErr;
}
/* Joins the pump so a following Start() cannot run a second one over the same
 * slot table.  Safe because the pump's shutdown path makes no CoreMIDI call:
 * MIDIServer holds its setup lock across this call. */
static OSStatus Stop(MIDIDriverRef self) {
    (void)self;
    g_running = 0;
    if (g_pump_live) {
        pthread_join(g_pump, NULL);
        g_pump_live = 0;
    }
    return noErr;
}
static OSStatus Configure(MIDIDriverRef self, MIDIDeviceRef d) { (void)self;(void)d; return noErr; }
/* A DAW sends to one destination endpoint; its refCon names the slot and the
 * generation it was published for. */
static OSStatus Send(MIDIDriverRef self, const MIDIPacketList *pl, void *r1, void *r2) {
    (void)self; (void)r2;
    if (!pl) return noErr;
    link_send(REF_IDX(r1), REF_GEN(r1), pl);
    return noErr;
}

static OSStatus EnableSource(MIDIDriverRef self, MIDIEndpointRef s, Boolean e) {
    (void)self; (void)s; (void)e;
    return noErr;
}
static OSStatus Flush(MIDIDriverRef self, MIDIEndpointRef d, void *r1, void *r2) {
    (void)self;(void)d;(void)r1;(void)r2; return noErr;
}
static OSStatus Monitor(MIDIDriverRef self, MIDIEndpointRef d, const MIDIPacketList *pl) {
    (void)self;(void)d;(void)pl; return noErr;
}

void *CDJ3KEmuMIDIFactory(CFAllocatorRef alloc, CFUUIDRef typeID);
void *CDJ3KEmuMIDIFactory(CFAllocatorRef alloc, CFUUIDRef typeID) {
    (void)alloc;
    g_log = os_log_create("com.cdj3k.emu", "midi-driver");
    if (!CFEqual(typeID, kMIDIDriverTypeID)) return NULL;
    Driver *d = (Driver *)calloc(1, sizeof *d);
    MIDIDriverInterface *i = (MIDIDriverInterface *)calloc(1, sizeof *i);
    i->_reserved = NULL;
    i->QueryInterface = QueryInterface;
    i->AddRef = AddRefD;
    i->Release = ReleaseD;
    i->FindDevices = NULL;
    i->Start = Start;  i->Stop = Stop;  i->Configure = Configure;
    i->Send = Send;    i->EnableSource = EnableSource;
    i->Flush = Flush;  i->Monitor = Monitor;
    d->intf = i; d->refCount = 1;
    d->factory = CFRetain(FACTORY_UUID);
    CFPlugInAddInstanceForFactory(d->factory);
    return d;
}
