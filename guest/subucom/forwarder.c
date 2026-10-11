// SPDX-License-Identifier: MIT OR Apache-2.0
/*
 * subucom_forwarder copies frames in both directions between /dev/subucom_ctrl
 * and the cdj3k.ctrl virtio-serial port.
 *
 *   LED frames, guest to host: subucom_virt.ko writes MOSI frames to
 *     /dev/subucom_ctrl, and this process copies them to the virtio-serial
 *     port, which QEMU exposes as ctrl.sock.
 *   Control frames, host to guest: this process copies frames from the
 *     virtio-serial port to /dev/subucom_ctrl, and subucom_virt.ko delivers
 *     them to EP122 as MISO frames.
 *
 * guest/Makefile builds it as the static binary subucom_forwarder_aarch64.
 */

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <dirent.h>
#include <pthread.h>

#define CTRL_PATH   "/dev/subucom_ctrl"
#define PORT_NAME   "cdj3k.ctrl"
#define FRAME_SIZE  64

/* The host's power flag is bit 7 of byte 12. A model whose frame uses byte 12
 * for panel data, the CDJ-1500X, carries the flag at byte 30, in the tail
 * after the CRC and the touch words, which the CRC does not cover. */
#define POWER_BYTE        12
#define POWER_BYTE_1500X  30
#define POWER_BIT         0x80U
#define MODEL_PARAM       "/sys/module/subucom_virt/parameters/model"

static int ctrl_rfd = -1;  /* O_RDONLY: reads MOSI (LED) frames */
static int ctrl_wfd = -1;  /* O_WRONLY: writes MISO (control) frames */
static int vport_fd = -1;  /* O_RDWR: the virtio-serial port */

/* Find the /dev/vportNpM path for a virtio-serial port by name. */
static int find_vport(char *out, size_t out_sz)
{
    DIR *d = opendir("/sys/class/virtio-ports");
    if (!d)
        return -1;

    struct dirent *de;
    while ((de = readdir(d)) != NULL) {
        if (de->d_name[0] == '.')
            continue;

        char name_path[512];
        snprintf(name_path, sizeof(name_path),
                 "/sys/class/virtio-ports/%s/name", de->d_name);

        FILE *f = fopen(name_path, "r");
        if (!f)
            continue;

        char name[64] = {0};
        if (!fgets(name, sizeof(name), f)) {
            fclose(f);
            continue;
        }
        fclose(f);
        name[strcspn(name, "\n")] = '\0';

        if (strcmp(name, PORT_NAME) == 0) {
            snprintf(out, out_sz, "/dev/%s", de->d_name);
            closedir(d);
            return 0;
        }
    }
    closedir(d);
    return -1;
}

static int open_ctrl_r(void)
{
    int fd;
    while ((fd = open(CTRL_PATH, O_RDONLY)) < 0) {
        fprintf(stderr, "[subucom_forwarder] Waiting for %s (read end): %s\n",
                CTRL_PATH, strerror(errno));
        usleep(100000);
    }
    return fd;
}

static int open_ctrl_w(void)
{
    int fd;
    while ((fd = open(CTRL_PATH, O_WRONLY)) < 0) {
        fprintf(stderr, "[subucom_forwarder] Waiting for %s (write end): %s\n",
                CTRL_PATH, strerror(errno));
        usleep(100000);
    }
    return fd;
}

static int open_vport(const char *path)
{
    int fd;
    while ((fd = open(path, O_RDWR)) < 0) {
        fprintf(stderr, "[subucom_forwarder] Waiting for %s: %s\n",
                path, strerror(errno));
        usleep(200000);
    }
    return fd;
}

static ssize_t read_exact(int fd, uint8_t *buf, size_t n)
{
    size_t got = 0;
    while (got < n) {
        ssize_t r = read(fd, buf + got, n - got);
        if (r < 0) {
            if (errno == EINTR) continue;
            return -1;
        }
        if (r == 0)
            return 0;
        got += (size_t)r;
    }
    return (ssize_t)got;
}

/* Copy LED frames from /dev/subucom_ctrl to the virtio-serial port (guest to host). */
static void *led_thread(void *arg)
{
    (void)arg;
    uint8_t frame[FRAME_SIZE];
    for (;;) {
        ssize_t r = read_exact(ctrl_rfd, frame, FRAME_SIZE);
        if (r <= 0) {
            fprintf(stderr, "[subucom_forwarder] Reading /dev/subucom_ctrl failed: %s\n",
                    r < 0 ? strerror(errno) : "EOF");
            exit(1);
        }

        if (write(vport_fd, frame, FRAME_SIZE) < 0) {
            usleep(50000);
        }
    }
    return NULL;
}

static void *poweroff_thread(void *arg)
{
    (void)arg;
    sleep(2);
    /* argv[0] must be "systemctl": systemctl acts as reboot or poweroff when
     * argv[0] has one of those names. */
    execl("/bin/systemctl", "systemctl", "reboot", (char *)NULL);
    return NULL;
}

/* Return the byte that carries the power flag in the loaded model's host
 * frames. */
static int power_byte(void)
{
    char model[16] = {0};
    FILE *f = fopen(MODEL_PARAM, "r");
    if (f) {
        if (!fgets(model, sizeof(model), f))
            model[0] = '\0';
        fclose(f);
    }
    return strncmp(model, "cdj1500x", 8) == 0 ? POWER_BYTE_1500X : POWER_BYTE;
}

int main(void)
{
    ctrl_rfd = open_ctrl_r();
    ctrl_wfd = open_ctrl_w();

    char vport_path[512];
    while (find_vport(vport_path, sizeof(vport_path)) != 0) {
        fprintf(stderr, "[subucom_forwarder] Waiting for the virtio-serial port '%s'.\n",
                PORT_NAME);
        sleep(1);
    }
    vport_fd = open_vport(vport_path);

    const int pb = power_byte();
    fprintf(stderr, "[subucom_forwarder] Ready; forwarding between %s and %s (power flag at byte %d).\n",
            vport_path, CTRL_PATH, pb);

    pthread_t t;
    if (pthread_create(&t, NULL, led_thread, NULL) != 0) {
        fprintf(stderr, "[subucom_forwarder] pthread_create failed: %s\n", strerror(errno));
        return 1;
    }
    pthread_detach(t);

/* Copy control frames from the virtio-serial port to /dev/subucom_ctrl (host to guest). */
    uint8_t frame[FRAME_SIZE];
    for (;;) {
        ssize_t r = read_exact(vport_fd, frame, FRAME_SIZE);
        if (r == 0) {
            /* While no client is connected to ctrl.sock, host_connected is
             * false and a read returns 0 at once. Keep the port open: a
             * reopened port also reads 0 until the UI connects, and each
             * reopen toggles guest_connected on the QEMU side. Sleep and
             * retry; reads block again once the UI connects and
             * host_connected becomes true. */
            usleep(50000);
            continue;
        }
        if (r < 0) {
            /* An I/O error means the port was unplugged or failed
             * permanently; reopen it. */
            fprintf(stderr, "[subucom_forwarder] Reading the virtio-serial port failed: %s; reopening it.\n",
                    strerror(errno));
            close(vport_fd);
            usleep(50000);
            vport_fd = open_vport(vport_path);
            continue;
        }

        /* Emulate the sub-CPU power-off timer: a cleared power flag reboots the
         * guest, where a real deck would cut the power. Act on the first such frame only, because the host keeps
         * sending them until the guest shuts down. */
        static int poweroff_armed;
        if (!poweroff_armed && (frame[pb] & POWER_BIT) == 0) {
            fprintf(stderr, "[subucom_forwarder] The power flag is clear (b%d=0x%02x); rebooting in 2 s.\n",
                    pb, frame[pb]);
            poweroff_armed = 1;
            pthread_t pt;
            pthread_create(&pt, NULL, poweroff_thread, NULL);
            pthread_detach(pt);
        }

        if (write(ctrl_wfd, frame, FRAME_SIZE) < 0) {
            fprintf(stderr, "[subucom_forwarder] Writing /dev/subucom_ctrl failed: %s; reopening it.\n",
                    strerror(errno));
            close(ctrl_wfd);
            ctrl_wfd = open_ctrl_w();
        }
    }
}
