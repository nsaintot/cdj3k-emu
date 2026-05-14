// SPDX-License-Identifier: MIT OR Apache-2.0
/*
 * subucom_forwarder - bidirectional bridge between /dev/subucom_ctrl and the
 * cdj3k.ctrl virtio-serial port.
 *
 *   LED  (guest→host): subucom_virt.ko writes MOSI frames to /dev/subucom_ctrl
 *                      → this process reads them → writes to vport → ctrl.sock
 *   CTRL (host→guest): ctrl.sock → vport → this process reads → /dev/subucom_ctrl
 *                      → subucom_virt.ko delivers MISO frames to EP122
 *
 * Build (aarch64 static):
 *   aarch64-linux-gnu-gcc -O2 -static -o subucom_forwarder_aarch64 subucom_forwarder.c -lpthread
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

static int ctrl_rfd = -1;  /* O_RDONLY - read MOSI/LED frames */
static int ctrl_wfd = -1;  /* O_WRONLY - write MISO/CTRL frames */
static int vport_fd = -1;  /* O_RDWR  - virtio-serial port     */

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
        fprintf(stderr, "[subucom_forwarder] waiting for %s (r): %s\n",
                CTRL_PATH, strerror(errno));
        usleep(100000);
    }
    return fd;
}

static int open_ctrl_w(void)
{
    int fd;
    while ((fd = open(CTRL_PATH, O_WRONLY)) < 0) {
        fprintf(stderr, "[subucom_forwarder] waiting for %s (w): %s\n",
                CTRL_PATH, strerror(errno));
        usleep(100000);
    }
    return fd;
}

static int open_vport(const char *path)
{
    int fd;
    while ((fd = open(path, O_RDWR)) < 0) {
        fprintf(stderr, "[subucom_forwarder] waiting for %s: %s\n",
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

/* LED thread: /dev/subucom_ctrl → vport (guest→host) */
static void *led_thread(void *arg)
{
    (void)arg;
    uint8_t frame[FRAME_SIZE];
    for (;;) {
        ssize_t r = read_exact(ctrl_rfd, frame, FRAME_SIZE);
        if (r <= 0) {
            fprintf(stderr, "[subucom_forwarder] ctrl read: %s\n",
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
    /* argv[0] must be "systemctl" - some versions special-case basenames like
     * "reboot" / "poweroff" only when invoked through those paths, never when
     * the binary is launched directly as /bin/systemctl. */
    execl("/bin/systemctl", "systemctl", "reboot", (char *)NULL);
    return NULL;
}

int main(void)
{
    ctrl_rfd = open_ctrl_r();
    ctrl_wfd = open_ctrl_w();

    char vport_path[512];
    while (find_vport(vport_path, sizeof(vport_path)) != 0) {
        fprintf(stderr, "[subucom_forwarder] waiting for virtio-serial port '%s'\n",
                PORT_NAME);
        sleep(1);
    }
    vport_fd = open_vport(vport_path);

    fprintf(stderr, "[subucom_forwarder] ready - %s ↔ %s\n", vport_path, CTRL_PATH);

    pthread_t t;
    if (pthread_create(&t, NULL, led_thread, NULL) != 0) {
        fprintf(stderr, "[subucom_forwarder] pthread_create: %s\n", strerror(errno));
        return 1;
    }
    pthread_detach(t);

/* CTRL direction: vport → /dev/subucom_ctrl (host→guest) */
    uint8_t frame[FRAME_SIZE];
    for (;;) {
        ssize_t r = read_exact(vport_fd, frame, FRAME_SIZE);
        if (r == 0) {
            /* host_connected=false: QEMU returns 0 immediately when no client
             * is connected to ctrl.sock.  Keep the vport open - closing and
             * reopening just re-triggers the same path and burns the guest_connected
             * slot unnecessarily.  Sleep and retry; the kernel will unblock once
             * cdj-ui connects and host_connected flips to true. */
            usleep(50000);
            continue;
        }
        if (r < 0) {
            /* Real I/O error - vport was unplugged or an interrupt caused a
             * permanent failure.  Reopen to recover. */
            fprintf(stderr, "[subucom_forwarder] vport read: %s - reopening\n",
                    strerror(errno));
            close(vport_fd);
            usleep(50000);
            vport_fd = open_vport(vport_path);
            continue;
        }

        /* Simulate sub-CPU power-off timer: high nibble of byte 12 = 0 → power off.
         * Latch the first detection - EP122 keeps sending power-off frames until
         * actual shutdown, and we don't need a fresh thread for each one. */
        static int poweroff_armed;
        if (!poweroff_armed && ((frame[12] >> 4) & 0x08U) == 0) {
            fprintf(stderr, "[subucom_forwarder] power-off detected (b12=0x%02x) - rebooting in 2s\n", frame[12]);
            poweroff_armed = 1;
            pthread_t pt;
            pthread_create(&pt, NULL, poweroff_thread, NULL);
            pthread_detach(pt);
        }

        if (write(ctrl_wfd, frame, FRAME_SIZE) < 0) {
            fprintf(stderr, "[subucom_forwarder] ctrl write: %s - reopening\n",
                    strerror(errno));
            close(ctrl_wfd);
            ctrl_wfd = open_ctrl_w();
        }
    }
}
