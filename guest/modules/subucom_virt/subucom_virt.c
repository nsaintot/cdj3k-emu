// SPDX-License-Identifier: GPL-2.0
/*
 * subucom_virt.c - a virtual subucom_spi character device for cdj3k-emu.
 *
 * Creates /dev/subucom_spi1.0, or /dev/subucom_spi3.0 for model=cdj1500x
 * (the node that the CDJ-1500X's app opens), under class "subucom_virt_class"
 * with a dynamic major and minor 0.
 *
 * Also creates /dev/subucom_ctrl (minor 1) for the host. Writing 64 bytes to
 * it injects a MISO frame, which the next read() of spi1.0 delivers. Reading
 * 64 bytes from it returns the latest LED frame that EP122 wrote.
 *
 * Ioctls on the sub-CPU device:
 *   | Code       | Name                | Effect                                   |
 *   |------------|---------------------|------------------------------------------|
 *   | 0x80027003 | TIMER_STATUS_READ   | returns the u16 timer_status (default 0) |
 *   | 0x40027003 | TIMER_STATUS_WRITE  | sets timer_status                        |
 *   | 0x80017001 | BITS_PER_WORD_READ  | returns the u8 bits_per_word (default 8) |
 *   | 0x40017001 | BITS_PER_WORD_WRITE | sets bits_per_word                       |
 *   | 0x40107000 | MOSI_TRANSFER       | EP122 sends a 16-byte MOSI command       |
 *   | 0x80047004 | RX_BYTES_READ       | returns the u32 rx frame size (64)       |
 *   | 0x40047004 | RX_BYTES_WRITE      | sets the rx frame size                   |
 *   | 0x80047002 | INTERVAL_READ       | returns the u32 interval in µs (1176)    |
 *   | 0x40047002 | INTERVAL_WRITE      | sets the interval                        |
 *
 * A read() on spi1.0 waits for the next MISO frame (about 850 Hz) and returns
 * 64 bytes. If the host injected a frame through ctrl, that frame is
 * returned; otherwise the idle frame is.
 *
 * A write() on spi1.0 is a reset request from subucom_reset. When the file
 * that received a valid reset frame is closed, the guest reboots, the way the
 * sub-CPU resets the SoC on a real deck.
 *
 * The real device:
 *   SPI: ff1d0000.spi, bus 1 CS0, 3.2 MHz, mode 3 (CPOL=1, CPHA=1)
 *   Device class: subucom_spiclass
 *   Major: 246, minor: 0
 *   Module parameters: DEFtmstat=0, txmulti=1, txpart=1, SZbufsize=1036
 */

#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/fs.h>
#include <linux/cdev.h>
#include <linux/device.h>
#include <linux/uaccess.h>
#include <linux/slab.h>
#include <linux/string.h>  /* strcmp */
#include <linux/wait.h>
#include <linux/spinlock.h>
#include <linux/delay.h>
#include <linux/sched.h>   /* schedule_timeout_interruptible */
#include <linux/poll.h>    /* poll_table, POLLIN, POLLRDNORM */
#include <linux/workqueue.h> /* delayed_work */
#include <linux/version.h>
#include <linux/reboot.h>  /* orderly_reboot */

#define DRV_NAME        "subucom_virt"
#define SPI_DEV_NAME    "subucom_spi1.0"
#define SPI_DEV_NAME_1500X "subucom_spi3.0"
/* The offset of the CDJ-1500X's frame CRC, which covers the bytes before it. */
#define MISO_CRC_1500X  24
#define CTRL_DEV_NAME   "subucom_ctrl"
#define CLASS_NAME      "subucom_virt_class"

/* alloc_chrdev_region picks a free major at load time. */
static int subucom_major;
#define MINOR_SPI       0
#define MINOR_CTRL      1
#define NDEVS           2

/* Frame sizes and the polling interval. */
#define MISO_SIZE       64
#define MOSI_CMD_SIZE   16  /* ioctl arg: {uint32 magic, uint32 size, uint64 data_ptr} */
#define LED_FRAME_SIZE  64  /* the LED payload that data_ptr points to */
#define INTERVAL_US     1176   /* about 850 Hz */

/* The layout of the 16-byte MOSI ioctl argument. */
struct mosi_cmd {
    u32 magic;    /* 0x01000000 */
    u32 size;     /* byte count at data_ptr, at most LED_FRAME_SIZE */
    u64 data_ptr; /* userspace VA of the LED frame */
} __packed;

/* Ioctl codes */
#define IOCTL_TIMER_STATUS_READ   0x80027003u
#define IOCTL_TIMER_STATUS_WRITE  0x40027003u
#define IOCTL_BITS_PER_WORD_READ  0x80017001u
#define IOCTL_BITS_PER_WORD_WRITE 0x40017001u
#define IOCTL_MOSI_TRANSFER       0x40107000u
#define IOCTL_RX_BYTES_READ       0x80047004u
#define IOCTL_RX_BYTES_WRITE      0x40047004u
#define IOCTL_INTERVAL_READ       0x80047002u
#define IOCTL_INTERVAL_WRITE      0x40047002u

/* The CDJ-3000 idle MISO frame. */
static const u8 miso_idle[MISO_SIZE] = {
    /* b00-b03: header; b04: direction rocker (0x03 is forward) */
    0x00, 0x00, 0x01, 0x04, 0x03,
    /* b05-b11: button bitmasks */
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    /* b12-b15: device state and rotary encoder (idle values) */
    0x81, 0x00, 0xff, 0xff,
    /* b16-b19: LCD touch */
    0x00, 0x00, 0x00, 0x00,
    /* b20-b21: unknown */
    0x00, 0x7f,
    /* b22-b25: tempo slider and vinyl */
    0x50, 0x7f, 0x00, 0x00,
    /* b26-b31: jog wheel (position 0xffff, velocity 0xffff, touch 0x00) */
    0xff, 0xff, 0xff, 0xff, 0x00, 0x00,
    /* b32-b43: cap sensor baseline */
    0x51, 0x01,
    0xd5, 0xd6, 0xd6, 0xd5, 0xd5, 0xd6, 0xd6, 0xd5, 0xd5, 0xd6,
    /* b44-b61: padding */
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00,
    /* b62-b63: crc16-x25 */
    0x00, 0x00,
};

/* ------------------------------------------------------------------ */
/* Global state                                                         */
/* ------------------------------------------------------------------ */

static struct class  *subucom_class;
static struct cdev    subucom_cdev[NDEVS];
static struct device *subucom_dev[NDEVS];

/* CRC-16/X-25: poly 0x8408 (reflected 0x1021), init 0xFFFF, final XOR 0xFFFF.
 * The 64-byte frames carry it over the first 62 bytes, stored LE in b62-b63. */
static u16 crc16_x25(const u8 *data, int n)
{
    u16 crc = 0xFFFF;
    int i, b;
    for (i = 0; i < n; i++) {
        crc ^= data[i];
        for (b = 0; b < 8; b++)
            crc = (crc & 1) ? (crc >> 1) ^ 0x8408u : crc >> 1;
    }
    return crc ^ 0xFFFF;
}

/* MISO state. spi_read() sleeps for interval_us in the caller's context and
 * then delivers the frame. Do not use an hrtimer here: its wake-up races with
 * wait_event under QEMU with HVF. */
static DEFINE_SPINLOCK(miso_lock);
static u8 inject_pending[MISO_SIZE];
static int has_inject;   /* 1 when inject_pending is valid */

/* MOSI: EP122 writes LED frames through the MOSI_TRANSFER ioctl. mosi_frame
 * holds the last 64-byte LED payload, copied from the command's data_ptr. */
static DEFINE_SPINLOCK(mosi_lock);
static u8 mosi_frame[LED_FRAME_SIZE];
static int mosi_fresh;   /* 1 when an unread LED frame is available */
static wait_queue_head_t mosi_wq;

/* Parameters that the ioctls read and set. */
static u16 timer_status;
static u8  bits_per_word = 8;
static u32 rx_bytes      = MISO_SIZE;
static u32 interval_us   = INTERVAL_US;

/* ------------------------------------------------------------------ */
/* Model                                                              */
/*                                                                    */
/* model=cdj3k (CDJ-3000, default), cdj3kx (CDJ-3000X) or cdj1500x    */
/* (CDJ-1500X). The CDJ-3000X's sub-CPU frame is the CDJ-3000 frame   */
/* shifted by 4 bytes, after a u16 format version, which the app      */
/* requires to be non-zero, and 2 pad bytes. The bit that is the      */
/* SD-cover switch on the CDJ-3000 (bit 0 of byte 16 in the shifted   */
/* frame) is a button on the CDJ-3000X. The CDJ-1500X's frame is 32   */
/* bytes long: a format version at byte 0, which must be non-zero,    */
/* data up to byte 23, and the CRC at bytes 24-25. Its idle frame is  */
/* all zeros apart from the version.                                  */
/* ------------------------------------------------------------------ */

static char *model = "cdj3k";
module_param(model, charp, 0444);
MODULE_PARM_DESC(model, "The player: cdj3k for the CDJ-3000 (default), cdj3kx for the CDJ-3000X, cdj1500x for the CDJ-1500X");

/* Resolved once at init; the parameter is read-only afterwards. */
static bool model_is_x;
static bool model_is_1500x;

/* The sub-CPU device node that the loaded model's app opens. */
static const char *spi_dev_name(void)
{
    return model_is_1500x ? SPI_DEV_NAME_1500X : SPI_DEV_NAME;
}

/* The idle frame of the loaded model, derived from miso_idle at init. */
static u8 miso_idle_model[MISO_SIZE];

/* ------------------------------------------------------------------ */
/* Testmode boot injection                                            */
/*                                                                    */
/* inject_testmode=1 holds the service-mode combo for about 5 s, so   */
/* subucom_read sees it during boot. The combo is BTN_TEMPO_RANGE and */
/* BTN_CALL_PREV on the CDJ-3000 (byte 6 0x04, byte 8 0x08), byte 10  */
/* 0x04 and byte 12 0x01 on the CDJ-3000X, and SHIFT and HOT CUE B on */
/* the CDJ-1500X (byte 17 0x01, byte 18 0x40). subucom_read compares  */
/* whole bytes. The combo is ORed into every frame that spi_read      */
/* delivers, idle or injected, so host writes do not cancel it. A     */
/* delayed_work releases it before the app starts its 850 Hz loop.    */
/* ------------------------------------------------------------------ */

static int inject_testmode;
module_param(inject_testmode, int, 0444);
MODULE_PARM_DESC(inject_testmode, "Hold the testmode buttons for about 5 s after loading (0 off, 1 on)");

#define TESTMODE_INJECT_MS  5000

/* The combo bytes and masks of the loaded model, which init sets. */
static u8 testmode_byte[2];
static u8 testmode_mask[2];
static int testmode_held;   /* 1 while the combo is ORed into every frame */

static void testmode_clear_work_fn(struct work_struct *work);
static DECLARE_DELAYED_WORK(testmode_clear_work, testmode_clear_work_fn);

static void testmode_clear_work_fn(struct work_struct *work)
{
    unsigned long flags;
    spin_lock_irqsave(&miso_lock, flags);
    testmode_held = 0;
    spin_unlock_irqrestore(&miso_lock, flags);
    pr_info("%s: released the testmode combo after %d ms\n", DRV_NAME, TESTMODE_INJECT_MS);
}

/* Copy to or from user space; return -EFAULT when the copy fails. */
static inline int xput_bytes(void __user *dst, const u8 *src, size_t n)
{
    return copy_to_user(dst, src, n) ? -EFAULT : 0;
}

static inline int xget_bytes(u8 *dst, const void __user *src, size_t n)
{
    return copy_from_user(dst, src, n) ? -EFAULT : 0;
}

/* ------------------------------------------------------------------ */
/* spi1.0 file operations                                             */
/* ------------------------------------------------------------------ */

static int spi_open(struct inode *inode, struct file *filp)
{
    return 0;
}

static int spi_release(struct inode *inode, struct file *filp)
{
    if (filp->private_data) {
        pr_info(DRV_NAME ": the sub-CPU received a reset request; rebooting\n");
        orderly_reboot();
    }
    return 0;
}

/* subucom_reset writes 64-byte frames with a CRC-16/X-25 over the first 62
 * bytes, little-endian in the last two. The app sends its LED frames through
 * MOSI_TRANSFER and never calls write(), so any valid frame here is a reset
 * request. spi_release() acts on it once subucom_reset closes the file, after
 * its last frame. */
static ssize_t spi_write(struct file *filp, const char __user *buf,
                         size_t len, loff_t *off)
{
    u8 frame[MISO_SIZE];
    u16 crc;

    if (len != MISO_SIZE)
        return -EINVAL;
    if (xget_bytes(frame, buf, MISO_SIZE))
        return -EFAULT;
    crc = crc16_x25(frame, 62);
    if (frame[62] != (u8)(crc & 0xFF) || frame[63] != (u8)(crc >> 8))
        return -EINVAL;
    filp->private_data = (void *)1;
    return (ssize_t)len;
}

static ssize_t spi_read(struct file *filp, char __user *buf,
                         size_t count, loff_t *ppos)
{
    unsigned long flags;
    u8 frame[MISO_SIZE];
    size_t n;

    /* Throttle to about 850 Hz in the caller's context.
     * schedule_timeout_interruptible() returns early when a signal is
     * pending. */
    if (schedule_timeout_interruptible(usecs_to_jiffies(interval_us) + 1))
        return -ERESTARTSYS;

    /* Build the MISO frame from the injected or idle frame, then stamp its
     * CRC-16/X-25. */
    spin_lock_irqsave(&miso_lock, flags);
    if (has_inject) {
        memcpy(frame, inject_pending, MISO_SIZE);
        /* has_inject stays 1: the injected frame stays in effect until
         * ctrl_write() replaces it, for example with the idle frame when a
         * button is released. */
    } else {
        memcpy(frame, miso_idle_model, MISO_SIZE);
    }
    if (testmode_held) {
        frame[testmode_byte[0]] |= testmode_mask[0];
        frame[testmode_byte[1]] |= testmode_mask[1];
    }
    spin_unlock_irqrestore(&miso_lock, flags);

    if (model_is_1500x) {
        /* CRC-16/X-25 over bytes 0-23, stored big-endian at bytes 24-25. */
        u16 crc = crc16_x25(frame, MISO_CRC_1500X);
        frame[MISO_CRC_1500X] = (u8)(crc >> 8);
        frame[MISO_CRC_1500X + 1] = (u8)(crc & 0xFF);
    } else {
        u16 crc = crc16_x25(frame, 62);
        frame[62] = (u8)(crc & 0xFF);
        frame[63] = (u8)(crc >> 8);
    }

    n = min(count, (size_t)MISO_SIZE);
    if (xput_bytes(buf, frame, n))
        return -EFAULT;

    return (ssize_t)n;
}

static long spi_ioctl(struct file *filp, unsigned int cmd, unsigned long arg)
{
    switch (cmd) {

    case IOCTL_TIMER_STATUS_READ:
        return xput_bytes((void __user *)arg, (u8 *)&timer_status, sizeof(timer_status));

    case IOCTL_TIMER_STATUS_WRITE:
        return xget_bytes((u8 *)&timer_status, (const void __user *)arg, sizeof(timer_status));

    case IOCTL_BITS_PER_WORD_READ:
        return xput_bytes((void __user *)arg, &bits_per_word, sizeof(bits_per_word));

    case IOCTL_BITS_PER_WORD_WRITE:
        return xget_bytes(&bits_per_word, (const void __user *)arg, sizeof(bits_per_word));

    case IOCTL_MOSI_TRANSFER: {
        /* EP122 sends a 16-byte command {magic 0x01000000, size, data_ptr}.
         * data_ptr is a user-space address of the LED frame, which is copied
         * here. */
        struct mosi_cmd cmd;
        u8 led[LED_FRAME_SIZE];
        u32 copy_size;
        unsigned long flags;

        if (xget_bytes((u8 *)&cmd, (const void __user *)arg, MOSI_CMD_SIZE))
            return -EFAULT;

        copy_size = min_t(u32, cmd.size, LED_FRAME_SIZE);
        if (copy_size == 0)
            return 0;

        memset(led, 0, LED_FRAME_SIZE);
        if (xget_bytes(led, (const void __user *)(uintptr_t)cmd.data_ptr, copy_size))
            return -EFAULT;

        spin_lock_irqsave(&mosi_lock, flags);
        memcpy(mosi_frame, led, LED_FRAME_SIZE);
        mosi_fresh = 1;
        spin_unlock_irqrestore(&mosi_lock, flags);
        wake_up_interruptible(&mosi_wq);
        return 0;
    }

    case IOCTL_RX_BYTES_READ:
        return xput_bytes((void __user *)arg, (u8 *)&rx_bytes, sizeof(rx_bytes));

    case IOCTL_RX_BYTES_WRITE:
        return xget_bytes((u8 *)&rx_bytes, (const void __user *)arg, sizeof(rx_bytes));

    case IOCTL_INTERVAL_READ:
        return xput_bytes((void __user *)arg, (u8 *)&interval_us, sizeof(interval_us));

    case IOCTL_INTERVAL_WRITE:
        return xget_bytes((u8 *)&interval_us, (const void __user *)arg, sizeof(interval_us));

    default:
        return -ENOTTY;
    }
}

/* spi_read() always delivers a frame after a short sleep (about 1.2 ms), so
 * the fd is always readable; EP122's read() calls set the 850 Hz pace.
 * Returning POLLIN unconditionally lets epoll_ctl() succeed, so EP122's event
 * loop can watch this fd. */
static unsigned int spi_poll(struct file *filp, poll_table *wait)
{
    return POLLIN | POLLRDNORM;
}

static const struct file_operations spi_fops = {
    .owner          = THIS_MODULE,
    .open           = spi_open,
    .release        = spi_release,
    .read           = spi_read,
    .write          = spi_write,
    .poll           = spi_poll,
    .unlocked_ioctl = spi_ioctl,
};

/* ------------------------------------------------------------------ */
/* ctrl file operations (/dev/subucom_ctrl)                           */
/* ------------------------------------------------------------------ */

static int ctrl_open(struct inode *inode, struct file *filp)
{
    return 0;
}

static int ctrl_release(struct inode *inode, struct file *filp)
{
    return 0;
}

/* A 64-byte write injects a MISO frame, which stays in effect until the next
 * write. A zero-byte write clears the injected frame, so reads return the
 * idle frame again. */
static ssize_t ctrl_write(struct file *filp, const char __user *buf,
                           size_t count, loff_t *ppos)
{
    unsigned long flags;

    if (count == 0) {
        spin_lock_irqsave(&miso_lock, flags);
        has_inject = 0;
        spin_unlock_irqrestore(&miso_lock, flags);
        return 0;
    }

    if (count < MISO_SIZE)
        return -EINVAL;

    {
        u8 tmp[MISO_SIZE];
        if (xget_bytes(tmp, buf, MISO_SIZE))
            return -EFAULT;
        spin_lock_irqsave(&miso_lock, flags);
        memcpy(inject_pending, tmp, MISO_SIZE);
        has_inject = 1;
        spin_unlock_irqrestore(&miso_lock, flags);
    }

    return MISO_SIZE;
}

/* A 64-byte read returns the latest LED frame, and blocks until one is
 * available. */
static ssize_t ctrl_read(struct file *filp, char __user *buf,
                          size_t count, loff_t *ppos)
{
    unsigned long flags;
    u8 tmp[LED_FRAME_SIZE];
    int ret;

    if (count < LED_FRAME_SIZE)
        return -EINVAL;

    ret = wait_event_interruptible(mosi_wq, mosi_fresh);
    if (ret)
        return ret;

    spin_lock_irqsave(&mosi_lock, flags);
    memcpy(tmp, mosi_frame, LED_FRAME_SIZE);
    mosi_fresh = 0;
    spin_unlock_irqrestore(&mosi_lock, flags);

    if (xput_bytes(buf, tmp, LED_FRAME_SIZE))
        return -EFAULT;

    return LED_FRAME_SIZE;
}

static const struct file_operations ctrl_fops = {
    .owner   = THIS_MODULE,
    .open    = ctrl_open,
    .release = ctrl_release,
    .read    = ctrl_read,
    .write   = ctrl_write,
};

/* ------------------------------------------------------------------ */
/* Module init / exit                                                 */
/* ------------------------------------------------------------------ */

static int __init subucom_virt_init(void)
{
    dev_t devno;
    int ret;

    ret = alloc_chrdev_region(&devno, MINOR_SPI, NDEVS, DRV_NAME);
    if (ret < 0) {
        pr_err("%s: alloc_chrdev_region failed: %d\n", DRV_NAME, ret);
        return ret;
    }
    subucom_major = MAJOR(devno);

#if LINUX_VERSION_CODE >= KERNEL_VERSION(6, 4, 0)
    subucom_class = class_create(CLASS_NAME);
#else
    subucom_class = class_create(THIS_MODULE, CLASS_NAME);
#endif
    if (IS_ERR(subucom_class)) {
        ret = PTR_ERR(subucom_class);
        pr_err("%s: class_create failed: %d\n", DRV_NAME, ret);
        goto err_unreg;
    }

    model_is_x = model && strcmp(model, "cdj3kx") == 0;
    model_is_1500x = model && strcmp(model, "cdj1500x") == 0;

    /* The sub-CPU device. */
    cdev_init(&subucom_cdev[0], &spi_fops);
    subucom_cdev[0].owner = THIS_MODULE;
    ret = cdev_add(&subucom_cdev[0], MKDEV(subucom_major, MINOR_SPI), 1);
    if (ret) goto err_class;

    subucom_dev[0] = device_create(subucom_class, NULL,
                                   MKDEV(subucom_major, MINOR_SPI),
                                   NULL, spi_dev_name());
    if (IS_ERR(subucom_dev[0])) { ret = PTR_ERR(subucom_dev[0]); goto err_cdev0; }

    /* The ctrl device. */
    cdev_init(&subucom_cdev[1], &ctrl_fops);
    subucom_cdev[1].owner = THIS_MODULE;
    ret = cdev_add(&subucom_cdev[1], MKDEV(subucom_major, MINOR_CTRL), 1);
    if (ret) goto err_dev0;

    subucom_dev[1] = device_create(subucom_class, NULL,
                                   MKDEV(subucom_major, MINOR_CTRL),
                                   NULL, CTRL_DEV_NAME);
    if (IS_ERR(subucom_dev[1])) { ret = PTR_ERR(subucom_dev[1]); goto err_cdev1; }

    init_waitqueue_head(&mosi_wq);

    if (model_is_1500x) {
        memset(miso_idle_model, 0, MISO_SIZE);
        miso_idle_model[0] = 0x01;
    } else if (model_is_x) {
        /* Format version 1 (u16 LE) and 2 pad bytes, then the CDJ-3000 frame at offset 4. */
        memset(miso_idle_model, 0, MISO_SIZE);
        miso_idle_model[0] = 0x01;
        memcpy(miso_idle_model + 4, miso_idle, MISO_SIZE - 2 - 4);
        miso_idle_model[16] &= ~0x01;
    } else {
        memcpy(miso_idle_model, miso_idle, MISO_SIZE);
    }

    /* Hold the model's service-mode combo during boot. */
    if (inject_testmode) {
        unsigned long flags;
        if (model_is_1500x) {
            /* pre-setting.sh enters ServiceMode on SHIFT and HOT CUE B. */
            testmode_byte[0] = 17; testmode_mask[0] = 0x01;
            testmode_byte[1] = 18; testmode_mask[1] = 0x40;
        } else if (model_is_x) {
            /* TEMPO RANGE: byte 10, mask 0x04 */
            testmode_byte[0] = 10; testmode_mask[0] = 0x04;
            /* MEMORY: byte 12, mask 0x01 */
            testmode_byte[1] = 12; testmode_mask[1] = 0x01;
        } else {
            /* BTN_TEMPO_RANGE: byte 6, mask 0x04 */
            testmode_byte[0] = 6; testmode_mask[0] = 0x04;
            /* BTN_CALL_PREV: byte 8, mask 0x08 */
            testmode_byte[1] = 8; testmode_mask[1] = 0x08;
        }
        spin_lock_irqsave(&miso_lock, flags);
        testmode_held = 1;
        spin_unlock_irqrestore(&miso_lock, flags);
        schedule_delayed_work(&testmode_clear_work,
                              msecs_to_jiffies(TESTMODE_INJECT_MS));
        pr_info("%s: holding the testmode combo for %d ms\n",
                DRV_NAME, TESTMODE_INJECT_MS);
    }

    pr_info("%s: /dev/%s (major %d minor %d) and /dev/%s (minor %d) are ready\n",
            DRV_NAME, spi_dev_name(), subucom_major, MINOR_SPI,
            CTRL_DEV_NAME, MINOR_CTRL);
    return 0;

err_cdev1: cdev_del(&subucom_cdev[1]);
err_dev0:  device_destroy(subucom_class, MKDEV(subucom_major, MINOR_SPI));
err_cdev0: cdev_del(&subucom_cdev[0]);
err_class: class_destroy(subucom_class);
err_unreg: unregister_chrdev_region(devno, NDEVS);
    return ret;
}

static void __exit subucom_virt_exit(void)
{
    cancel_delayed_work_sync(&testmode_clear_work);
    device_destroy(subucom_class, MKDEV(subucom_major, MINOR_CTRL));
    device_destroy(subucom_class, MKDEV(subucom_major, MINOR_SPI));
    cdev_del(&subucom_cdev[1]);
    cdev_del(&subucom_cdev[0]);
    class_destroy(subucom_class);
    unregister_chrdev_region(MKDEV(subucom_major, MINOR_SPI), NDEVS);
    pr_info("%s: unloaded\n", DRV_NAME);
}

module_init(subucom_virt_init);
module_exit(subucom_virt_exit);

MODULE_LICENSE("GPL");
MODULE_AUTHOR("cdj3k-emu");
MODULE_DESCRIPTION("Virtual subucom_spi device for cdj3k-emu");
MODULE_ALIAS("subucom_virt");
