// SPDX-License-Identifier: MIT OR Apache-2.0
/* link.c: instance sockets, the identity handshake, and the poll pump.  The slot
 * table lives here; everything outside reaches it through link_send(). */
#include "driver.h"

static struct slot g_slots[MAX_SLOTS] = {
    { .fd = -1 }, { .fd = -1 }, { .fd = -1 }, { .fd = -1 },
    { .fd = -1 }, { .fd = -1 }, { .fd = -1 }, { .fd = -1 },
};
static uint32_t g_next_gen = 1;
static pthread_mutex_t g_slots_lock = PTHREAD_MUTEX_INITIALIZER;

static int slot_connected(const char *path) {
    for (int i = 0; i < MAX_SLOTS; i++)
        if (g_slots[i].fd >= 0 && strcmp(g_slots[i].path, path) == 0) return 1;
    return 0;
}

/* Dial one instance socket and read its id.  Returns the fd, or -1. */
static int dial(const char *path, struct pc_link_identity *id) {
    int fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (fd < 0) return -1;
    struct sockaddr_un a; memset(&a, 0, sizeof a);
    a.sun_family = AF_UNIX;
    strncpy(a.sun_path, path, sizeof a.sun_path - 1);
    if (connect(fd, (struct sockaddr *)&a, sizeof a) < 0) { close(fd); return -1; }

    /* Scoped to this fd: a write to a closed link must return EPIPE, not
     * signal the host process.  We run inside MIDIServer. */
    int on = 1;
    setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &on, sizeof on);
    /* Both halves are bounded: a wedged instance must not stall the pump or
     * a DAW's Send(). */
    struct timeval tv = { .tv_sec = 0, .tv_usec = 250000 };
    setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof tv);
    struct timeval rtv = { .tv_sec = 2, .tv_usec = 0 };
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &rtv, sizeof rtv);

    size_t got = 0;
    while (got < sizeof *id) {
        ssize_t r = read(fd, (char *)id + got, sizeof *id - got);
        if (r > 0) { got += (size_t)r; continue; }
        if (r < 0 && errno == EINTR) continue;
        break;
    }
    struct timeval zero = { 0, 0 };
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &zero, sizeof zero);
    if (got != sizeof *id || id->magic != IDENTITY_MAGIC ||
        id->version != IDENTITY_VERSION) {
        os_log_error(g_log, "bad identity from %{public}s (%zu bytes)", path, got);
        close(fd);
        return -1;
    }
    return fd;
}

/* Every instance advertises itself by binding a socket in its own runtime
 * directory, so the set of live slots is what is in the filesystem.
 * `dial()` blocks on connect and the identity read and must stay outside the
 * lock, which Send() also takes. */
static void scan_and_connect(void) {
    char base[128];
    snprintf(base, sizeof base, "/tmp/cdj3k-emu-%u", (unsigned)getuid());
    DIR *d = opendir(base);
    if (!d) return;
    struct dirent *e;
    while ((e = readdir(d))) {
        if (strncmp(e->d_name, "instance-", 9) != 0) continue;
        char path[128];
        snprintf(path, sizeof path, "%s/%s/midi-driver.sock", base, e->d_name);

        /* Claim a free slot before dialling: a full table then costs a
         * readdir and nothing else. */
        pthread_mutex_lock(&g_slots_lock);
        int busy = slot_connected(path), idx = -1;
        if (!busy)
            for (int i = 0; i < MAX_SLOTS; i++) if (g_slots[i].fd < 0) { idx = i; break; }
        pthread_mutex_unlock(&g_slots_lock);
        if (busy) continue;
        if (idx < 0) continue;   /* table full; nothing useful to do */

        struct pc_link_identity id;
        int fd = dial(path, &id);
        if (fd < 0) continue;

        pthread_mutex_lock(&g_slots_lock);
        struct slot *sl = &g_slots[idx];
        if (sl->fd >= 0 || slot_connected(path)) {  /* raced; drop ours */
            pthread_mutex_unlock(&g_slots_lock);
            close(fd);
            continue;
        }
        sl->fd = fd;
        sl->gen = g_next_gen++;
        sl->location = id.location;
        sl->usb_vp = ((uint32_t)id.vid << 16) | id.pid;
        snprintf(sl->path, sizeof sl->path, "%s", path);
        id.product[sizeof id.product - 1] = 0;
        id.serial[sizeof id.serial - 1] = 0;
        id.manufacturer[sizeof id.manufacturer - 1] = 0;
        sl->name = CFStringCreateWithCString(NULL, id.product,
                                             kCFStringEncodingUTF8);
        sl->maker = CFStringCreateWithCString(NULL, id.manufacturer,
                                              kCFStringEncodingUTF8);
        if (!sl->name || !sl->maker) {
            (void)slot_detach(sl);
            pthread_mutex_unlock(&g_slots_lock);
            continue;
        }
        os_log(g_log,
               "slot %d connected: instance %u, %{public}s %{public}s, location %u",
               idx, (unsigned)id.instance, id.product, id.serial,
               (unsigned)id.location);
        pthread_mutex_unlock(&g_slots_lock);

        /* Published with the lock released; MIDIServer calls into this driver
         * while holding the setup lock this needs. */
        if (create_slot_device(sl, idx) < 0) {
            pthread_mutex_lock(&g_slots_lock);
            MIDIDeviceRef d = slot_detach(sl);
            uint32_t loc = sl->location;
            pthread_mutex_unlock(&g_slots_lock);
            slot_retire(d, loc);
        }
    }
    closedir(d);
}

void *pump(void *arg) {
    (void)arg;
    /* Monotonic, so an NTP step or a wake from sleep cannot stall the
     * rescan. */
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    time_t last_scan = ts.tv_sec - SCAN_SECS;
    while (g_running) {
        clock_gettime(CLOCK_MONOTONIC, &ts);
        time_t now = ts.tv_sec;
        if (now - last_scan >= SCAN_SECS) {
            last_scan = now;
            scan_and_connect();
        }

        struct pollfd pfd[MAX_SLOTS];
        int map[MAX_SLOTS], n = 0;
        pthread_mutex_lock(&g_slots_lock);
        for (int i = 0; i < MAX_SLOTS; i++) {
            if (g_slots[i].fd < 0) continue;
            pfd[n].fd = g_slots[i].fd; pfd[n].events = POLLIN; pfd[n].revents = 0;
            map[n++] = i;
        }
        pthread_mutex_unlock(&g_slots_lock);

        if (n == 0) { sleep(SCAN_SECS); continue; }
        int pr = poll(pfd, (nfds_t)n, POLL_TIMEOUT);
        if (pr == 0) continue;                    /* idle tick */
        if (pr < 0) {
            if (errno != EINTR) sleep(1);         /* never spin on a hard error */
            continue;
        }

        for (int k = 0; k < n; k++) {
            if (!pfd[k].revents) continue;
            unsigned char buf[512];
            /* Send() can close this fd, so the descriptor is only touched
             * under the lock.  poll() reported it ready, so the read returns
             * without blocking. */
            MIDIDeviceRef retire = 0;
            uint32_t retire_loc = 0;
            pthread_mutex_lock(&g_slots_lock);
            struct slot *sl = &g_slots[map[k]];
            if (sl->fd != pfd[k].fd) { pthread_mutex_unlock(&g_slots_lock); continue; }
            ssize_t r = read(sl->fd, buf, sizeof buf);
            if (r > 0)                        emit_from_slot(sl, buf, (int)r);
            else if (r < 0 && errno == EINTR) { /* retry next tick */ }
            else { retire_loc = sl->location; retire = slot_detach(sl); }
            pthread_mutex_unlock(&g_slots_lock);
            slot_retire(retire, retire_loc);
        }
    }

    /* Shutdown runs with MIDIServer inside Stop(), holding the setup lock, so
     * nothing here may call CoreMIDI.  Sockets and strings are released; the
     * published devices are left for the MARKER sweep in remove_stale_devices
     * on the next Start. */
    pthread_mutex_lock(&g_slots_lock);
    for (int i = 0; i < MAX_SLOTS; i++)
        if (g_slots[i].fd >= 0) (void)slot_detach(&g_slots[i]);
    pthread_mutex_unlock(&g_slots_lock);
    return NULL;
}


/* Write one packet list down the socket of `idx`, if that slot is still the
 * one the endpoint was published for.  A stale generation means the index has
 * been reused by another instance. */
void link_send(int idx, uint32_t gen, const MIDIPacketList *pl) {
    if (idx < 0 || idx >= MAX_SLOTS) return;

    pthread_mutex_lock(&g_slots_lock);
    struct slot *sl = &g_slots[idx];
    if (sl->fd < 0 || sl->gen != gen) {
        pthread_mutex_unlock(&g_slots_lock);
        return;
    }

    int failed = 0;
    const MIDIPacket *p = &pl->packet[0];
    for (UInt32 i = 0; i < pl->numPackets && !failed; i++) {
        ssize_t off = 0;
        while (off < p->length) {
            ssize_t w = write(sl->fd, p->data + off, p->length - off);
            if (w > 0) { off += w; continue; }
            if (w < 0 && errno == EINTR) continue;
            /* A partial write leaves half a message on the wire; the link goes
             * rather than the next packet's status byte following it. */
            failed = 1;
            break;
        }
        p = MIDIPacketNext(p);
    }
    MIDIDeviceRef retire = 0;
    uint32_t retire_loc = 0;
    if (failed) {
        os_log_error(g_log, "slot %d write failed (%{public}s); dropping link",
                     idx, strerror(errno));
        retire_loc = sl->location;
        retire = slot_detach(sl);
    }
    pthread_mutex_unlock(&g_slots_lock);
    slot_retire(retire, retire_loc);
}
