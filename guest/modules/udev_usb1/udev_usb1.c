// SPDX-License-Identifier: GPL-2.0
/*
 * udev_usb1 - /proc/udev_usb1 shim for vanilla 6.6 kernel.
 *
 * On Pioneer EP122 hardware the kernel exposes /proc/udev_usb1 as an IPC
 * channel between USB mount scripts and the EP122 process:
 *
 *   Writer (shell): printf 'mount %s %s protect:0' <mnt> <fs> > /proc/udev_usb1
 *   Reader (EP122): blocking read() - wakes when new data arrives
 *
 * This module recreates that interface.  Each write() stores one message;
 * the next read() returns it and clears the buffer (so subsequent reads block
 * again until a new write arrives).  poll() is supported for EP122's select loop.
 */

#include <linux/module.h>
#include <linux/proc_fs.h>
#include <linux/uaccess.h>
#include <linux/wait.h>
#include <linux/mutex.h>
#include <linux/poll.h>

#define BUF_SIZE 512

static char udev_buf[BUF_SIZE];
static size_t udev_buf_len;
static bool udev_pending;
static DECLARE_WAIT_QUEUE_HEAD(udev_wq);
static DEFINE_MUTEX(udev_mutex);

static ssize_t udev_usb1_read(struct file *f, char __user *buf,
			      size_t len, loff_t *off)
{
	ssize_t ret;

	if (wait_event_interruptible(udev_wq, udev_pending))
		return -ERESTARTSYS;

	mutex_lock(&udev_mutex);
	ret = min(len, udev_buf_len);
	if (copy_to_user(buf, udev_buf, ret)) {
		mutex_unlock(&udev_mutex);
		return -EFAULT;
	}
	udev_pending  = false;
	udev_buf_len  = 0;
	mutex_unlock(&udev_mutex);
	return ret;
}

static ssize_t udev_usb1_write(struct file *f, const char __user *buf,
			       size_t len, loff_t *off)
{
	size_t n = min(len, (size_t)(BUF_SIZE - 1));

	mutex_lock(&udev_mutex);
	if (copy_from_user(udev_buf, buf, n)) {
		mutex_unlock(&udev_mutex);
		return -EFAULT;
	}
	udev_buf[n]  = '\0';
	udev_buf_len = n;
	udev_pending = true;
	mutex_unlock(&udev_mutex);

	wake_up_interruptible(&udev_wq);
	return len;
}

static __poll_t udev_usb1_poll(struct file *f, poll_table *wait)
{
	poll_wait(f, &udev_wq, wait);
	return udev_pending ? (EPOLLIN | EPOLLRDNORM) : 0;
}

static const struct proc_ops udev_usb1_fops = {
	.proc_read  = udev_usb1_read,
	.proc_write = udev_usb1_write,
	.proc_poll  = udev_usb1_poll,
};

static struct proc_dir_entry *proc_entry;

static int __init udev_usb1_init(void)
{
	proc_entry = proc_create("udev_usb1", 0666, NULL, &udev_usb1_fops);
	if (!proc_entry)
		return -ENOMEM;
	pr_info("udev_usb1: /proc/udev_usb1 ready\n");
	return 0;
}

static void __exit udev_usb1_exit(void)
{
	proc_remove(proc_entry);
}

module_init(udev_usb1_init);
module_exit(udev_usb1_exit);
MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("EP122 USB event channel (/proc/udev_usb1)");
