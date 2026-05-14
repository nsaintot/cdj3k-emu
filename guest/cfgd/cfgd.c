// SPDX-License-Identifier: MIT OR Apache-2.0
/*
 * cfgd.c - guest-side config daemon for the cdj3k.cfg virtio-serial port.
 *
 * Single bidirectional channel between the guest and the cdj3k-emu UI process.
 * Replaces the old cdj3k.usbcmd + cdj3k.usbstate pair plus exposes
 * /sys/module/virtio_snd/parameters for live tuning + latency telemetry.
 *
 * Protocol (line-based, ASCII, '\n'-terminated):
 *
 *   host → guest
 *     usb attach              - invoke /usr/sbin/usb-external-attach.sh
 *     usb detach              - signal EP122 via /proc/udev_usb1, then
 *                               lazy-umount /media/usb/sd* and emit usb_state 0
 *     set <name> <value>      - write <value> to a whitelisted sysfs param
 *     get <name>              - emit a `param <name> <value>` response
 *
 *   guest → host
 *     usb_state <0|1>         - emitted by cfgd in response to SIGUSR1/SIGUSR2
 *                               from the USB hook scripts (see below)
 *     param <name> <value>    - response to get or unsolicited push
 *     latency <g>,<h>,<t>     - pushed every LATENCY_PERIOD_MS
 *
 * Signal IPC from in-guest scripts:
 *
 *   The cfg virtio-serial port is single-opener on Linux - once cfgd holds
 *   it `O_RDWR`, any other process trying to write gets EBUSY.  The USB
 *   hook scripts therefore can't write directly; instead they signal cfgd
 *   and cfgd does the emit:
 *
 *     SIGUSR1  ->  emit("usb_state 0")   (guest-initiated eject)
 *     SIGUSR2  ->  emit("usb_state 1")   (guest-initiated attach)
 *
 *   Scripts use `kill -USR1 $(pidof cdj3k-cfgd)` etc.  The host only acts
 *   on the eject side (it already knows when it initiated an attach), but
 *   both signals exist for symmetry.
 */

#define _GNU_SOURCE
#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define LATENCY_PERIOD_MS 3000

#define VPORT_NAME       "cdj3k.cfg"
#define VPORT_SYS_GLOB   "/sys/class/virtio-ports/"
#define USB_ATTACH       "/usr/sbin/usb-external-attach.sh"
#define SYSFS_DIR        "/sys/module/virtio_snd/parameters/"
#define LATENCY_FILE     SYSFS_DIR "audio_latency_ms"

/* Sysfs parameters cfgd is allowed to expose. Anything not listed here is
 * rejected with `param <name> ?` so a host bug can't poke arbitrary kernel
 * knobs. */
struct param_def {
    const char *name;
    int writable;
};
static const struct param_def PARAMS[] = {
    { "audio_sync_enabled",  1 },
    { "link_pos_offset_ms",  1 },
    { "audio_latency_ms",    0 },  /* read-only - also pushed every 3s */
};
#define N_PARAMS (sizeof(PARAMS) / sizeof(PARAMS[0]))

static volatile sig_atomic_t g_stop;
static volatile sig_atomic_t g_emit_state_0;
static volatile sig_atomic_t g_emit_state_1;
static int g_port_fd = -1;

static void on_signal(int sig) { (void)sig; g_stop = 1; }
static void on_usr1(int sig)   { (void)sig; g_emit_state_0 = 1; }
static void on_usr2(int sig)   { (void)sig; g_emit_state_1 = 1; }

static long now_ms(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (long)ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
}

static int open_port_blocking(void)
{
    char path[128];
    snprintf(path, sizeof(path), "/dev/virtio-ports/%s", VPORT_NAME);
    for (;;) {
        if (g_stop) return -1;
        int fd = open(path, O_RDWR | O_CLOEXEC);
        if (fd >= 0) return fd;
        struct timespec ts = { .tv_sec = 1, .tv_nsec = 0 };
        nanosleep(&ts, NULL);
    }
}

/* Atomic line write. virtio-serial respects PIPE_BUF for atomicity. */
static void emit(const char *fmt, ...)
{
    if (g_port_fd < 0) return;
    char buf[512];
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(buf, sizeof(buf), fmt, ap);
    va_end(ap);
    if (n <= 0) return;
    if (n >= (int)sizeof(buf)) n = (int)sizeof(buf) - 1;
    if (buf[n - 1] != '\n' && n < (int)sizeof(buf) - 1) {
        buf[n++] = '\n';
    }
    ssize_t w = write(g_port_fd, buf, (size_t)n);
    (void)w;
}

static const struct param_def *find_param(const char *name)
{
    for (size_t i = 0; i < N_PARAMS; i++) {
        if (strcmp(PARAMS[i].name, name) == 0) return &PARAMS[i];
    }
    return NULL;
}

/* Read a sysfs file fully (small files only). On success returns a malloc'd
 * NUL-terminated string with trailing newline stripped. */
static char *read_sysfs(const char *name)
{
    char path[256];
    snprintf(path, sizeof(path), "%s%s", SYSFS_DIR, name);
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) return NULL;
    char buf[256] = {0};
    ssize_t n = read(fd, buf, sizeof(buf) - 1);
    close(fd);
    if (n <= 0) return NULL;
    while (n > 0 && (buf[n - 1] == '\n' || buf[n - 1] == ' ')) {
        buf[--n] = '\0';
    }
    return strdup(buf);
}

static int write_sysfs(const char *name, const char *value)
{
    char path[256];
    snprintf(path, sizeof(path), "%s%s", SYSFS_DIR, name);
    int fd = open(path, O_WRONLY | O_CLOEXEC);
    if (fd < 0) return -1;
    size_t len = strlen(value);
    ssize_t w = write(fd, value, len);
    close(fd);
    return (w == (ssize_t)len) ? 0 : -1;
}

/* Tell EP122 the USB is gone via /proc/udev_usb1 (it reads this channel as
 * IPC and closes its FDs in response), then lazy-unmount any /media/usb/sd*
 * mount points.  Mirrors what the hardware eject path produces; the
 * `umount /dev/sdb` line is what EP122 actually parses (the unbind script
 * uses the same channel with a mount-path argument, but EP122 accepts the
 * device-path form too and that's what the host knows). */
static void detach_usb(void)
{
    int fd = open("/proc/udev_usb1", O_WRONLY | O_CLOEXEC);
    if (fd >= 0) {
        static const char msg[] = "umount /dev/sdb";
        ssize_t w = write(fd, msg, sizeof(msg) - 1);
        (void)w;
        close(fd);
    }

    /* Let EP122 close its files before we umount. */
    struct timespec ts = { .tv_sec = 0, .tv_nsec = 150L * 1000 * 1000 };
    nanosleep(&ts, NULL);

    FILE *fp = fopen("/proc/mounts", "r");
    if (!fp) return;
    char line[512];
    while (fgets(line, sizeof(line), fp)) {
        char dev[128], mnt[256];
        if (sscanf(line, "%127s %255s", dev, mnt) != 2) continue;
        if (strncmp(mnt, "/media/usb/sd", 13) != 0) continue;
        pid_t pid = fork();
        if (pid == 0) {
            execl("/bin/umount", "umount", "-l", mnt, (char *)NULL);
            _exit(127);
        }
        if (pid > 0) {
            int status;
            waitpid(pid, &status, 0);
        }
    }
    fclose(fp);
}

static void handle_usb(const char *arg)
{
    if (strcmp(arg, "attach") == 0) {
        pid_t pid = fork();
        if (pid == 0) {
            execl(USB_ATTACH, USB_ATTACH, (char *)NULL);
            _exit(127);
        }
        if (pid > 0) {
            int status;
            waitpid(pid, &status, 0);
        }
    } else if (strcmp(arg, "detach") == 0) {
        /* No `usb_state 0` emit here: this path runs in response to a
         * host-initiated eject, so the host already knows.  The
         * guest-initiated path (EP122 button → unbind-usb-device.sh)
         * is the only one that needs to inform the host, and it writes
         * `usb_state 0` to cdj3k.cfg itself. */
        detach_usb();
    }
}

static void handle_set(const char *args)
{
    const char *space = strchr(args, ' ');
    if (!space) return;
    char name[64];
    size_t name_len = (size_t)(space - args);
    if (name_len == 0 || name_len >= sizeof(name)) return;
    memcpy(name, args, name_len);
    name[name_len] = '\0';
    const char *value = space + 1;

    const struct param_def *p = find_param(name);
    if (!p || !p->writable) {
        emit("param %s ?", name);
        return;
    }
    if (write_sysfs(name, value) == 0) {
        char *back = read_sysfs(name);
        if (back) {
            emit("param %s %s", name, back);
            free(back);
        }
    } else {
        emit("param %s !", name);
    }
}

static void handle_get(const char *name)
{
    const struct param_def *p = find_param(name);
    if (!p) {
        emit("param %s ?", name);
        return;
    }
    char *v = read_sysfs(name);
    if (v) {
        emit("param %s %s", name, v);
        free(v);
    } else {
        emit("param %s !", name);
    }
}

static void dispatch_line(char *line)
{
    /* Strip trailing whitespace. */
    size_t n = strlen(line);
    while (n > 0 && (line[n - 1] == '\n' || line[n - 1] == '\r' ||
                     line[n - 1] == ' ' || line[n - 1] == '\t')) {
        line[--n] = '\0';
    }
    if (n == 0) return;

    /* Echo passthrough lines (anything emitted by the USB hook scripts):
     * we receive our own "usb_state 1" frames because the port is RDWR
     * and the kernel loops back nothing - but defensive guard anyway. */
    if (strncmp(line, "usb_state ", 10) == 0) return;
    if (strncmp(line, "param ", 6) == 0) return;
    if (strncmp(line, "latency ", 8) == 0) return;

    if (strncmp(line, "usb ", 4) == 0) {
        handle_usb(line + 4);
    } else if (strncmp(line, "set ", 4) == 0) {
        handle_set(line + 4);
    } else if (strncmp(line, "get ", 4) == 0) {
        handle_get(line + 4);
    } else if (strcmp(line, "ping") == 0) {
        emit("pong");
    } else {
        fprintf(stderr, "cfgd: unknown command '%s'\n", line);
    }
}

static void push_latency(void)
{
    char *v = read_sysfs("audio_latency_ms");
    if (!v) return;
    /* virtio_snd already emits "guest,host,total". */
    emit("latency %s", v);
    free(v);
}

int main(void)
{
    signal(SIGPIPE, SIG_IGN);
    signal(SIGINT, on_signal);
    signal(SIGTERM, on_signal);
    signal(SIGUSR1, on_usr1);
    signal(SIGUSR2, on_usr2);

    g_port_fd = open_port_blocking();
    if (g_port_fd < 0) return 1;
    fprintf(stderr, "cfgd: connected to /dev/virtio-ports/%s (fd %d)\n",
            VPORT_NAME, g_port_fd);

    /* Initial state push so the host doesn't have to query immediately. */
    push_latency();

    char rx[1024];
    size_t rx_n = 0;
    long next_push = now_ms() + LATENCY_PERIOD_MS;

    while (!g_stop) {
        /* Drain signal-driven emit flags before blocking on poll().  Signals
         * (SIGUSR1/SIGUSR2) interrupt poll() with EINTR, the existing handler
         * does `continue`, which loops back here. */
        if (g_emit_state_0) { g_emit_state_0 = 0; emit("usb_state 0"); }
        if (g_emit_state_1) { g_emit_state_1 = 0; emit("usb_state 1"); }

        long now = now_ms();
        long timeout = next_push - now;
        if (timeout < 0) timeout = 0;

        struct pollfd pfd = { .fd = g_port_fd, .events = POLLIN };
        int pr = poll(&pfd, 1, (int)timeout);
        if (pr < 0) {
            if (errno == EINTR) continue;
            break;
        }
        if (pr > 0 && (pfd.revents & POLLIN)) {
            ssize_t r = read(g_port_fd, rx + rx_n, sizeof(rx) - rx_n - 1);
            if (r <= 0) {
                /* EOF / disconnect - reopen. */
                close(g_port_fd);
                g_port_fd = open_port_blocking();
                rx_n = 0;
                continue;
            }
            rx_n += (size_t)r;
            rx[rx_n] = '\0';

            /* Split on '\n'. */
            char *start = rx;
            char *nl;
            while ((nl = strchr(start, '\n')) != NULL) {
                *nl = '\0';
                dispatch_line(start);
                start = nl + 1;
            }
            /* Move remainder. */
            size_t rem = rx_n - (size_t)(start - rx);
            memmove(rx, start, rem);
            rx_n = rem;
            /* Hard reset on overflow (malformed input).  Log so the host
             * sees we discarded data instead of silently corrupting the
             * stream - a misbehaving sender is rare but worth diagnosing. */
            if (rx_n >= sizeof(rx) - 1) {
                fprintf(stderr,
                        "cfgd: rx buffer overflow (%zu bytes), discarding\n",
                        rx_n);
                rx_n = 0;
            }
        }
        if (now_ms() >= next_push) {
            push_latency();
            next_push = now_ms() + LATENCY_PERIOD_MS;
        }
    }

    if (g_port_fd >= 0) close(g_port_fd);
    return 0;
}
