// SPDX-License-Identifier: MIT OR Apache-2.0
/* plugin.c: the CFPlugIn factory MIDIServer loads, and the driver entry
 * points it calls. */
#include "driver.h"

/* Replaced with a subsystem handle in the factory; valid before that. */
os_log_t g_log = OS_LOG_DEFAULT;

MIDIDriverRef g_owner = 0;
/* One pump thread for the life of the process, created on the first Start().
 * Stop() never waits for it: MIDIServer holds its setup lock across Stop(),
 * and the pump can be inside a CoreMIDI call that needs that lock. */
static pthread_mutex_t g_run_lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t  g_run_cv   = PTHREAD_COND_INITIALIZER;
static int      g_running = 0;
static uint32_t g_epoch = 0;     /* bumped by every Stop() */
static int      g_pump_live = 0;

int driver_live(uint32_t epoch) {
    return __atomic_load_n(&g_running, __ATOMIC_ACQUIRE) &&
           __atomic_load_n(&g_epoch, __ATOMIC_ACQUIRE) == epoch;
}

uint32_t driver_wait_started(void) {
    pthread_mutex_lock(&g_run_lock);
    while (!g_running) pthread_cond_wait(&g_run_cv, &g_run_lock);
    uint32_t epoch = g_epoch;
    pthread_mutex_unlock(&g_run_lock);
    return epoch;
}

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
    pthread_mutex_lock(&g_run_lock);
    if (!g_pump_live) {
        pthread_t t;
        if (pthread_create(&t, NULL, pump, NULL) == 0) {
            pthread_detach(t);
            g_pump_live = 1;
        } else {
            os_log_error(g_log, "pump thread create failed");
        }
    }
    __atomic_store_n(&g_running, 1, __ATOMIC_RELEASE);
    pthread_cond_broadcast(&g_run_cv);
    pthread_mutex_unlock(&g_run_lock);
    return noErr;
}
/* Ends the current run without waiting for the pump; it drops its slots when
 * it next wakes, within POLL_TIMEOUT. */
static OSStatus Stop(MIDIDriverRef self) {
    (void)self;
    pthread_mutex_lock(&g_run_lock);
    __atomic_store_n(&g_running, 0, __ATOMIC_RELEASE);
    __atomic_add_fetch(&g_epoch, 1, __ATOMIC_ACQ_REL);
    pthread_mutex_unlock(&g_run_lock);
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
