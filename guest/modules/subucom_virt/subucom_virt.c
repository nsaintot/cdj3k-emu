// SPDX-License-Identifier: GPL-2.0
/*
 * subucom_virt.c - Virtual subucom_spi character device for QEMU cdj3k-emu
 *
 * Creates /dev/subucom_spi1.0 (major 246, minor 0) under class "subucom_spiclass",
 *
 * Also creates /dev/subucom_ctrl for host communication:
 *   write(ctrl, 64 bytes) → inject MISO frame delivered on next read() of spi1.0
 *   read(ctrl, 64 bytes)  → consume latest LED frame written by EP122
 *
 * Ioctl interface (/sys/module/subucom_spi/parameters/):
 *   0x80027003  TIMER_STATUS_READ   → returns u16 DEFtmstat (default 0)
 *   0x40027003  TIMER_STATUS_WRITE  → sets timer_status (starts transfer timer)
 *   0x80017001  BITS_PER_WORD_READ  → returns u8 bits_per_word (default 8)
 *   0x40017001  BITS_PER_WORD_WRITE → sets bits_per_word
 *   0x40107000  MOSI_TRANSFER       → EP122 sends 16-byte MOSI frame
 *   0x80047004  RX_BYTES_READ       → returns u32 rx frame size (64)
 *   0x40047004  RX_BYTES_WRITE      → sets rx frame size
 *   0x80047002  INTERVAL_READ       → returns u32 polling interval in µs (1176)
 *   0x40047002  INTERVAL_WRITE      → sets polling interval
 *
 * read() on spi1.0: blocks until MISO frame available (~850 Hz via hrtimer),
 * returns 64 bytes. If host injected a frame via ctrl, that frame is used;
 * otherwise the idle frame is returned.
 *
 * Specs:
 *   SPI: ff1d0000.spi, bus1 CS0, 3.2 MHz, Mode 3 (CPOL=1, CPHA=1)
 *   Device class: subucom_spiclass
 *   Major: 246, Minor: 0
 *   Module params: DEFtmstat=0, txmulti=1, txpart=1, SZbufsize=1036
 */

#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/fs.h>
#include <linux/cdev.h>
#include <linux/device.h>
#include <linux/uaccess.h>
#include <linux/slab.h>
#include <linux/wait.h>
#include <linux/spinlock.h>
#include <linux/delay.h>   /* usleep_range */
#include <linux/sched.h>   /* schedule_timeout_interruptible */
#include <linux/poll.h>    /* poll_table, POLLIN, POLLRDNORM */
#include <linux/workqueue.h> /* delayed_work */
#include <linux/version.h>

#define DRV_NAME        "subucom_virt"
#define SPI_DEV_NAME    "subucom_spi1.0"
#define CTRL_DEV_NAME   "subucom_ctrl"
#define CLASS_NAME      "subucom_virt_class"

/* Dynamic major - alloc_chrdev_region picks a free slot at runtime */
static int subucom_major;
#define MINOR_SPI       0
#define MINOR_CTRL      1
#define NDEVS           2

/* MISO frame size and polling interval */
#define MISO_SIZE       64
#define MOSI_CMD_SIZE   16  /* ioctl arg: {uint32 magic, uint32 size, uint64 data_ptr} */
#define LED_FRAME_SIZE  64  /* actual LED payload pointed to by data_ptr */
#define INTERVAL_US     1176   /* ~850 Hz */

/* Layout of the 16-byte MOSI ioctl argument */
struct mosi_cmd {
    u32 magic;    /* 0x01000000 */
    u32 size;     /* byte count of data at data_ptr (≤ LED_FRAME_SIZE) */
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

/* MISO initial frame. */
static const u8 miso_idle[MISO_SIZE] = {
    /* b00-b04: header */
    0x00, 0x00, 0x01, 0x04, 0x03,
    /* b05-b11: button bitmasks */
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    /* b12-b15: device state + rotary encoder */
    0x81, 0x00, 0xff, 0xff,
    /* b16-b19: LCD touch */
    0x00, 0x00, 0x00, 0x00,
    /* b20-b21: unknown */
    0x00, 0x7f,
    /* b22-b25: tempo slider + vinyl */
    0x50, 0x7f, 0x00, 0x00,
    /* b26-b31: jog wheel (position=0xffff, velocity=0xffff, touch=0x00) */
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
 * Applied over the first 62 bytes of the MISO frame; result stored LE in b62-b63. */
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

/* MISO state: spi_read() sleeps interval_us then delivers the frame.
 * No external hrtimer needed - usleep_range() throttles in the caller's
 * context, avoiding the hrtimer→wake_up→wait_event race on QEMU/HVF. */
static DEFINE_SPINLOCK(miso_lock);
static u8 inject_pending[MISO_SIZE];
static int has_inject;   /* 1 = inject_pending is valid */

/* MOSI: EP122 writes LED frames via MOSI_TRANSFER ioctl.
 * mosi_frame holds the last 64-byte LED payload (dereferenced from the ioctl cmd). */
static DEFINE_SPINLOCK(mosi_lock);
static u8 mosi_frame[LED_FRAME_SIZE];
static int mosi_fresh;   /* 1 = unread LED frame available */
static wait_queue_head_t mosi_wq;

/* Ioctl-configurable parameters */
static u16 timer_status;
static u8  bits_per_word = 8;
static u32 rx_bytes      = MISO_SIZE;
static u32 interval_us   = INTERVAL_US;

/* ------------------------------------------------------------------ */
/* Testmode boot injection                                            */
/*                                                                    */
/* inject_testmode=1 pre-presses BTN_CALL_PREV + BTN_TEMPO_RANGE for  */
/* ~5 seconds so subucom_read sees the service-mode combo during boot.*/
/* A delayed_work automatically clears the injection before EP122     */
/* starts its 850 Hz polling loop.                                    */
/* ------------------------------------------------------------------ */

static int inject_testmode;
module_param(inject_testmode, int, 0444);
MODULE_PARM_DESC(inject_testmode, "Pre-inject testmode buttons for ~5s on load (0=off, 1=on)");

#define TESTMODE_INJECT_MS  5000

static void testmode_clear_work_fn(struct work_struct *work);
static DECLARE_DELAYED_WORK(testmode_clear_work, testmode_clear_work_fn);

static void testmode_clear_work_fn(struct work_struct *work)
{
    unsigned long flags;
    spin_lock_irqsave(&miso_lock, flags);
    has_inject = 0;
    spin_unlock_irqrestore(&miso_lock, flags);
    pr_info("%s: testmode inject cleared after %d ms\n", DRV_NAME, TESTMODE_INJECT_MS);
}

/* uaccess helpers. cdj3k-emu only targets vanilla 6.6 - the Pioneer 4.4
 * branch that called __arch_copy_{to,from}_user directly is no longer needed. */
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
    return 0;
}

static ssize_t spi_read(struct file *filp, char __user *buf,
                         size_t count, loff_t *ppos)
{
    unsigned long flags;
    u8 frame[MISO_SIZE];
    size_t n;

    /* Throttle at ~850 Hz in the caller's context.
     * schedule_timeout_interruptible uses TASK_INTERRUPTIBLE - no get_current()
     * assumptions, and correctly returns early if a signal is pending. */
    if (schedule_timeout_interruptible(usecs_to_jiffies(interval_us) + 1))
        return -ERESTARTSYS;

    /* Build the MISO frame: injected or idle, then stamp CRC-16/X-25 */
    spin_lock_irqsave(&miso_lock, flags);
    if (has_inject) {
        memcpy(frame, inject_pending, MISO_SIZE);
        /* has_inject stays 1 - injected frame is sticky until ctrl_write()
         * replaces it with a new frame (e.g. idle on button release). */
    } else {
        memcpy(frame, miso_idle, MISO_SIZE);
    }
    spin_unlock_irqrestore(&miso_lock, flags);

    {
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
        /* EP122 sends a 16-byte cmd: {magic=0x01000000, size, data_ptr}.
         * data_ptr is a userspace VA pointing to the actual LED frame.
         * We dereference it here to capture the real 64-byte payload. */
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

/* spi_read() always delivers a frame after a brief sleep (~1.2 ms).
 * From epoll's perspective this fd is always readable - EP122 calls
 * read() itself to throttle at 850 Hz.  Returning POLLIN unconditionally
 * lets epoll_ctl() succeed so EP122's event loop can watch this fd. */
static unsigned int spi_poll(struct file *filp, poll_table *wait)
{
    return POLLIN | POLLRDNORM;
}

static const struct file_operations spi_fops = {
    .owner          = THIS_MODULE,
    .open           = spi_open,
    .release        = spi_release,
    .read           = spi_read,
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

/* write(ctrl, 64 bytes) → inject MISO frame (sticky until next write)
 * write(ctrl, 0 bytes)  → clear inject, revert to idle */
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

/* read(ctrl, 64 bytes) → consume latest LED frame; blocks if none */
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

    /* spi1.0 device */
    cdev_init(&subucom_cdev[0], &spi_fops);
    subucom_cdev[0].owner = THIS_MODULE;
    ret = cdev_add(&subucom_cdev[0], MKDEV(subucom_major, MINOR_SPI), 1);
    if (ret) goto err_class;

    subucom_dev[0] = device_create(subucom_class, NULL,
                                   MKDEV(subucom_major, MINOR_SPI),
                                   NULL, SPI_DEV_NAME);
    if (IS_ERR(subucom_dev[0])) { ret = PTR_ERR(subucom_dev[0]); goto err_cdev0; }

    /* ctrl device */
    cdev_init(&subucom_cdev[1], &ctrl_fops);
    subucom_cdev[1].owner = THIS_MODULE;
    ret = cdev_add(&subucom_cdev[1], MKDEV(subucom_major, MINOR_CTRL), 1);
    if (ret) goto err_dev0;

    subucom_dev[1] = device_create(subucom_class, NULL,
                                   MKDEV(subucom_major, MINOR_CTRL),
                                   NULL, CTRL_DEV_NAME);
    if (IS_ERR(subucom_dev[1])) { ret = PTR_ERR(subucom_dev[1]); goto err_cdev1; }

    init_waitqueue_head(&mosi_wq);

    /* Testmode boot injection: pre-press BTN_CALL_PREV + BTN_TEMPO_RANGE */
    if (inject_testmode) {
        unsigned long flags;
        memcpy(inject_pending, miso_idle, MISO_SIZE);
        /* BTN_TEMPO_RANGE: byte 6, mask 0x04 */
        inject_pending[6] |= 0x04;
        /* BTN_CALL_PREV: byte 8, mask 0x08 */
        inject_pending[8] |= 0x08;
        spin_lock_irqsave(&miso_lock, flags);
        has_inject = 1;
        spin_unlock_irqrestore(&miso_lock, flags);
        schedule_delayed_work(&testmode_clear_work,
                              msecs_to_jiffies(TESTMODE_INJECT_MS));
        pr_info("%s: testmode buttons injected, auto-clear in %d ms\n",
                DRV_NAME, TESTMODE_INJECT_MS);
    }

    pr_info("%s: /dev/%s (major %d minor %d) + /dev/%s (minor %d) ready\n",
            DRV_NAME, SPI_DEV_NAME, subucom_major, MINOR_SPI,
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
MODULE_DESCRIPTION("Virtual subucom_spi device for CDJ-3000 QEMU emulation");
MODULE_ALIAS("subucom_virt");
