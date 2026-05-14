// SPDX-License-Identifier: MIT OR Apache-2.0
/*
 * ep122_shim_link.c - sendto/sendmsg interceptor that aligns master-mode
 * Pro DJ Link broadcasts with our actually-audible playback.
 *
 * Strategy: delay-send ALL Pro DJ Link broadcasts (any packet on port
 * 50001 / 50002 with magic header "Qspt1WmJOL") by audio_latency_ms.
 * Every outbound packet arrives at the slave at our audible-event
 * moment. Position display, beat sync, phase lock, BPM, sync flags -
 * all timing references are consistent because they share the same
 * delay applied to the same packet stream.
 *
 * No payload modification - we just hold the packet in a queue until
 * its scheduled send time, then forward it via raw sendto. The packet's
 * ARRIVAL TIME is the implicit reference Pioneer slaves use, so the
 * packet has to actually land on the wire at our audible moment, not
 * at JUCE's pre-pipeline tick.
 *
 * Tradeoff: slave's display position cosmetically lags JUCE by
 * audio_latency_ms. But it MATCHES audible, which is what matters for
 * synchronization between decks.
 *
 * Toggle (shared with ep122_shim_clock - single switch for the whole
 * LD_PRELOAD audio-sync compensation pipeline):
 *   /sys/module/virtio_snd/parameters/audio_sync_enabled
 *     0 = full no-op. sendto/sendmsg pass through unmodified, slave
 *         clock shim is also inert. Audio plays raw, no Pro DJ Link
 *         sync compensation.
 *     1 = both shims active. Slave clock shift on OptFstUdpServer +
 *         delay-send on Pro DJ Link broadcasts.
 *
 * delay_ms source (refreshed once per wall-clock second on hot path):
 *   /sys/module/virtio_snd/parameters/link_pos_offset_ms (manual; if >0)
 *   /sys/module/virtio_snd/parameters/audio_latency_ms   (auto fallback)
 */

#include "ep122_shim.h"
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
#define PORT_BCAST_HI    50002       /* PLAYER_STATUS, etc. */
#define LINK_QUEUE_SIZE  512         /* ABS_POS at 3ms cadence × ~100ms delay = ~33 in flight; +headroom */
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
    /* Set by the close() hook when EP122 calls close(sockfd) while this
     * packet is still pending. The worker then performs the real close
     * after sendto, so EP122's socket-per-packet pattern survives the
     * delay. EP122's close() returned 0 to it immediately - the actual
     * kernel close just happens later from our side. */
    int                     ep122_wants_close;
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
    /* Master-mode toggle. Until enabled, the shim is fully transparent. */
    long enabled = shim_read_sysfs_long(VSND_LATENCY_ENABLED_PATH);
    if (enabled < 0) enabled = 0;
    __atomic_store_n(&g_enabled_atomic, enabled, __ATOMIC_RELAXED);
    if (g_link_debug && g_last_logged_enabled != enabled) {
        g_last_logged_enabled = enabled;
        fprintf(stderr, "[ep122_link] enabled = %ld\n", enabled);
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
        fprintf(stderr, "[ep122_link] delay = %ld ms (%s)\n",
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

        /* Hold a stable reference to the head-of-queue entry. We do NOT
         * advance g_q_tail yet - keeping the entry "active" lets the
         * close() hook find and mark it (ep122_wants_close) right up
         * until we finish processing. The producer's queue-full check
         * uses (head - tail), so leaving tail unmoved correctly counts
         * this slot as in-use; a producer will use a different slot. */
        pending_send_t *p = &g_q[g_q_tail % LINK_QUEUE_SIZE];
        struct timespec deadline = p->deadline;
        pthread_mutex_unlock(&g_q_mutex);

        /* Wait until the packet's scheduled send time. While sleeping,
         * close() hook may set p->ep122_wants_close - we read it after
         * sendto under the lock (same lock that close() acquires). */
        clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &deadline, NULL);

        /* Forward via raw syscall to avoid recursing into our own sendto.
         * Read packet body fields without lock - producers can't overwrite
         * this slot because tail hasn't advanced (slot still counts as
         * in-use); close() hook only writes ep122_wants_close, never
         * any of the fields we read here. */
        syscall(SYS_sendto, (long)p->sockfd, (long)p->buf,
                (long)p->len, (long)p->flags,
                (long)&p->dest, (long)p->addrlen);

        /* Atomic critical section: read close-flag, snapshot fd, advance
         * tail. Holding the lock across these three steps prevents a
         * close() hook from setting the flag *after* we read it but
         * *before* we advance tail (which would orphan the close). */
        pthread_mutex_lock(&g_q_mutex);
        int close_now = p->ep122_wants_close;
        int fd        = p->sockfd;
        g_q_tail++;
        pthread_mutex_unlock(&g_q_mutex);

        /* Real close, if EP122 already called close() while the packet
         * was queued. Done outside the queue lock. */
        if (close_now)
            syscall(SYS_close, (long)fd);

        /* Periodic delay-value refresh. */
        if (deadline.tv_sec - last_refresh_s >= VSND_LATENCY_REFRESH_S) {
            last_refresh_s = deadline.tv_sec;
            refresh_delay_ms();
        }
    }
    return NULL;   /* unreachable; silences -Wreturn-type */
}

static void __attribute__((constructor)) shim_link_init(void)
{
    g_link_debug = (getenv("EP122_LINK_DEBUG") != NULL);
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
            fprintf(stderr, "[ep122_link] pthread_create failed; "
                            "delay-send disabled for beats\n");
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
        /* Queue full - fall back to immediate send rather than drop. */
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
    p->ep122_wants_close = 0;
    memcpy(p->buf, buf, len);

    g_q_head++;
    pthread_cond_signal(&g_q_cond);
    pthread_mutex_unlock(&g_q_mutex);
    return 1;
}

/* ---------- core dispatch (used by both sendto and sendmsg) ---------- */

static int is_djpl_port(uint16_t port)
{
    return port == PORT_BCAST_LO || port == PORT_BCAST_HI;
}

/* Returns:
 *   0  caller should send NOW unchanged (packet not Pro DJ Link, or
 *      shim disabled, or queue full / worker unavailable).
 *   1  enqueued for delayed send - caller should report success.
 * Filter: any packet on Pro DJ Link ports (50001/50002) with the magic
 * header gets queued and held until master's audible event time. */
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

/* Refresh sysfs values lazily on the hot path: at most once per wall-clock
 * second, using __thread state. Cheap when nothing changed. */
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
            return (ssize_t)len;   /* enqueued - report success */
    }
    long r = syscall(SYS_sendto, (long)sockfd, (long)buf, (long)len,
                     (long)flags, (long)dest_addr, (long)addrlen);
    if (r < 0) { errno = (int)-r; return -1; }
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
    if (r < 0) { errno = (int)-r; return -1; }
    return (ssize_t)r;
}

/* Called from the existing close() hook in ep122_shim_syscalls.c.
 *
 * EP122's socket-per-packet pattern means it calls close() on the
 * sending FD microseconds after sendto returns. If we've queued the
 * packet for delayed send, the FD must stay open until the worker
 * actually fires the sendto.
 *
 * Returns 1 if the FD has a pending packet - caller should NOT call
 * sys_close; the kernel close will happen from our worker later.
 * Returns 0 if no match - caller should sys_close as normal.
 *
 * Race notes:
 *   - The queue entry remains in the active range until the worker has
 *     done sendto and atomically read ep122_wants_close + bumped tail
 *     under g_q_mutex. So a close() that races with worker either wins
 *     (flag set before worker reads it) or loses (tail already advanced
 *     past, scan misses, falls through to real close).
 *   - If our scan misses but worker happens to close shortly after,
 *     caller's sys_close fails with EBADF - harmless, we've already
 *     told EP122 close succeeded above. */
int ep122_link_intercept_close(int fd)
{
    pthread_mutex_lock(&g_q_mutex);
    for (unsigned i = g_q_tail; i != g_q_head; i++) {
        pending_send_t *p = &g_q[i % LINK_QUEUE_SIZE];
        if (p->sockfd == fd) {
            p->ep122_wants_close = 1;
            pthread_mutex_unlock(&g_q_mutex);
            return 1;
        }
    }
    pthread_mutex_unlock(&g_q_mutex);
    return 0;
}
