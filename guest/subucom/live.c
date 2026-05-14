// SPDX-License-Identifier: MIT OR Apache-2.0
/*
 * subucom_live.c - Live real-time display of subucom SPI frames
 *
 * MISO mode (default): reads /dev/subucom_spi1.0, shows 64-byte control frames
 *   (buttons, jog wheel, potentiometers) at ~850 Hz.
 *
 * MOSI / LED mode (-mosi): reads /dev/subucom_ctrl, shows 64-byte LED frames
 *   written by EP122 at ~100 Hz, with decoded bitfield + pad RGB sections.
 *
 * Common flags:
 *   -1      one-shot: print frame once and exit (plain hex, no ANSI)
 *   -mosi   switch to MOSI / LED mode
 *   -filter <list>  comma-separated byte indices (0-63); only those bytes
 *                    are shown in hex grid, -1, and -list output
 *   -list   stream each frame to stdout as plain hex (no ANSI/cursor); one
 *            line per read; appends only (use with -1 for single line)
 *   -miso-freq <hz>  target display/output rate in MISO mode (default 10)
 *
 * Changed bytes are highlighted in bright yellow (*XX).
 * Zero bytes are dimmed.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/time.h>
#include <signal.h>
#include <ctype.h>

#define PKT_SIZE     64
#define MAX_FILTER   64
#define MISO_DEV     "/dev/subucom_spi1.0"
#define MOSI_DEV     "/dev/subucom_ctrl"
#define DEFAULT_MISO_HZ 10

static int g_fd = -1;
static int g_filter_count = 0;
static int g_filter_idx[MAX_FILTER];
static int g_list_mode = 0;

static int icmp(const void *a, const void *b) {
    int x = *(const int *)a, y = *(const int *)b;
    return (x > y) - (x < y);
}

/* Parse "0,5,10" into g_filter_idx (sorted, unique). */
static void parse_filter_list(const char *s) {
    g_filter_count = 0;
    if (!s || !*s) {
        fprintf(stderr, "-filter: empty list\n");
        exit(1);
    }
    const char *p = s;
    while (*p && g_filter_count < MAX_FILTER) {
        while (*p && (isspace((unsigned char)*p) || *p == ',')) p++;
        if (!*p) break;
        char *end = NULL;
        long v = strtol(p, &end, 0);
        if (end == p) {
            fprintf(stderr, "-filter: invalid token near \"%s\"\n", p);
            exit(1);
        }
        if (v < 0 || v >= PKT_SIZE) {
            fprintf(stderr, "-filter: index %ld out of range 0-%d\n",
                    v, PKT_SIZE - 1);
            exit(1);
        }
        g_filter_idx[g_filter_count++] = (int)v;
        p = end;
    }
    while (*p && (isspace((unsigned char)*p) || *p == ',')) p++;
    if (*p) {
        fprintf(stderr, "-filter: too many indices (max %d)\n", MAX_FILTER);
        exit(1);
    }
    if (g_filter_count == 0) {
        fprintf(stderr, "-filter: no indices parsed\n");
        exit(1);
    }
    qsort(g_filter_idx, (size_t)g_filter_count, sizeof(int), icmp);
    int w = 0;
    for (int r = 0; r < g_filter_count; r++) {
        if (w == 0 || g_filter_idx[r] != g_filter_idx[w - 1])
            g_filter_idx[w++] = g_filter_idx[r];
    }
    g_filter_count = w;
}

static void print_bytes_plain_line(const uint8_t *buf) {
    if (g_filter_count > 0) {
        for (int j = 0; j < g_filter_count; j++) {
            if (j) putchar(' ');
            printf("%02x", buf[g_filter_idx[j]]);
        }
    } else {
        for (int i = 0; i < PKT_SIZE; i++) {
            if (i) putchar(' ');
            printf("%02x", buf[i]);
        }
    }
    putchar('\n');
}

static void cleanup(int sig) {
    (void)sig;
    if (g_list_mode)
        fputc('\n', stdout);
    else
        fprintf(stdout, "\033[?25h\033[0m\n");
    if (g_fd >= 0) close(g_fd);
    exit(0);
}

/* ── helpers ─────────────────────────────────────────────────────────── */

static void print_byte(uint8_t v, int changed) {
    if (changed)
        fprintf(stdout, "\033[1;33m*%02x\033[0m", v);
    else if (v == 0)
        fprintf(stdout, "\033[2m %02x\033[0m", v);
    else
        fprintf(stdout, " %02x", v);
}

static void render_hex_grid_filtered(const uint8_t *curr, const uint8_t *prev,
                                     int data_row_offset) {
    const int per_line = 8;
    int j = 0, line = 0;
    while (j < g_filter_count) {
        fprintf(stdout, "\033[%d;1H\033[K", data_row_offset + line);
        fprintf(stdout, "%s", line == 0 ? "filter  " : "        ");
        for (int k = 0; k < per_line && j < g_filter_count; k++, j++) {
            int idx = g_filter_idx[j];
            fprintf(stdout, "b%02d", idx);
            print_byte(curr[idx], curr[idx] != prev[idx]);
            fprintf(stdout, "  ");
        }
        line++;
    }
    for (; line < PKT_SIZE / 16; line++) {
        fprintf(stdout, "\033[%d;1H\033[K", data_row_offset + line);
    }
}

static void render_hex_grid(const uint8_t *curr, const uint8_t *prev, int data_row_offset) {
    if (g_filter_count > 0) {
        render_hex_grid_filtered(curr, prev, data_row_offset);
        return;
    }
    for (int row = 0; row < PKT_SIZE / 16; row++) {
        fprintf(stdout, "\033[%d;1H", data_row_offset + row);
        fprintf(stdout, "b%02d  ", row * 16);
        for (int col = 0; col < 16; col++) {
            int idx = row * 16 + col;
            print_byte(curr[idx], curr[idx] != prev[idx]);
        }
    }
}

/* ── MISO mode ───────────────────────────────────────────────────────── */

static void run_miso(int fd, int one_shot, int refresh_us) {
    if (one_shot) {
        uint8_t buf[PKT_SIZE];
        int attempts = 0, nonzero = 0;
        do {
            ssize_t n = read(fd, buf, PKT_SIZE);
            if (n != PKT_SIZE) { perror("read"); exit(1); }
            for (int i = 0; i < PKT_SIZE; i++) if (buf[i]) { nonzero = 1; break; }
            if (!nonzero) usleep(5000);
        } while (!nonzero && ++attempts < 100);
        if (g_filter_count > 0) {
            print_bytes_plain_line(buf);
            return;
        }
        for (int i = 0; i < PKT_SIZE; i++)
            printf("%02x%c", buf[i], (i % 16 == 15) ? '\n' : ' ');
        return;
    }

    uint8_t prev[PKT_SIZE], curr[PKT_SIZE];
    memset(prev, 0, PKT_SIZE);

    fprintf(stdout, "\033[?25l\033[2J\033[H");
    fprintf(stdout, "subucom_live - CDJ-3000 MISO 64 bytes  (Ctrl-C to quit)\n");
    fprintf(stdout, "Changed bytes show as *XX  |  row = byte index / 16\n\n");
    fprintf(stdout, "     ");
    for (int i = 0; i < 16; i++) fprintf(stdout, " %02x", i);
    fprintf(stdout, "\n     ");
    for (int i = 0; i < 16; i++) fprintf(stdout, " --");
    fprintf(stdout, "\n");
    fflush(stdout);

    const int DATA_ROW_OFFSET = 6;

    for (;;) {
        ssize_t n = read(fd, curr, PKT_SIZE);
        if (n != PKT_SIZE) { usleep(refresh_us / 4); continue; }

        render_hex_grid(curr, prev, DATA_ROW_OFFSET);

        fprintf(stdout, "\033[%d;1H", DATA_ROW_OFFSET + (PKT_SIZE / 16) + 1);
        struct timeval tv; gettimeofday(&tv, NULL);
        fprintf(stdout, "\033[K[%.3f]", tv.tv_sec + tv.tv_usec / 1e6);

        fflush(stdout);
        memcpy(prev, curr, PKT_SIZE);
        usleep(refresh_us);
    }
}

/* Plain streaming: one space-separated hex line per frame, no ANSI. */
static void run_miso_list(int fd, int one_shot, int refresh_us) {
    uint8_t buf[PKT_SIZE];

    if (one_shot) {
        int attempts = 0, nonzero = 0;
        do {
            ssize_t n = read(fd, buf, PKT_SIZE);
            if (n != PKT_SIZE) { perror("read"); exit(1); }
            for (int i = 0; i < PKT_SIZE; i++) if (buf[i]) { nonzero = 1; break; }
            if (!nonzero) usleep(5000);
        } while (!nonzero && ++attempts < 100);
        print_bytes_plain_line(buf);
        return;
    }

    for (;;) {
        ssize_t n = read(fd, buf, PKT_SIZE);
        if (n != PKT_SIZE) { usleep(refresh_us / 4); continue; }
        print_bytes_plain_line(buf);
        fflush(stdout);
        usleep(refresh_us);
    }
}

/* ── MOSI / LED mode ─────────────────────────────────────────────────── */

/*
 * LED frame layout (64 bytes):
 *
 *  [00-01]  0x0000        frame type / header (always zero)
 *  [02-11]  bitfield      monochromatic LEDs: direct GPIO + shift registers
 *             byte 05 bits 7:6 = 0xc0 always set (IC6003 dim nav LEDs)
 *             byte 10 bits 5:4 = 0x30 always set (IC6003 dim nav LEDs)
 *  [12-35]  8 × {R,G,B}  HOT CUE A-H pad LEDs  (full white = 44 78 7f)
 *  [36-38]  {R,G,B}       SOURCE indicator      (full = 88 f0 ff)
 *  [39-41]  {R,G,B}       SD indicator
 *  [42-44]  {R,G,B}       USB indicator
 *  [45-61]  zeros         unused / padding
 *  [62-63]  CRC-16        over bytes 0-61 (algorithm TBD)
 */

static void render_led_decoded(const uint8_t *f, const uint8_t *prev,
                                int start_row) {
    int row = start_row;

    /* ── Bitfield [02-11] ── */
    fprintf(stdout, "\033[%d;1H\033[K", row++);
    fprintf(stdout, "\033[1m Bitfield [02-11]\033[0m  "
                    "(dim-always-on: b05&0xc0, b10&0x30)");

    for (int b = 2; b <= 11; b++) {
        if ((b - 2) % 4 == 0) {
            fprintf(stdout, "\033[%d;1H\033[K", row++);
            fprintf(stdout, "  ");
        }
        int changed = (f[b] != prev[b]);
        if (changed)
            fprintf(stdout, "\033[1;33m");
        fprintf(stdout, "b%02d:", b);
        for (int bit = 7; bit >= 0; bit--)
            fprintf(stdout, "%c", (f[b] >> bit) & 1 ? '1' : '0');
        if (changed)
            fprintf(stdout, "\033[0m");
        fprintf(stdout, "  ");
    }

    /* ── HOT CUE pads A-H [12-35] ── */
    fprintf(stdout, "\033[%d;1H\033[K", row++);
    fprintf(stdout, "\033[1m Pads A-H [12-35]\033[0m  R/G/B:");

    fprintf(stdout, "\033[%d;1H\033[K", row++);
    fprintf(stdout, "  ");
    const char *pad_names[] = { "A","B","C","D","E","F","G","H" };
    for (int p = 0; p < 8; p++) {
        int base = 12 + p * 3;
        int ch = (f[base]   != prev[base]   ||
                  f[base+1] != prev[base+1] ||
                  f[base+2] != prev[base+2]);
        if (ch) fprintf(stdout, "\033[1;33m");
        fprintf(stdout, "%s:%02x/%02x/%02x  ", pad_names[p],
                f[base], f[base+1], f[base+2]);
        if (ch) fprintf(stdout, "\033[0m");
    }

    /* ── Indicators [36-44] ── */
    fprintf(stdout, "\033[%d;1H\033[K", row++);
    fprintf(stdout, "\033[1m Indicators [36-44]\033[0m  R/G/B:");

    fprintf(stdout, "\033[%d;1H\033[K", row++);
    fprintf(stdout, "  ");
    struct { int base; const char *name; } inds[] = {
        {36, "SOURCE"}, {39, "SD"}, {42, "USB"}
    };
    for (int i = 0; i < 3; i++) {
        int b = inds[i].base;
        int ch = (f[b] != prev[b] || f[b+1] != prev[b+1] || f[b+2] != prev[b+2]);
        if (ch) fprintf(stdout, "\033[1;33m");
        fprintf(stdout, "%s:%02x/%02x/%02x  ", inds[i].name, f[b], f[b+1], f[b+2]);
        if (ch) fprintf(stdout, "\033[0m");
    }

    /* ── CRC ── */
    fprintf(stdout, "\033[%d;1H\033[K", row);
    int crc_ch = (f[62] != prev[62] || f[63] != prev[63]);
    if (crc_ch) fprintf(stdout, "\033[1;33m");
    fprintf(stdout, "  CRC[62-63]: %02x %02x", f[62], f[63]);
    if (crc_ch) fprintf(stdout, "\033[0m");
}

static void run_mosi(int fd, int one_shot) {
    if (one_shot) {
        uint8_t buf[PKT_SIZE];
        ssize_t n = read(fd, buf, PKT_SIZE);
        if (n != PKT_SIZE) { perror("read"); exit(1); }
        if (g_filter_count > 0) {
            print_bytes_plain_line(buf);
            return;
        }
        for (int i = 0; i < PKT_SIZE; i++)
            printf("%02x%c", buf[i], (i % 16 == 15) ? '\n' : ' ');
        return;
    }

    uint8_t prev[PKT_SIZE], curr[PKT_SIZE];
    memset(prev, 0, PKT_SIZE);

    fprintf(stdout, "\033[?25l\033[2J\033[H");
    fprintf(stdout, "subucom_live - CDJ-3000 LED frame (MOSI 64 bytes)  (Ctrl-C to quit)\n");
    fprintf(stdout, "Changed bytes show as *XX\n\n");
    fprintf(stdout, "     ");
    for (int i = 0; i < 16; i++) fprintf(stdout, " %02x", i);
    fprintf(stdout, "\n     ");
    for (int i = 0; i < 16; i++) fprintf(stdout, " --");
    fprintf(stdout, "\n");
    fflush(stdout);

    /* layout:
     *   line 1:   title
     *   line 2:   subtitle
     *   line 3:   blank
     *   line 4:   column index header
     *   line 5:   dashes
     *   lines 6-9:  hex grid (4 rows × 16 bytes)
     *   line 10:  blank
     *   lines 11+:  decoded sections */
    const int DATA_ROW_OFFSET = 6;
    const int DECODED_OFFSET  = DATA_ROW_OFFSET + (PKT_SIZE / 16) + 2;

    for (;;) {
        /* ctrl_read blocks until a fresh LED frame is available */
        ssize_t n = read(fd, curr, PKT_SIZE);
        if (n != PKT_SIZE) { usleep(10000); continue; }

        render_hex_grid(curr, prev, DATA_ROW_OFFSET);
        if (g_filter_count == 0)
            render_led_decoded(curr, prev, DECODED_OFFSET);

        if (g_filter_count == 0)
            fprintf(stdout, "\033[%d;1H\033[K", DECODED_OFFSET + 9);
        else
            fprintf(stdout, "\033[%d;1H\033[K", DATA_ROW_OFFSET + (PKT_SIZE / 16) + 1);
        struct timeval tv; gettimeofday(&tv, NULL);
        fprintf(stdout, "[%.3f]", tv.tv_sec + tv.tv_usec / 1e6);

        fflush(stdout);
        memcpy(prev, curr, PKT_SIZE);
    }
}

static void run_mosi_list(int fd, int one_shot) {
    uint8_t buf[PKT_SIZE];

    for (;;) {
        ssize_t n = read(fd, buf, PKT_SIZE);
        if (n != PKT_SIZE) { usleep(10000); continue; }
        print_bytes_plain_line(buf);
        fflush(stdout);
        if (one_shot) return;
    }
}

/* ── main ────────────────────────────────────────────────────────────── */

int main(int argc, char **argv) {
    int one_shot = 0, mosi_mode = 0;
    int miso_hz = DEFAULT_MISO_HZ;

    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "-1") == 0) {
            one_shot = 1;
            continue;
        }
        if (strcmp(argv[i], "-mosi") == 0) {
            mosi_mode = 1;
            continue;
        }
        if (strcmp(argv[i], "-list") == 0) {
            g_list_mode = 1;
            continue;
        }
        if (strncmp(argv[i], "-filter=", 8) == 0) {
            parse_filter_list(argv[i] + 8);
            continue;
        }
        if (strcmp(argv[i], "-filter") == 0) {
            if (i + 1 >= argc) {
                fprintf(stderr, "-filter requires a comma-separated list\n");
                return 1;
            }
            parse_filter_list(argv[++i]);
            continue;
        }
        if (strcmp(argv[i], "-miso-freq") == 0) {
            if (i + 1 >= argc) {
                fprintf(stderr, "-miso-freq requires a Hz value\n");
                return 1;
            }
            char *end = NULL;
            long v = strtol(argv[++i], &end, 10);
            if (end == argv[i] || *end != '\0' || v <= 0 || v > 10000) {
                fprintf(stderr, "-miso-freq must be an integer in range 1-10000\n");
                return 1;
            }
            miso_hz = (int)v;
            continue;
        }
        fprintf(stderr, "unknown argument: %s\n", argv[i]);
        return 1;
    }

    int refresh_us = 1000000 / miso_hz;
    if (refresh_us <= 0) refresh_us = 1;

    signal(SIGINT,  cleanup);
    signal(SIGTERM, cleanup);

    const char *dev = mosi_mode ? MOSI_DEV : MISO_DEV;
    int fd = open(dev, O_RDWR);
    if (fd < 0) { perror(dev); return 1; }
    g_fd = fd;

    if (g_list_mode) {
        if (mosi_mode)
            run_mosi_list(fd, one_shot);
        else
            run_miso_list(fd, one_shot, refresh_us);
    } else if (mosi_mode) {
        run_mosi(fd, one_shot);
    } else {
        run_miso(fd, one_shot, refresh_us);
    }

    close(fd);
    return 0;
}
