// SPDX-License-Identifier: MIT OR Apache-2.0
/*
 * link.c - a sendto and sendmsg hook that aligns the master's Pro DJ Link
 * broadcasts with its audible playback.
 *
 * The shim delays every Pro DJ Link broadcast (any packet to port 50001 or
 * 50002 with the magic header "Qspt1WmJOL") by audio_latency_ms, so each
 * packet reaches the slave when its audio becomes audible here. The position
 * display, beat sync, phase lock, BPM and sync flags stay consistent with
 * each other because the same delay applies to the whole packet stream.
 *
 * The payload is not modified: the packet waits in a queue until its send
 * time, then goes out through a raw sendto. Pioneer slaves use the packet's
 * arrival time as the timing reference, so the packet must reach the wire
 * when its audio becomes audible.
 *
 * The slave's displayed position lags JUCE by audio_latency_ms, but it
 * matches the audible output, and decks synchronise on that.
 *
 * The switch, shared with clock.c, turns all audio-sync compensation on or
 * off:
 *   /sys/module/virtio_snd/parameters/audio_sync_enabled
 *     0: both hooks are off. sendto and sendmsg pass through unchanged,
 *        clock.c does not shift the clock, and there is no Pro DJ Link sync
 *        compensation.
 *     1: both hooks are on. clock.c shifts the OptFstUdpServer clock, and
 *        this file delays the Pro DJ Link broadcasts.
 *
 * The delay comes from these parameters, read at most once per wall-clock
 * second on the hot path:
 *   /sys/module/virtio_snd/parameters/link_pos_offset_ms (manual, if positive)
 *   /sys/module/virtio_snd/parameters/audio_latency_ms   (otherwise)
 */

#include "deck_shim.h"
#include <pthread.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>

#ifndef SYS_sendto
#define SYS_sendto         206  /* aarch64 */
#endif
#ifndef SYS_sendmsg
#define SYS_sendmsg        211
#endif

#define PORT_BCAST_LO    50001       /* ABS_POS, BEAT, MIXER_VOLUMES */
#define PORT_BCAST_HI    50002       /* PLAYER_STATUS and others */
#define LINK_QUEUE_SIZE  512         /* ABS_POS goes out every 3 ms, so a 100 ms delay keeps about 33 packets in flight; the rest is headroom. */
#define LINK_PKT_MAX     1500

static const unsigned char g_djpl_magic[10] = {
    'Q','s','p','t','1','W','m','J','O','L'
};

typedef struct {
    struct timespec         deadline;
    struct sockaddr_storage dest;
    socklen_t               addrlen;
    int                     sockfd;
    int                     flags;
    size_t                  len;
    /* The close() hook sets this when the app calls close(sockfd) while this
     * packet is still pending. The worker then closes the fd after sendto,
     * so the app's socket-per-packet pattern survives the delay. The app's
     * close() has already returned 0. */
    int                     app_wants_close;
    unsigned char           buf[LINK_PKT_MAX];
} pending_send_t;

static pending_send_t   g_q[LINK_QUEUE_SIZE];
static unsigned int     g_q_head;
static unsigned int     g_q_tail;
static pthread_mutex_t  g_q_mutex = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t   g_q_cond  = PTHREAD_COND_INITIALIZER;
static pthread_t        g_worker;
static int              g_worker_started;

static long             g_delay_ms_atomic;
static long             g_enabled_atomic;
static long             g_last_logged_delay = -1;
static long             g_last_logged_enabled = -1;
static int              g_link_debug;

static void refresh_delay_ms(void)
{
    /* The master-mode switch. While it is off, packets pass through. */
    long enabled = shim_read_sysfs_long(VSND_LATENCY_ENABLED_PATH);
    if (enabled < 0) enabled = 0;
    __atomic_store_n(&g_enabled_atomic, enabled, __ATOMIC_RELAXED);
    if (g_link_debug && g_last_logged_enabled != enabled) {
        g_last_logged_enabled = enabled;
        fprintf(stderr, "[deck_link] audio_sync_enabled is %ld.\n", enabled);
        fflush(stderr);
    }

    long manual = shim_read_sysfs_long(VSND_LATENCY_MANUAL_PATH);
    long ms;
    if (manual > 0) {
        ms = manual;
    } else {
        ms = shim_read_sysfs_long(VSND_LATENCY_AUTO_PATH);
        if (ms < 0) ms = 0;
    }
    if (ms > 5000) ms = 5000;
    long prev = __atomic_load_n(&g_delay_ms_atomic, __ATOMIC_RELAXED);
    if (prev == ms) return;
    __atomic_store_n(&g_delay_ms_atomic, ms, __ATOMIC_RELAXED);
    if (g_link_debug && g_last_logged_delay != ms) {
        g_last_logged_delay = ms;
        fprintf(stderr, "[deck_link] The delay is now %ld ms (%s).\n",
                ms, manual > 0 ? "manual" : "auto");
        fflush(stderr);
    }
}

/* ---------- worker thread ---------- */

static void *link_worker_fn(void *arg)
{
    (void)arg;
    long last_refresh_s = 0;

    for (;;) {
        pthread_mutex_lock(&g_q_mutex);
        while (g_q_head == g_q_tail)
            pthread_cond_wait(&g_q_cond, &g_q_mutex);

        /* Take the oldest entry. g_q_tail does not advance yet, so the
         * entry stays active and the close() hook can still mark it
         * (app_wants_close) until processing ends. The producer's
         * queue-full check uses (head - tail), so the slot still counts as
         * in use and a producer picks a different one. */
        pending_send_t *p = &g_q[g_q_tail % LINK_QUEUE_SIZE];
        struct timespec deadline = p->deadline;
        pthread_mutex_unlock(&g_q_mutex);

        /* Wait until the packet's send time. While the worker sleeps, the
         * close() hook may set p->app_wants_close; the worker reads it after
         * sendto, under the same lock that close() takes. */
        clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &deadline, NULL);

        /* Send through a raw syscall, so the call does not recurse into the
         * sendto hook. The packet fields are read without the lock:
         * producers cannot overwrite this slot because the tail has not
         * advanced, and the close() hook writes only app_wants_close. */
        syscall(SYS_sendto, (long)p->sockfd, (long)p->buf,
                (long)p->len, (long)p->flags,
                (long)&p->dest, (long)p->addrlen);

        /* Read the close flag, copy the fd and advance the tail under one
         * lock. Otherwise a close() hook could set the flag after it is read
         * but before the tail advances, and the close would be lost. */
        pthread_mutex_lock(&g_q_mutex);
        int close_now = p->app_wants_close;
        int fd        = p->sockfd;
        g_q_tail++;
        pthread_mutex_unlock(&g_q_mutex);

        /* Close the fd outside the queue lock if the app called close()
         * while the packet was queued. */
        if (close_now)
            syscall(SYS_close, (long)fd);

        if (deadline.tv_sec - last_refresh_s >= VSND_LATENCY_REFRESH_S) {
            last_refresh_s = deadline.tv_sec;
            refresh_delay_ms();
        }
    }
    return NULL;   /* Unreachable; this silences -Wreturn-type. */
}

static void __attribute__((constructor)) shim_link_init(void)
{
    g_link_debug = (getenv("DECK_LINK_DEBUG") != NULL);
    refresh_delay_ms();
}

static void ensure_worker_started(void)
{
    if (__atomic_load_n(&g_worker_started, __ATOMIC_ACQUIRE)) return;
    int expected = 0;
    if (!__atomic_compare_exchange_n(&g_worker_started, &expected, 1,
                                     0, __ATOMIC_ACQ_REL, __ATOMIC_RELAXED))
        return;
    if (pthread_create(&g_worker, NULL, link_worker_fn, NULL) != 0) {
        __atomic_store_n(&g_worker_started, 0, __ATOMIC_RELEASE);
        if (g_link_debug) {
            fprintf(stderr, "[deck_link] pthread_create failed; "
                            "Pro DJ Link packets are sent without delay.\n");
            fflush(stderr);
        }
    }
}

/* ---------- packet classification ---------- */

static uint16_t port_from_sa(const struct sockaddr *sa, socklen_t alen)
{
    if (!sa || sa->sa_family != AF_INET) return 0;
    if (alen < (socklen_t)sizeof(struct sockaddr_in)) return 0;
    return ntohs(((const struct sockaddr_in *)sa)->sin_port);
}

static int is_djpl_packet(const void *buf, size_t len)
{
    if (len < 11) return 0;
    return memcmp(buf, g_djpl_magic, sizeof(g_djpl_magic)) == 0;
}

/* ---------- enqueue ---------- */

static int try_enqueue(int sockfd, const void *buf, size_t len, int flags,
                       const struct sockaddr *dest_addr, socklen_t addrlen,
                       long delay_ms)
{
    if (delay_ms <= 0) return 0;
    if (len > LINK_PKT_MAX) return 0;
    ensure_worker_started();
    if (!__atomic_load_n(&g_worker_started, __ATOMIC_ACQUIRE)) return 0;

    pthread_mutex_lock(&g_q_mutex);
    if (g_q_head - g_q_tail >= LINK_QUEUE_SIZE) {
        /* The queue is full: send the packet immediately. */
        pthread_mutex_unlock(&g_q_mutex);
        return 0;
    }
    pending_send_t *p = &g_q[g_q_head % LINK_QUEUE_SIZE];

    struct timespec now;
    syscall(SYS_clock_gettime, (long)CLOCK_MONOTONIC, (long)&now);
    p->deadline = now;
    p->deadline.tv_nsec += delay_ms * 1000000L;
    if (p->deadline.tv_nsec >= 1000000000L) {
        p->deadline.tv_sec  += p->deadline.tv_nsec / 1000000000L;
        p->deadline.tv_nsec %= 1000000000L;
    }

    memcpy(&p->dest, dest_addr, addrlen);
    p->addrlen = addrlen;
    p->sockfd  = sockfd;
    p->flags   = flags;
    p->len     = len;
    p->app_wants_close = 0;
    memcpy(p->buf, buf, len);

    g_q_head++;
    pthread_cond_signal(&g_q_cond);
    pthread_mutex_unlock(&g_q_mutex);
    return 1;
}

/* ---------- dispatch, shared by sendto and sendmsg ---------- */

static int is_djpl_port(uint16_t port)
{
    return port == PORT_BCAST_LO || port == PORT_BCAST_HI;
}

/* Returns:
 *   0  The caller sends the packet immediately and unchanged. This happens
 *      when the packet is not Pro DJ Link, the shim is disabled, the queue
 *      is full or the worker is unavailable.
 *   1  The packet is queued for a delayed send, and the caller reports
 *      success.
 * A packet to a Pro DJ Link port (50001 or 50002) with the magic header is
 * held until its audio becomes audible on this deck. */
static int try_handle_packet(int sockfd, const void *buf, size_t len, int flags,
                             const struct sockaddr *dest_addr, socklen_t addrlen,
                             uint16_t port)
{
    if (!is_djpl_port(port)) return 0;
    if (!is_djpl_packet(buf, len)) return 0;
    if (!__atomic_load_n(&g_enabled_atomic, __ATOMIC_RELAXED)) return 0;

    long delay_ms = __atomic_load_n(&g_delay_ms_atomic, __ATOMIC_RELAXED);
    return try_enqueue(sockfd, buf, len, flags, dest_addr, addrlen, delay_ms);
}

/* ---------- intercepted libc symbols ---------- */

/* Refresh the sysfs values on the hot path at most once per wall-clock
 * second per thread. */
static void link_lazy_refresh(void)
{
    static __thread long last_refresh_s;
    struct timespec now;
    syscall(SYS_clock_gettime, (long)CLOCK_MONOTONIC, (long)&now);
    if ((long)now.tv_sec - last_refresh_s >= VSND_LATENCY_REFRESH_S) {
        last_refresh_s = (long)now.tv_sec;
        refresh_delay_ms();
    }
}

ssize_t sendto(int sockfd, const void *buf, size_t len, int flags,
               const struct sockaddr *dest_addr, socklen_t addrlen)
{
    uint16_t port = port_from_sa(dest_addr, addrlen);
    if (is_djpl_port(port) && len <= LINK_PKT_MAX) {
        link_lazy_refresh();
        if (try_handle_packet(sockfd, buf, len, flags,
                              dest_addr, addrlen, port))
            return (ssize_t)len;   /* Queued; report success. */
    }
    long r = syscall(SYS_sendto, (long)sockfd, (long)buf, (long)len,
                     (long)flags, (long)dest_addr, (long)addrlen);
    if (r < 0) return -1;   /* glibc syscall() already set errno */
    return (ssize_t)r;
}

ssize_t sendmsg(int sockfd, const struct msghdr *msg, int flags)
{
    if (msg && msg->msg_name && msg->msg_iov && msg->msg_iovlen == 1) {
        uint16_t port = port_from_sa(
            (const struct sockaddr *)msg->msg_name, msg->msg_namelen);
        size_t len = msg->msg_iov[0].iov_len;
        const void *p = msg->msg_iov[0].iov_base;
        if (is_djpl_port(port) && len <= LINK_PKT_MAX) {
            link_lazy_refresh();
            if (try_handle_packet(
                    sockfd, p, len, flags,
                    (const struct sockaddr *)msg->msg_name,
                    msg->msg_namelen, port))
                return (ssize_t)len;
        }
    }
    long r = syscall(SYS_sendmsg, (long)sockfd, (long)msg, (long)flags);
    if (r < 0) return -1;   /* glibc syscall() already set errno */
    return (ssize_t)r;
}

/* The close() hook in syscalls.c calls this.
 *
 * The app opens one socket per packet and closes it microseconds after
 * sendto returns. A queued packet needs its fd open until the worker sends
 * it.
 *
 * Returns 1 if the fd has a pending packet; the caller must not call
 * sys_close, because the worker closes the fd later. Returns 0 otherwise,
 * and the caller closes the fd as usual.
 *
 * Races:
 *   - The queue entry stays in the active range until the worker has sent
 *     it, then read app_wants_close and advanced the tail under g_q_mutex.
 *     A close() that races with the worker either wins, when it sets the
 *     flag before the worker reads it, or loses, when the tail has already
 *     moved past the entry and the caller closes the fd itself.
 *   - If the scan misses, the worker has already advanced past the entry
 *     and does not close the fd, so the caller's sys_close is the only
 *     close. */
int shim_link_intercept_close(int fd)
{
    pthread_mutex_lock(&g_q_mutex);
    for (unsigned i = g_q_tail; i != g_q_head; i++) {
        pending_send_t *p = &g_q[i % LINK_QUEUE_SIZE];
        if (p->sockfd == fd) {
            p->app_wants_close = 1;
            pthread_mutex_unlock(&g_q_mutex);
            return 1;
        }
    }
    pthread_mutex_unlock(&g_q_mutex);
    return 0;
}
