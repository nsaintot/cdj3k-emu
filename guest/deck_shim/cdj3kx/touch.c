/* SPDX-License-Identifier: MIT OR Apache-2.0 */
/* ------------------------------------------------------------------ */
/* CDJ-3000X touch panel - sub-CPU frame → evdev records              */
/* ------------------------------------------------------------------ */
/*
 * The app reads its touch panel as a Goodix kernel input device and hands
 * what it finds to XWarpPointer; QEMU virt has that device nowhere.  The
 * coordinates ride in the sub-CPU frame the app already receives, so read()
 * passes each frame here and the contact is written to a stub pipe as the
 * records the app looks for: EV_ABS with ABS_MT_SLOT, ABS_MT_TRACKING_ID,
 * ABS_MT_POSITION_X/Y, closed by EV_SYN.  The values are X screen pixels.
 *
 * Every gt928 detail lives in this file.  The core carries none and reaches
 * the panel only through the weak `deck_touch_*` hooks below, which a model
 * that keeps touch inside the sub-CPU frame simply does not define.
 */

#include "deck_shim.h"

/* The node the app opens. */
#define GT928_PATH        "/dev/input/by-id/gt928"
#define MAX_GT928_FDS     4

/* linux/input-event-codes.h subset the app matches on */
#define GT_EV_SYN             0x00
#define GT_EV_ABS             0x03
#define GT_SYN_REPORT         0x00
#define GT_ABS_MT_SLOT        0x2f
#define GT_ABS_MT_POSITION_X  0x35
#define GT_ABS_MT_POSITION_Y  0x36
#define GT_ABS_MT_TRACKING_ID 0x39

/* The frame carries touch as two u16 LE at these offsets, already in the
 * model's own units (screen pixels here). Pixel 0 is a coordinate, so a
 * contact is flagged in the X word's top bit (cdj3k_emu_panel::TOUCH_DOWN)
 * rather than told by a non-zero pair. */
#define FRAME_TOUCH_X   20
#define FRAME_TOUCH_Y   22
#define FRAME_TOUCH_DOWN 0x8000u

/* struct input_event, aarch64 (16-byte timeval, then type/code/value). */
struct touch_event {
    uint64_t sec;
    uint64_t usec;
    uint16_t type;
    uint16_t code;
    int32_t  value;
};

/* The stub pipe, created on the first open of the node, so only the process
 * that reads the panel holds it.  Both ends pack into one word (read end
 * high, write end low) so they appear together; PIPE_NONE until then. */
#define PIPE_NONE UINT64_MAX
static uint64_t g_pipe = PIPE_NONE;
static int g_active[MAX_GT928_FDS] = { -1, -1, -1, -1 };

/* Last contact published, so a frame that changes nothing writes nothing.
 * Only the holder of g_publishing touches these. */
static int g_publishing;
static int g_touch_down;
static int g_touch_x;
static int g_touch_y;
static int g_touch_id;

static inline int pipe_rd(uint64_t p) { return (int)(uint32_t)(p >> 32); }
static inline int pipe_wr(uint64_t p) { return (int)(uint32_t)p; }

/* One binary serves every model, so the panel answers only on the one that
 * has it; elsewhere the app never opens the node anyway. */
static int touch_is_ours(void) {
    return deck_model() == DECK_MODEL_CDJ3KX;
}

static uint64_t touch_pipe(void) {
    uint64_t cur = __atomic_load_n(&g_pipe, __ATOMIC_ACQUIRE);
    if (cur != PIPE_NONE) return cur;

    /* The app blocks in select()/read() on the read end, so the pipe stays
     * blocking; the write end is not, and a full pipe drops the sample
     * rather than stalling the thread feeding it. */
    int pfds[2];
    if (syscall(SYS_pipe2, pfds, O_CLOEXEC) != 0) return PIPE_NONE;
    int fl = (int)syscall(SYS_fcntl, pfds[1], F_GETFL, 0);
    if (fl >= 0) syscall(SYS_fcntl, pfds[1], F_SETFL, (long)(fl | O_NONBLOCK));

    uint64_t mine = ((uint64_t)(uint32_t)pfds[0] << 32) | (uint32_t)pfds[1];
    if (__atomic_compare_exchange_n(&g_pipe, &cur, mine, 0,
                                    __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE)) {
        DBG("touch pipe: rd=%d wr=%d\n", pfds[0], pfds[1]);
        return mine;
    }
    /* A concurrent open created it first. */
    sys_close(pfds[0]);
    sys_close(pfds[1]);
    return cur;
}

int deck_touch_owns_path(const char *path) {
    return path && touch_is_ours() && strcmp(path, GT928_PATH) == 0;
}

int deck_touch_open(int flags) {
    uint64_t p = touch_pipe();
    if (p == PIPE_NONE) { errno = ENODEV; return -1; }

    long r = syscall(SYS_fcntl, pipe_rd(p),
                     (flags & O_CLOEXEC) ? F_DUPFD_CLOEXEC : F_DUPFD, 0L);
    if (r < 0) return -1;
    int fd = (int)r;
    if (fdset_add(g_active, MAX_GT928_FDS, fd) < 0) {
        sys_close(fd);
        errno = EMFILE;
        return -1;
    }
    return fd;
}

int deck_touch_is_fd(int fd) {
    return fdset_has(g_active, MAX_GT928_FDS, fd);
}

void deck_touch_close(int fd) {
    fdset_remove(g_active, MAX_GT928_FDS, fd);
}

static void fill(struct touch_event *ev, uint16_t type, uint16_t code,
                 int32_t value, const struct timespec *now) {
    ev->sec   = (uint64_t)now->tv_sec;
    ev->usec  = (uint64_t)(now->tv_nsec / 1000);
    ev->type  = type;
    ev->code  = code;
    ev->value = value;
}

void deck_touch_publish(const unsigned char *frame, size_t n) {
    if (n < 64 || !touch_is_ours()) return;
    uint64_t p = __atomic_load_n(&g_pipe, __ATOMIC_ACQUIRE);
    if (p == PIPE_NONE) return;

    int listening = 0;
    for (int i = 0; i < MAX_GT928_FDS; i++) {
        if (__atomic_load_n(&g_active[i], __ATOMIC_ACQUIRE) >= 0) {
            listening = 1;
            break;
        }
    }
    if (!listening) return;

    /* One publisher at a time, so records reach the pipe in state order.
     * A frame that finds another thread publishing is skipped: the state
     * is a level, and the next frame carries it. */
    if (__atomic_exchange_n(&g_publishing, 1, __ATOMIC_ACQUIRE)) return;

    unsigned rx = (unsigned)frame[FRAME_TOUCH_X]
                | ((unsigned)frame[FRAME_TOUCH_X + 1] << 8);
    unsigned ry = (unsigned)frame[FRAME_TOUCH_Y]
                | ((unsigned)frame[FRAME_TOUCH_Y + 1] << 8);
    int down = (rx & FRAME_TOUCH_DOWN) != 0;
    /* The host sends the model's own units, so the pair travels into the
     * records unchanged once the flag is off. */
    int x = (int)(rx & ~FRAME_TOUCH_DOWN);
    int y = (int)ry;

    if (down == g_touch_down && (!down || (x == g_touch_x && y == g_touch_y)))
        goto out;

    struct timespec now;
    clock_gettime(CLOCK_REALTIME, &now);

    struct touch_event ev[6];
    int k = 0;
    fill(&ev[k++], GT_EV_ABS, GT_ABS_MT_SLOT, 0, &now);
    if (down) {
        if (!g_touch_down)
            fill(&ev[k++], GT_EV_ABS, GT_ABS_MT_TRACKING_ID, g_touch_id + 1, &now);
        fill(&ev[k++], GT_EV_ABS, GT_ABS_MT_POSITION_X, x, &now);
        fill(&ev[k++], GT_EV_ABS, GT_ABS_MT_POSITION_Y, y, &now);
    } else {
        fill(&ev[k++], GT_EV_ABS, GT_ABS_MT_TRACKING_ID, -1, &now);
    }
    fill(&ev[k++], GT_EV_SYN, GT_SYN_REPORT, 0, &now);

    /* pipe full: the next frame carries the state on */
    if (sys_write(pipe_wr(p), ev, (size_t)k * sizeof(ev[0])) < 0) goto out;

    if (down && !g_touch_down) g_touch_id++;
    g_touch_down = down;
    g_touch_x = x;
    g_touch_y = y;
    DBG("touch %s x=%d y=%d\n", down ? "down" : "lift", x, y);
out:
    __atomic_store_n(&g_publishing, 0, __ATOMIC_RELEASE);
}
