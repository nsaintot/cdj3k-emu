// SPDX-License-Identifier: GPL-2.0
/*
 * udev_usb1 - Pioneer's media event channels.
 *
 * The deck reads media events from proc files:
 *   /proc/udev_usb1  'mount <mnt> <fs> protect:0'
 *   /proc/udev_sd1   the same for the SD card
 *   /proc/udev_nfs   'mount <mnt> nfs', written by FuseFilsine
 *
 * Node names vary per model; this module creates the nodes of every model.
 *
 * Each node queues its own messages: the deck classifies media by the node an
 * event arrives on. FuseFilsine writes one 'mount' per exported drive, faster
 * than the deck polls, so a node keeps every message until it is read.
 *
 * The deck polls one node per pass (open, fcntl F_SETFL O_NONBLOCK, read,
 * close). A read() returns the oldest message, or EAGAIN on an empty node.
 */

#include <linux/module.h>
#include <linux/proc_fs.h>
#include <linux/uaccess.h>
#include <linux/wait.h>
#include <linux/mutex.h>
#include <linux/poll.h>
#include <linux/sched.h>
#include <linux/moduleparam.h>
#include <linux/slab.h>
#include <linux/string.h>
#include <linux/err.h>

#define BUF_SIZE  512
#define QUEUE_LEN 16

struct udev_msg {
	char	buf[BUF_SIZE];
	size_t	len;
};

struct udev_chan {
	const char		*name;
	struct udev_msg		queue[QUEUE_LEN];
	unsigned int		head;	/* index of the oldest message */
	unsigned int		count;
	struct proc_dir_entry	*pde;
};

static struct udev_chan chans[] = {
	{ .name = "udev_usb1"    },
	{ .name = "udev_usb2"    },
	{ .name = "udev_sd1"     },
	{ .name = "udev_nfs"     },
	{ .name = "udev_usbctn1" },
	{ .name = "udev_usbctn2" },
	{ .name = "udev_sdctn1"  },
	{ .name = "udev_usbg1"   },
};

static DECLARE_WAIT_QUEUE_HEAD(udev_wq);	/* shared by every node */
static DEFINE_MUTEX(udev_lock);

static struct udev_chan *chan_of(struct file *f)
{
	return pde_data(file_inode(f));
}

static ssize_t udev_chan_read(struct file *f, char __user *buf,
			      size_t len, loff_t *off)
{
	struct udev_chan *c = chan_of(f);
	struct udev_msg *m;
	ssize_t ret;

	for (;;) {
		mutex_lock(&udev_lock);
		if (c->count) {
			m = &c->queue[c->head];
			ret = min(len, m->len);
			if (copy_to_user(buf, m->buf, ret)) {
				mutex_unlock(&udev_lock);
				return -EFAULT;
			}
			c->head = (c->head + 1) % QUEUE_LEN;
			c->count--;
			mutex_unlock(&udev_lock);
			return ret;
		}
		mutex_unlock(&udev_lock);

		if (f->f_flags & O_NONBLOCK)
			return -EAGAIN;

		if (wait_event_interruptible(udev_wq, READ_ONCE(c->count)))
			return -ERESTARTSYS;
	}
}

static ssize_t udev_chan_write(struct file *f, const char __user *buf,
			       size_t len, loff_t *off)
{
	struct udev_chan *c = chan_of(f);
	size_t n = min(len, (size_t)(BUF_SIZE - 1));
	struct udev_msg *m;
	char *msg;

	msg = memdup_user_nul(buf, n);
	if (IS_ERR(msg))
		return PTR_ERR(msg);

	mutex_lock(&udev_lock);
	if (c->count == QUEUE_LEN) {
		/*
		 * Writers do not retry, and a model leaves some nodes unread:
		 * drop the oldest message rather than fail the write.
		 */
		pr_warn_ratelimited("udev_usb1: /proc/%s is full, dropping its oldest message\n",
				    c->name);
		c->head = (c->head + 1) % QUEUE_LEN;
		c->count--;
	}
	m = &c->queue[(c->head + c->count) % QUEUE_LEN];
	memcpy(m->buf, msg, n + 1);
	m->len = n;
	c->count++;
	mutex_unlock(&udev_lock);
	kfree(msg);

	wake_up_interruptible_all(&udev_wq);
	return len;
}

static __poll_t udev_chan_poll(struct file *f, poll_table *wait)
{
	struct udev_chan *c = chan_of(f);
	__poll_t mask;

	poll_wait(f, &udev_wq, wait);
	mutex_lock(&udev_lock);
	mask = c->count ? (EPOLLIN | EPOLLRDNORM) : 0;
	mutex_unlock(&udev_lock);
	return mask;
}

static const struct proc_ops udev_chan_fops = {
	.proc_read  = udev_chan_read,
	.proc_write = udev_chan_write,
	.proc_poll  = udev_chan_poll,
};

static void udev_chans_remove(void)
{
	int i;

	for (i = 0; i < ARRAY_SIZE(chans); i++)
		if (chans[i].pde) {
			proc_remove(chans[i].pde);
			chans[i].pde = NULL;
		}
}

static int __init udev_usb1_init(void)
{
	int i;

	for (i = 0; i < ARRAY_SIZE(chans); i++) {
		chans[i].pde = proc_create_data(chans[i].name, 0666, NULL,
						&udev_chan_fops, &chans[i]);
		if (!chans[i].pde) {
			udev_chans_remove();
			return -ENOMEM;
		}
		pr_info("udev_usb1: /proc/%s is ready\n", chans[i].name);
	}
	return 0;
}

static void __exit udev_usb1_exit(void)
{
	udev_chans_remove();
}

module_init(udev_usb1_init);
module_exit(udev_usb1_exit);
MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("Pioneer media/gadget event channels (/proc/udev_*)");
