// SPDX-License-Identifier: MIT OR Apache-2.0
/*
 * devd.c - the guest side of the control API's guest commands, on the
 * cdj3k.dev virtio-serial port.
 *
 * The host adds this daemon and its service to a boot only in Developer
 * Mode. It runs shell commands for the host and streams their output back.
 *
 * The protocol is line-based ASCII; each line ends in '\n'.
 *
 *   Host to guest:
 *     exec <id> <command>   Run `/bin/sh -c <command>` in its own session,
 *                           with stdin on /dev/null and stdout and stderr
 *                           merged. <id> is a number the host chooses.
 *     kill <id>             Send SIGKILL to the job's process group.
 *     ping                  Answer with `pong`.
 *
 *   Guest to host:
 *     hello 1               Sent each time devd opens the port; the number
 *                           is the protocol version.
 *     out <id> <text>       One line of the job's output. A line longer
 *                           than OUT_MAX bytes arrives in pieces.
 *     exit <id> <status>    The job ended: its exit code, or 128 plus the
 *                           signal that ended it. It follows the last `out`.
 *     error <id> <why>      The job did not start.
 *     pong                  The answer to `ping`.
 *
 * A job ends when its process has exited and its output pipe is closed. When
 * the port reopens, devd kills every job, because the host no longer knows
 * about them.
 */

#define _GNU_SOURCE
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

#define PORT_PATH  "/dev/virtio-ports/cdj3k.dev"
#define MAX_JOBS   16
#define OUT_MAX    1024
#define RX_MAX     4096
/* How often finished jobs are reaped while one waits for its exit status. */
#define REAP_MS    100

struct job {
    unsigned id;
    pid_t pid;          /* 0 for a free slot */
    int fd;             /* the read end of the output pipe; -1 once closed */
    int status;         /* valid once `exited` is set */
    int exited;
    char buf[OUT_MAX];
    size_t n;
};

static struct job g_jobs[MAX_JOBS];
static volatile sig_atomic_t g_stop;
static int g_port = -1;

static void on_signal(int sig) { (void)sig; g_stop = 1; }

static int open_port_blocking(void)
{
    for (;;) {
        if (g_stop) return -1;
        int fd = open(PORT_PATH, O_RDWR | O_CLOEXEC);
        if (fd >= 0) return fd;
        struct timespec ts = { .tv_sec = 1, .tv_nsec = 0 };
        nanosleep(&ts, NULL);
    }
}

static void write_all(const char *p, size_t n)
{
    while (n > 0 && g_port >= 0) {
        ssize_t w = write(g_port, p, n);
        if (w < 0) {
            /* SIGTERM interrupts a write that blocks while the host is away. */
            if (errno == EINTR && !g_stop) continue;
            return;
        }
        p += w;
        n -= (size_t)w;
    }
}

static void emit(const char *fmt, ...)
{
    char line[256];
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(line, sizeof(line) - 1, fmt, ap);
    va_end(ap);
    if (n <= 0) return;
    if (n > (int)sizeof(line) - 2) n = (int)sizeof(line) - 2;
    line[n++] = '\n';
    write_all(line, (size_t)n);
}

/* Send `text`, which holds no newline, as one `out` line. */
static void emit_out(unsigned id, const char *text, size_t n)
{
    char head[32];
    int h = snprintf(head, sizeof(head), "out %u ", id);
    char line[sizeof(head) + OUT_MAX + 1];
    memcpy(line, head, (size_t)h);
    memcpy(line + h, text, n);
    line[(size_t)h + n] = '\n';
    write_all(line, (size_t)h + n + 1);
}

static struct job *find_job(unsigned id)
{
    for (int i = 0; i < MAX_JOBS; i++)
        if (g_jobs[i].pid && g_jobs[i].id == id) return &g_jobs[i];
    return NULL;
}

static void start_job(unsigned id, const char *cmd)
{
    if (find_job(id)) { emit("error %u the job number is in use", id); return; }
    struct job *j = NULL;
    for (int i = 0; i < MAX_JOBS && !j; i++)
        if (!g_jobs[i].pid) j = &g_jobs[i];
    if (!j) { emit("error %u %d jobs are already running", id, MAX_JOBS); return; }

    int p[2];
    if (pipe2(p, O_CLOEXEC) < 0) { emit("error %u pipe: %s", id, strerror(errno)); return; }
    pid_t pid = fork();
    if (pid < 0) {
        emit("error %u fork: %s", id, strerror(errno));
        close(p[0]);
        close(p[1]);
        return;
    }
    if (pid == 0) {
        setsid();
        int null = open("/dev/null", O_RDONLY);
        if (null >= 0) dup2(null, 0);
        if (null > 2) close(null);
        dup2(p[1], 1);
        dup2(p[1], 2);
        signal(SIGPIPE, SIG_DFL);
        execl("/bin/sh", "sh", "-c", cmd, (char *)NULL);
        _exit(127);
    }
    close(p[1]);
    memset(j, 0, sizeof(*j));
    j->id = id;
    j->pid = pid;
    j->fd = p[0];
}

/* Send the complete lines in the job's buffer. Send the rest too when it
 * fills the buffer or when `flush` is set. */
static void drain(struct job *j, int flush)
{
    size_t start = 0;
    for (size_t i = 0; i < j->n; i++) {
        if (j->buf[i] == '\n') {
            emit_out(j->id, j->buf + start, i - start);
            start = i + 1;
        }
    }
    size_t rest = j->n - start;
    if (rest == OUT_MAX || (flush && rest > 0)) {
        emit_out(j->id, j->buf + start, rest);
        rest = 0;
        start = j->n;
    }
    memmove(j->buf, j->buf + start, rest);
    j->n = rest;
}

static void read_job(struct job *j)
{
    ssize_t r = read(j->fd, j->buf + j->n, OUT_MAX - j->n);
    if (r < 0 && errno == EINTR) return;
    if (r <= 0) {
        drain(j, 1);
        close(j->fd);
        j->fd = -1;
        return;
    }
    j->n += (size_t)r;
    drain(j, 0);
}

/* Collect exit statuses, and end the jobs whose output is also closed. */
static void reap(void)
{
    for (int i = 0; i < MAX_JOBS; i++) {
        struct job *j = &g_jobs[i];
        if (!j->pid) continue;
        if (!j->exited) {
            int st;
            if (waitpid(j->pid, &st, WNOHANG) == j->pid) {
                j->exited = 1;
                j->status = WIFSIGNALED(st) ? 128 + WTERMSIG(st) : WEXITSTATUS(st);
            }
        }
        if (j->exited && j->fd < 0) {
            emit("exit %u %d", j->id, j->status);
            j->pid = 0;
        }
    }
}

static void kill_job(unsigned id)
{
    /* The group can outlive the shell: a background child keeps the output
     * pipe open, and the job lasts until that pipe closes. */
    struct job *j = find_job(id);
    if (j && (!j->exited || j->fd >= 0)) kill(-j->pid, SIGKILL);
}

static void kill_all(void)
{
    for (int i = 0; i < MAX_JOBS; i++) {
        struct job *j = &g_jobs[i];
        if (!j->pid) continue;
        kill(-j->pid, SIGKILL);
        if (!j->exited) waitpid(j->pid, NULL, 0);
        if (j->fd >= 0) close(j->fd);
        j->pid = 0;
    }
}

static void dispatch(char *line)
{
    unsigned id;
    int used = 0;
    if (strcmp(line, "ping") == 0) {
        emit("pong");
    } else if (sscanf(line, "exec %u %n", &id, &used) == 1 && used > 0) {
        start_job(id, line + used);
    } else if (sscanf(line, "kill %u", &id) == 1) {
        kill_job(id);
    } else {
        fprintf(stderr, "devd: ignoring the line \"%s\"\n", line);
    }
}

static void open_port(void)
{
    kill_all();
    g_port = open_port_blocking();
    if (g_port >= 0) emit("hello 1");
}

int main(void)
{
    signal(SIGPIPE, SIG_IGN);
    /* Without SA_RESTART, so that a stop request interrupts a blocked write. */
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = on_signal;
    sigemptyset(&sa.sa_mask);
    sigaction(SIGINT, &sa, NULL);
    sigaction(SIGTERM, &sa, NULL);

    open_port();
    char rx[RX_MAX];
    size_t rx_n = 0;

    while (!g_stop && g_port >= 0) {
        struct pollfd pfd[MAX_JOBS + 1];
        struct job *owner[MAX_JOBS + 1];
        int n = 0, waiting = 0;
        pfd[n] = (struct pollfd){ .fd = g_port, .events = POLLIN };
        owner[n++] = NULL;
        for (int i = 0; i < MAX_JOBS; i++) {
            if (!g_jobs[i].pid) continue;
            if (g_jobs[i].fd >= 0) {
                pfd[n] = (struct pollfd){ .fd = g_jobs[i].fd, .events = POLLIN };
                owner[n++] = &g_jobs[i];
            } else {
                waiting = 1;
            }
        }
        int pr = poll(pfd, (nfds_t)n, waiting ? REAP_MS : 1000);
        if (pr < 0 && errno != EINTR) break;
        for (int i = 1; pr > 0 && i < n; i++)
            if (pfd[i].revents & (POLLIN | POLLHUP | POLLERR)) read_job(owner[i]);
        reap();
        if (pr <= 0 || !(pfd[0].revents & (POLLIN | POLLHUP | POLLERR))) continue;

        ssize_t r = read(g_port, rx + rx_n, sizeof(rx) - rx_n - 1);
        if (r <= 0) {
            if (r < 0 && errno == EINTR) continue;
            close(g_port);
            rx_n = 0;
            open_port();
            continue;
        }
        rx_n += (size_t)r;
        rx[rx_n] = '\0';
        char *start = rx, *nl;
        while ((nl = strchr(start, '\n')) != NULL) {
            *nl = '\0';
            dispatch(start);
            start = nl + 1;
        }
        size_t rem = rx_n - (size_t)(start - rx);
        memmove(rx, start, rem);
        rx_n = rem;
        if (rx_n >= sizeof(rx) - 1) {
            fprintf(stderr, "devd: a request line is too long; discarding it\n");
            rx_n = 0;
        }
    }
    kill_all();
    return 0;
}
