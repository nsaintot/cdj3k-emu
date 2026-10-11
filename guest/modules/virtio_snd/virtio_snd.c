// SPDX-License-Identifier: GPL-2.0
/*
 * virtio_snd.c - a minimal virtio-sound PCM playback driver for Linux 6.6 on
 * AArch64.
 *
 * ALSA timing: the hrtimer fires once per ALSA period. Its tasklet advances
 * hw_ptr and calls snd_pcm_period_elapsed, which wakes the EP122 JUCE thread.
 *
 * Audio forwarding: the tasklet copies each period from the ring buffer, at
 * the hw_ptr position before the advance, into the current TX buffer, and
 * submits the buffer to QEMU once it holds qemu_factor periods (about 94 Hz
 * at 96 kHz). hw_ptr advances only for copied periods. When the TX pool is
 * empty, hw_ptr stalls, so EP122 sees back-pressure; only the Phase 2
 * fallback below advances it without copying.
 *
 * Buffer sizing: periods_min=2 lets EP122 negotiate its preferred buffer.
 * One tasklet run processes at most buffer_size/period_size - 1 periods, so
 * hw_ptr never laps appl_ptr.
 */

#include <linux/module.h>
#include <linux/virtio.h>
#include <linux/virtio_ids.h>
#include <linux/virtio_config.h>
#include <linux/vmalloc.h>
#include <linux/slab.h>
#include <linux/interrupt.h>
#include <linux/workqueue.h>
#include <linux/scatterlist.h>
#include <linux/hrtimer.h>
#include <linux/ktime.h>
#include <asm/div64.h>
#include <sound/core.h>
#include <sound/pcm.h>
#include <sound/pcm_params.h>
#include <sound/initval.h>

/* Virtqueue indices */
#define VSND_VQ_CTL             0
#define VSND_VQ_EVT             1
#define VSND_VQ_TX              2
#define VSND_VQ_RX              3
#define VSND_VQ_MAX             4

/* Control command codes */
#define VIRTIO_SND_R_PCM_SET_PARAMS     0x0101
#define VIRTIO_SND_R_PCM_PREPARE        0x0102
#define VIRTIO_SND_R_PCM_RELEASE        0x0103
#define VIRTIO_SND_R_PCM_START          0x0104
#define VIRTIO_SND_R_PCM_STOP           0x0105
#define VIRTIO_SND_S_OK                 0x8000

/* PCM format codes from enum virtio_snd_pcm_fmt in virtio_snd.h. */
#define VSND_FMT_S16    5   /* VIRTIO_SND_PCM_FMT_S16  (0=IMA_ADPCM,1=MU_LAW,2=A_LAW,3=S8,4=U8,5=S16) */
#define VSND_FMT_S32    17  /* VIRTIO_SND_PCM_FMT_S32  (...,17=S32) */

/* PCM rate codes from enum virtio_snd_pcm_rate in virtio_snd.h. */
#define VSND_RATE_44100 6   /* VIRTIO_SND_PCM_RATE_44100 (0=5512,1=8000,...,5=32000,6=44100) */
#define VSND_RATE_48000 7   /* VIRTIO_SND_PCM_RATE_48000 */
#define VSND_RATE_96000 10  /* VIRTIO_SND_PCM_RATE_96000 (8=64000,9=88200,10=96000) */

/* The cap on the reported latency. Above it the pipeline holds a backlog
 * after a preemption, and the raw value would start a feedback loop in the
 * slave-mode shim: a larger shift makes the audio play sooner, which puts
 * more buffers in flight, which grows the shift further. Capping the value
 * breaks the loop. */
#define AUDIO_LATENCY_MS_MAX 200u

/*
 * VSND_QEMU_FACTOR is the number of ALSA periods in one QEMU TX buffer at the
 * default 64-frame period. That gives 1024 frames (10.67 ms at 96 kHz), which
 * matches the time the host takes to return a buffer, so the pool does not
 * run dry between submissions.
 */
#define VSND_QEMU_FACTOR    16

/* One QEMU TX buffer holds this many frames whatever the ALSA period. The
 * per-stream factor (qemu_factor) is VSND_QEMU_FRAMES / period_size, so a
 * larger period means fewer periods per buffer and the same buffer size.
 * Every threshold below that counts periods scales by the same factor and
 * keeps its duration. At the default 64-frame period the factor is
 * VSND_QEMU_FACTOR. */
#define VSND_QEMU_FRAMES    (64 * VSND_QEMU_FACTOR)

/* The ALSA period in S32 stereo frames. The default of 64 matches the EP122
 * JUCE callback cadence. A host that cannot wake the guest that often, such
 * as TCG, passes a larger value on the kernel command line
 * (cdj3k.snd_period=N, read by insmod-virtio-snd). The value must be a power
 * of two from 64 to VSND_QEMU_FRAMES. */
static unsigned int period_frames = 64;

/* The pool of QEMU TX buffers. Each holds VSND_QEMU_FRAMES frames (10.67 ms
 * at 96 kHz).
 *
 * QEMU returns a TX buffer only after the host audio callback has consumed
 * its frames, so each buffer stays in flight for about 50-100 ms (the virtio
 * queue and the bypass ring, depending on warm-up). That
 * keeps 5-10 buffers in flight in steady state. A pool of 32 leaves margin
 * for macOS scheduler preemption: the QEMU main loop can pause for up to
 * about 25 ms, and during that time the guest keeps submitting on its own
 * clock while the drain timer cannot run. A pool that does not cover the
 * round trip causes pool-empty Phase 1 cycles, which show as latency jitter
 * on Pro DJ Link. */
#define VSND_TX_BUFS        32

/* A periodic stats line in the TX path, off by default. Turn it on at run time
 * with `echo 1 > /sys/module/virtio_snd/parameters/verbose_stats`. It is
 * declared here so vsnd_tx_submit can read it; its module_param() is at the
 * end of the file with the others. */
static unsigned int verbose_stats;

/* ------------------------------------------------------------------ */
/* Wire structures                                                      */
/* ------------------------------------------------------------------ */

struct virtio_snd_config {
	__le32 jacks;
	__le32 streams;
	__le32 chmaps;
	/* QEMU's struct virtio_snd_config has this fourth field
	 * (VIRTIO_SND_F_CTLS, spec v1.2). It keeps pipeline_extra_frames at
	 * offset 16; without it the driver would read the controls value (0)
	 * at offset 12. */
	__le32 controls;
	/* An extension that the project's QEMU patches add to the config space
	 * (4 more bytes). It holds the depth in frames of the host pipeline
	 * that frames_in_flight does not see: the host backend's queue and the
	 * device's fixed latency. It is not in the virtio spec and reads as 0
	 * on an unpatched QEMU. */
	__le32 pipeline_extra_frames;
} __packed;

struct virtio_snd_pcm_hdr {
	__le32 code;
	__le32 stream_id;
} __packed;

struct virtio_snd_pcm_set_params {
	__le32 code;
	__le32 stream_id;
	__le32 buffer_bytes;
	__le32 period_bytes;
	__le32 features;
	__u8   channels;
	__u8   format;
	__u8   rate;
	__u8   padding;
} __packed;

struct virtio_snd_result  { __le32 code; } __packed;
struct virtio_snd_pcm_xfer { __le32 stream_id; } __packed;
struct virtio_snd_pcm_status { __le32 status; __le32 latency_bytes; } __packed;

/* ------------------------------------------------------------------ */
/* Driver state                                                       */
/* ------------------------------------------------------------------ */

struct vsnd_tx_buf {
	struct virtio_snd_pcm_xfer   xfer;
	void                        *data;     /* qemu_period_bytes of PCM */
	struct virtio_snd_pcm_status status;
	struct scatterlist           sg[3];
	bool                         in_flight; /* submitted to QEMU and not yet returned */
};

struct vsnd_dev;

/* Set in vsnd_probe and cleared in vsnd_remove. The audio_latency_ms getter
 * uses it to read the running stream's frames_in_flight and rate. */
static struct vsnd_dev *g_vsnd_dev;

struct vsnd_pcm {
	struct vsnd_dev          *vsnd;
	struct snd_pcm_substream *substream;

	/* The circular pool of free TX buffers. The hardirq handler and the
	 * tasklet both access it. */
	struct vsnd_tx_buf  bufs[VSND_TX_BUFS];
	struct vsnd_tx_buf *pool[VSND_TX_BUFS + 1];
	int                 pool_head;
	int                 pool_tail;

	/* qemu_factor ALSA periods are accumulated into one QEMU TX buffer. */
	struct vsnd_tx_buf *accum_buf;   /* TX buf currently being filled */
	unsigned int        accum_count; /* ALSA periods written into accum_buf */

	/* ALSA ring-buffer geometry */
	snd_pcm_uframes_t   hw_ptr;
	snd_pcm_uframes_t   period_size;
	snd_pcm_uframes_t   buffer_size;
	unsigned int        rate;             /* current PCM rate (Hz), set in hw_params */
	long                frames_in_flight; /* frames submitted minus frames returned: the guest pipeline depth */
	u32                 host_extra_frames;/* pipeline_extra_frames cached from the QEMU config, refreshed after each TX submission */
	unsigned int        period_bytes;     /* bytes per ALSA period */
	unsigned int        qemu_period_bytes;/* bytes per QEMU TX buffer: qemu_factor × period_bytes */
	unsigned int        qemu_factor;      /* ALSA periods per QEMU TX buffer */
	unsigned int        frame_bytes;

	/* The hrtimer drives hw_ptr and snd_pcm_period_elapsed. */
	struct hrtimer        timer;
	ktime_t               period_time;
	struct tasklet_struct tasklet;
	atomic_t              pending_periods; /* overrun count from hrtimer_forward_now */
	u32                   pool_stall_ticks; /* consecutive tasklet runs with empty pool */
	u32                   phase2_streak;   /* consecutive Phase-2 advances; reset on pool recovery */
	bool                  xrun_pending;    /* set under the lock; the tasklet reads it after releasing the lock */
	u64                   dbg_xrun_signals;/* total snd_pcm_stop_xrun calls issued */

	/* The pipeline-depth watchdog forces an xrun when frames_in_flight
	 * stays high for a sustained time. It is the only way to drain the
	 * stream_q backlog: JUCE pauses production while it recovers, which
	 * lets the queue empty. A cooldown limits the clicks under chronic
	 * stress. */
	unsigned long         watchdog_deep_start; /* jiffies when the deep state began, or 0 outside it */
	unsigned long         watchdog_last_fired; /* jiffies of last forced xrun */
	u64                   dbg_watchdog_fires;  /* total forced xruns */

	/* Diagnostics. The tasklet and the hrtimer write these without the
	 * lock; the races are acceptable for statistics. */
	ktime_t  dbg_last_tx_time;      /* ktime of last vsnd_tx_submit call */
	u64      dbg_tx_count;          /* total TX buffers submitted */
	u64      dbg_pool_drop;         /* total periods dropped due to pool-empty */
	u32      dbg_max_overrun;       /* all-time largest hrtimer overrun */
	u32      dbg_max_tx_gap_us;     /* all-time largest TX gap (µs) */
	u64      dbg_phase2_drop;       /* total Phase-2 silent hw_ptr advances (audible pops) */
	/* Per-interval counters, reset by each stats line (every 300 TX buffers). */
	u64      dbg_interval_drop;     /* pool_drops since last log */
	u64      dbg_interval_phase2;   /* Phase-2 drops since last log */
	u32      dbg_interval_overrun;  /* max overrun since last log */
	u32      dbg_interval_gap_us;   /* max TX gap since last log */

	spinlock_t          lock;
	bool                running;

	struct work_struct  start_work;
	struct work_struct  stop_work;
	struct work_struct  refresh_lag_work; /* refreshes host_extra_frames from virtio config */
};

struct vsnd_dev {
	struct virtio_device *vdev;
	struct virtqueue     *vqs[VSND_VQ_MAX];
	struct snd_card      *card;
	struct vsnd_pcm      *pcm_state;
	struct mutex          ctl_lock;
	struct completion     ctl_done;
	struct virtio_snd_result ctl_res;
};

/* ------------------------------------------------------------------ */
/* Pool helpers (the caller holds the lock)                           */
/* ------------------------------------------------------------------ */

static bool vsnd_pool_empty(struct vsnd_pcm *v)
{
	return v->pool_head == v->pool_tail;
}

static int vsnd_pool_depth(struct vsnd_pcm *v)
{
	int diff = v->pool_tail - v->pool_head;
	if (diff < 0)
		diff += VSND_TX_BUFS + 1;
	return diff;
}

static void vsnd_pool_push(struct vsnd_pcm *v, struct vsnd_tx_buf *tb)
{
	v->pool[v->pool_tail] = tb;
	v->pool_tail = (v->pool_tail + 1) % (VSND_TX_BUFS + 1);
}

static struct vsnd_tx_buf *vsnd_pool_pop(struct vsnd_pcm *v)
{
	struct vsnd_tx_buf *tb = v->pool[v->pool_head];
	v->pool_head = (v->pool_head + 1) % (VSND_TX_BUFS + 1);
	tb->in_flight = true;
	return tb;
}

/* ------------------------------------------------------------------ */
/* Control queue                                                      */
/* ------------------------------------------------------------------ */

static int vsnd_ctl_cmd(struct vsnd_dev *vsnd, void *req, size_t rlen)
{
	struct scatterlist sg_out, sg_in, *sgs[2];
	void *buf;
	int err;

	/*
	 * virtqueue_add_sgs maps req through the DMA API. Stack memory is in
	 * the kernel linear map, but some DMA paths (SWIOTLB, non-coherent
	 * mappings) need kmalloc memory, so copy req to a new heap buffer.
	 */
	buf = kmemdup(req, rlen, GFP_KERNEL);
	if (!buf)
		return -ENOMEM;

	sg_init_one(&sg_out, buf, rlen);
	sg_init_one(&sg_in,  &vsnd->ctl_res, sizeof(vsnd->ctl_res));
	sgs[0] = &sg_out; sgs[1] = &sg_in;

	mutex_lock(&vsnd->ctl_lock);
	reinit_completion(&vsnd->ctl_done);
	err = virtqueue_add_sgs(vsnd->vqs[VSND_VQ_CTL], sgs, 1, 1,
				vsnd, GFP_KERNEL);
	if (!err)
		virtqueue_kick(vsnd->vqs[VSND_VQ_CTL]);
	mutex_unlock(&vsnd->ctl_lock);
	if (err) {
		kfree(buf);
		return err;
	}
	if (!wait_for_completion_timeout(&vsnd->ctl_done, HZ * 2)) {
		kfree(buf);
		return -ETIMEDOUT;
	}
	kfree(buf);
	return (le32_to_cpu(vsnd->ctl_res.code) == VIRTIO_SND_S_OK) ? 0 : -EIO;
}

static void vsnd_ctl_done(struct virtqueue *vq)
{
	struct vsnd_dev *vsnd = vq->vdev->priv;
	unsigned int len;
	if (virtqueue_get_buf(vq, &len))
		complete(&vsnd->ctl_done);
}

static void vsnd_evt_done(struct virtqueue *vq) { }
static void vsnd_rx_done(struct virtqueue *vq)  { }

/* ------------------------------------------------------------------ */
/* TX queue: submit an accumulated buffer to QEMU                     */
/* ------------------------------------------------------------------ */

static void vsnd_tx_submit(struct vsnd_pcm *vpcm, struct vsnd_tx_buf *tb)
{
	struct scatterlist *sgs[3];

	tb->xfer.stream_id = 0;
	sg_init_one(&tb->sg[0], &tb->xfer,  sizeof(tb->xfer));
	sg_init_one(&tb->sg[1],  tb->data,  vpcm->qemu_period_bytes);
	sg_init_one(&tb->sg[2], &tb->status, sizeof(tb->status));
	sgs[0] = &tb->sg[0]; sgs[1] = &tb->sg[1]; sgs[2] = &tb->sg[2];

	virtqueue_add_sgs(vpcm->vsnd->vqs[VSND_VQ_TX], sgs, 2, 1,
			  tb, GFP_ATOMIC);
	virtqueue_kick(vpcm->vsnd->vqs[VSND_VQ_TX]);

	/* Count the buffer's frames as in flight until QEMU returns the buffer
	 * (the virtio queue and the bypass ring). vsnd_tx_done subtracts them
	 * then. */
	if (vpcm->frame_bytes)
		vpcm->frames_in_flight +=
			(long)vpcm->qemu_period_bytes / vpcm->frame_bytes;

	/* Diagnostics: track the TX cadence. */
	{
		ktime_t now = ktime_get();
		if (vpcm->dbg_tx_count > 0) {
			u32 gap_us = (u32)ktime_to_us(ktime_sub(now, vpcm->dbg_last_tx_time));
			if (gap_us > vpcm->dbg_max_tx_gap_us)
				vpcm->dbg_max_tx_gap_us = gap_us;
			if (gap_us > vpcm->dbg_interval_gap_us)
				vpcm->dbg_interval_gap_us = gap_us;
		}
		vpcm->dbg_last_tx_time = now;
		vpcm->dbg_tx_count++;
		if (verbose_stats && vpcm->dbg_tx_count % 300 == 0) {
			/* lat is the guest pipeline depth (frames_in_flight) plus the
			 * host pipeline depth (HAL and static device latency, cached
			 * from QEMU). It is the audio_latency_ms value without the cap.
			 * inflight_n counts the TX buffers in flight, as a check on
			 * frames_in_flight. wdog counts the xrun recoveries that the
			 * watchdog triggered. */
			unsigned int rate = vpcm->rate ? vpcm->rate : 1;
			long inflight = vpcm->frames_in_flight;
			unsigned int inflight_ms = inflight > 0
				? (unsigned int)(inflight * 1000 / rate) : 0;
			unsigned int host_ms =
				(unsigned int)((u64)vpcm->host_extra_frames * 1000 / rate);
			int j, in_flight_count = 0;
			for (j = 0; j < VSND_TX_BUFS; j++)
				if (vpcm->bufs[j].in_flight) in_flight_count++;
			pr_info("virtio_snd: tx=%llu stall=%llu(+%llu) pop=%llu(+%llu) "
				"xrun=%llu wdog=%llu overrun=%u(%u) gap=%u(%u)us "
				"lat=%u+%u=%ums inflight_n=%d/%d\n",
				vpcm->dbg_tx_count,
				vpcm->dbg_pool_drop,     vpcm->dbg_interval_drop,
				vpcm->dbg_phase2_drop,   vpcm->dbg_interval_phase2,
				vpcm->dbg_xrun_signals,  vpcm->dbg_watchdog_fires,
				vpcm->dbg_max_overrun,   vpcm->dbg_interval_overrun,
				vpcm->dbg_max_tx_gap_us, vpcm->dbg_interval_gap_us,
				inflight_ms, host_ms, inflight_ms + host_ms,
				in_flight_count, VSND_TX_BUFS);
			vpcm->dbg_interval_drop    = 0;
			vpcm->dbg_interval_phase2  = 0;
			vpcm->dbg_interval_overrun = 0;
			vpcm->dbg_interval_gap_us  = 0;
		}
	}
}

/*
 * Return each TX buffer that QEMU has finished with to the free pool. This
 * does not touch hw_ptr; the hrtimer owns ALSA timing.
 */
static void vsnd_tx_done(struct virtqueue *vq)
{
	struct vsnd_dev *vsnd = vq->vdev->priv;
	struct vsnd_pcm *vpcm = vsnd->pcm_state;
	struct vsnd_tx_buf *tb;
	unsigned int len;
	unsigned long flags;

	spin_lock_irqsave(&vpcm->lock, flags);
	while ((tb = virtqueue_get_buf(vq, &len)) != NULL) {
		tb->in_flight = false;
		vsnd_pool_push(vpcm, tb);
		if (vpcm->frame_bytes)
			vpcm->frames_in_flight -=
				(long)vpcm->qemu_period_bytes / vpcm->frame_bytes;
	}
	spin_unlock_irqrestore(&vpcm->lock, flags);
}

/* ------------------------------------------------------------------ */
/* hrtimer and tasklet: ALSA timing and audio accumulation           */
/* ------------------------------------------------------------------ */

static void vsnd_tasklet_fn(struct tasklet_struct *t)
{
	struct vsnd_pcm *vpcm = from_tasklet(vpcm, t, tasklet);
	struct snd_pcm_runtime *r;
	snd_pcm_uframes_t old_hw_ptr;
	unsigned long flags;
	int count, i;

	/*
	 * Consume the periods that the hrtimer accumulated, capped at
	 * buffer_size/period_size - 1 so hw_ptr never passes appl_ptr. If the
	 * TX pool empties mid-loop, put the remainder back for the next tasklet
	 * run; dropping it would lose periods and cause xruns during track
	 * loading.
	 */
	count = atomic_xchg(&vpcm->pending_periods, 0);
	if (count <= 0)
		return;

	spin_lock_irqsave(&vpcm->lock, flags);

	if (!vpcm->running || !vpcm->substream) {
		spin_unlock_irqrestore(&vpcm->lock, flags);
		atomic_add(count, &vpcm->pending_periods);
		return;
	}

	r = vpcm->substream->runtime;

	/* Never process more periods than the ring holds, minus one. The excess
	 * goes back to pending_periods before the loop, so the next hrtimer tick
	 * picks it up; discarding it would accumulate into a slow clock drift
	 * that becomes a JUCE spin after 1-2 minutes. */
	{
		int cap = (int)(vpcm->buffer_size / vpcm->period_size) - 1;
		if (count > cap) {
			atomic_add(count - cap, &vpcm->pending_periods);
			count = cap;
		}
	}

	for (i = 0; i < count; i++) {
		/* Take a free TX buffer when a new batch starts. When the pool is
		 * empty, the code below first collects returned buffers, then waits
		 * (Phase 1) or drops the period (Phase 2). */
		if (vpcm->accum_buf == NULL) {
			if (vsnd_pool_empty(vpcm)) {
				/* QEMU may have marked TX buffers as used before the
				 * interrupt fired. Collecting them here avoids a stall
				 * when the virtqueue callbacks lag slightly. */
				struct vsnd_tx_buf *rtb;
				unsigned int rlen;
				while ((rtb = virtqueue_get_buf(
						vpcm->vsnd->vqs[VSND_VQ_TX],
						&rlen)) != NULL) {
					rtb->in_flight = false;
					vsnd_pool_push(vpcm, rtb);
					if (vpcm->frame_bytes)
						vpcm->frames_in_flight -=
							(long)vpcm->qemu_period_bytes /
							vpcm->frame_bytes;
				}
			}
			if (vsnd_pool_empty(vpcm)) {
				/*
				 * The pool is empty.
				 *
				 * Phase 1 (retry, no drift): return without advancing
				 * hw_ptr for up to qemu_factor*2 consecutive tasklet
				 * runs (about two TX periods). Releasing the lock lets
				 * vsnd_tx_done refill the pool. In normal operation
				 * QEMU returns a buffer within about 5 ms, so the retry
				 * adds no clock drift.
				 *
				 * Extended Phase 1: if the short window has expired but
				 * QEMU still holds TX buffers, QEMU is consuming audio
				 * and will return them soon. Stay in Phase 1 until
				 * VSND_PHASE1_HARDCAP; entering Phase 2 here would
				 * cause pops while the stream recovers from a long
				 * EP122 thread stall.
				 *
				 * Phase 2 (drop, no deadlock): when no buffer is in
				 * flight, or the hard cap has passed, the QEMU transport
				 * is broken or too slow. Advance hw_ptr so EP122's writei
				 * does not block indefinitely: a blocked writei leaves
				 * the guest CPU idle, macOS then deprioritises the QEMU
				 * vCPU, QEMU never returns the buffers, and the stream
				 * deadlocks.
				 */
				vpcm->pool_stall_ticks++;
				vpcm->dbg_pool_drop++;
				vpcm->dbg_interval_drop++;
				if (vpcm->pool_stall_ticks < vpcm->qemu_factor * 2) {
					break; /* Phase 1: short retry. */
				}
				/* Enter Phase 2 only when no buffer is in flight, or when
				 * the stall has lasted more than VSND_PHASE1_HARDCAP
				 * tasklet runs (about 340 ms at the normal hrtimer rate),
				 * after which QEMU is no longer expected to return a
				 * buffer soon. Without the cap, a slow CoreAudio drain
				 * keeps all 32 buffers in flight, so Phase 1 never ends
				 * and hw_ptr stops. JUCE's writei then blocks while
				 * wall-clock time advances, and JUCE logs "snd over
				 * delta" and enters its recovery spin. The scan is
				 * O(VSND_TX_BUFS) and runs only on the pool-empty
				 * path. */
#define VSND_PHASE1_HARDCAP  (vpcm->qemu_factor * 32)  /* about 340 ms */
				if (vpcm->pool_stall_ticks < VSND_PHASE1_HARDCAP) {
					int j, in_flight = 0;
					for (j = 0; j < VSND_TX_BUFS; j++)
						if (vpcm->bufs[j].in_flight)
							in_flight++;
					if (in_flight > 0)
						break; /* Extended Phase 1: wait for QEMU. */
				}
				/* Phase 2: the QEMU transport is too slow or broken. Do
				 * not reset pool_stall_ticks here, so hw_ptr keeps
				 * advancing at one period per hrtimer tick until the pool
				 * refills. Resetting it would make Phase 1 climb back to
				 * the hard cap before every Phase 2 step, which gives
				 * about 3 pops per second and an effective sample rate of
				 * about 94 Hz during a sustained stall. The counter is
				 * reset once the pool depth reaches VSND_POOL_HEALTHY.
				 *
				 * After VSND_XRUN_THRESHOLD consecutive Phase 2 periods,
				 * signal an explicit ALSA xrun. JUCE's xrun recovery
				 * (snd_pcm_recover) resets appl_ptr and hw_ptr together.
				 * If JUCE detects the drift through snd-over-delta
				 * cascades instead, the EP122 HuiProcessor thread locks
				 * in a user-space busy loop. */
#define VSND_XRUN_THRESHOLD  (vpcm->qemu_factor * 4)  /* about 42 ms */
				vpcm->dbg_phase2_drop++;
				vpcm->dbg_interval_phase2++;
				vpcm->hw_ptr = (vpcm->hw_ptr + vpcm->period_size) %
				               vpcm->buffer_size;
				vpcm->phase2_streak++;
				if (vpcm->phase2_streak == VSND_XRUN_THRESHOLD &&
				    !vpcm->xrun_pending) {
					vpcm->xrun_pending = true;
					vpcm->dbg_xrun_signals++;
				}
				continue;
			}
			/*
			 * Reset pool_stall_ticks only when the pool has recovered to
			 * a healthy depth (VSND_TX_BUFS/4, about 85 ms of headroom).
			 * A reset on every pop would hide a chronic near-empty
			 * oscillation: when QEMU returns TX buffers just as fast as
			 * the guest submits them, the pool cycles between 0 and 1-2
			 * buffers at about 1000 stalls per second, and each pop would
			 * clear the counter before the Phase 1 hard cap fires. With
			 * the depth threshold, the counter accumulates, the hard cap
			 * triggers, Phase 2 advances hw_ptr at real-time pace, JUCE's
			 * writei unblocks, and the system leaves the spin in about
			 * 340 ms instead of about 5 minutes.
			 */
#define VSND_POOL_HEALTHY  (VSND_TX_BUFS / 4)
			if (vsnd_pool_depth(vpcm) >= VSND_POOL_HEALTHY) {
				vpcm->pool_stall_ticks = 0;
				vpcm->phase2_streak    = 0;  /* The pool is healthy again. */
			}
			vpcm->accum_buf = vsnd_pool_pop(vpcm);
		}

		/* EP122 has data and a TX buffer is available, so advance. */
		old_hw_ptr = vpcm->hw_ptr;
		vpcm->hw_ptr = (old_hw_ptr + vpcm->period_size) % vpcm->buffer_size;

		char *dst = (char *)vpcm->accum_buf->data +
			    vpcm->accum_count * vpcm->period_bytes;
		char *src = (char *)r->dma_area +
			    old_hw_ptr * vpcm->frame_bytes;
		memcpy(dst, src, vpcm->period_bytes);
		vpcm->accum_count++;

		if (vpcm->accum_count == vpcm->qemu_factor) {
			vsnd_tx_submit(vpcm, vpcm->accum_buf);
			vpcm->accum_buf   = NULL;
			vpcm->accum_count = 0;
			/* Refresh host_extra_frames in process context. A
			 * submission happens every 10.7 ms, often enough for the
			 * cache to follow changes in pipeline depth. */
			schedule_work(&vpcm->refresh_lag_work);
		}
	}

	{
		/* The pipeline-depth watchdog. If frames_in_flight stays above
		 * VSND_WATCHDOG_DEEP_FRAMES (about 100 ms) for
		 * VSND_WATCHDOG_DEEP_JIFFIES, and none fired within
		 * VSND_WATCHDOG_COOLDOWN, signal an xrun. JUCE prepares the
		 * stream again, and its pause in production lets the QEMU
		 * stream_q drain. The xrun causes one audible click, and the
		 * pipeline returns to its baseline depth.
		 *
		 * VSND_WATCHDOG_DEEP_FRAMES is above the steady-state maximum
		 * (about 50 ms) and well below the ceiling (about 340 ms,
		 * VSND_TX_BUFS buffers), so it triggers only when the pipeline
		 * is stuck. VSND_WATCHDOG_DEEP_JIFFIES is long enough that a
		 * single preemption, which usually drains on its own within a
		 * few seconds, does not trip it. VSND_WATCHDOG_COOLDOWN limits
		 * the clicks to one per minute, even under chronic stress. */
#define VSND_WATCHDOG_DEEP_FRAMES   (10 * 1024)        /* about 107 ms at 96 kHz */
#define VSND_WATCHDOG_DEEP_JIFFIES  (8 * HZ)
#define VSND_WATCHDOG_COOLDOWN      (60 * HZ)
		if (vpcm->frames_in_flight > VSND_WATCHDOG_DEEP_FRAMES) {
			if (vpcm->watchdog_deep_start == 0) {
				vpcm->watchdog_deep_start = jiffies;
			} else if (time_after(jiffies,
					      vpcm->watchdog_deep_start +
					      VSND_WATCHDOG_DEEP_JIFFIES) &&
				   time_after(jiffies,
					      vpcm->watchdog_last_fired +
					      VSND_WATCHDOG_COOLDOWN) &&
				   !vpcm->xrun_pending) {
				vpcm->xrun_pending = true;
				vpcm->dbg_xrun_signals++;
				vpcm->dbg_watchdog_fires++;
				vpcm->watchdog_last_fired = jiffies;
				vpcm->watchdog_deep_start = 0;
			}
		} else {
			vpcm->watchdog_deep_start = 0;
		}

		bool fire_xrun = vpcm->xrun_pending;
		vpcm->xrun_pending = false;
		spin_unlock_irqrestore(&vpcm->lock, flags);

		/* Put back the periods that could not be processed because the
		 * pool emptied mid-loop. */
		if (i < count)
			atomic_add(count - i, &vpcm->pending_periods);

		/*
		 * Signal an explicit ALSA xrun when one is pending. This must run
		 * outside vpcm->lock, because snd_pcm_stop_xrun takes the
		 * substream's stream lock and may call back into the trigger op.
		 * The xrun stops the stream, so skip the period_elapsed call in the
		 * same tasklet run.
		 */
		if (fire_xrun) {
			snd_pcm_stop_xrun(vpcm->substream);
		} else if (i > 0) {
			snd_pcm_period_elapsed(vpcm->substream);
		}
	}
}

static enum hrtimer_restart vsnd_hrtimer_fn(struct hrtimer *t)
{
	struct vsnd_pcm *vpcm = container_of(t, struct vsnd_pcm, timer);
	u64 overrun;

	if (!vpcm->running)
		return HRTIMER_NORESTART;

	overrun = hrtimer_forward_now(t, vpcm->period_time);
	if ((u32)overrun > vpcm->dbg_max_overrun)
		vpcm->dbg_max_overrun = (u32)overrun;
	if ((u32)overrun > vpcm->dbg_interval_overrun)
		vpcm->dbg_interval_overrun = (u32)overrun;
	/*
	 * Clamp a catch-up burst to half the pool (16 TX buffers, about
	 * 170 ms): a single tasklet run absorbs a host stall of that length
	 * without drift and cannot drain the whole pool. Missed intervals
	 * beyond it are dropped.
	 */
#define VSND_OVERRUN_CLAMP  (vpcm->qemu_factor * VSND_TX_BUFS / 2)  /* about 170 ms */
	if (overrun > VSND_OVERRUN_CLAMP)
		overrun = VSND_OVERRUN_CLAMP;
	atomic_add((int)overrun, &vpcm->pending_periods);
	tasklet_schedule(&vpcm->tasklet);
	return HRTIMER_RESTART;
}

/* ------------------------------------------------------------------ */
/* Workqueue: START and STOP (process context, can sleep)             */
/* ------------------------------------------------------------------ */

static void vsnd_start_work(struct work_struct *w)
{
	struct vsnd_pcm *vpcm = container_of(w, struct vsnd_pcm, start_work);
	struct virtio_snd_pcm_hdr hdr = {
		.code      = cpu_to_le32(VIRTIO_SND_R_PCM_START),
		.stream_id = 0,
	};

	/*
	 * Tell QEMU to start consuming TX buffers. TRIGGER_START has already
	 * started the hrtimer, so EP122 gets period callbacks at once instead
	 * of after this blocking control command completes.
	 */
	vsnd_ctl_cmd(vpcm->vsnd, &hdr, sizeof(hdr));
}

/* Refresh host_extra_frames (HAL and static device latency) from QEMU's
 * config space. This runs in process context because virtio_cread may sleep
 * and cannot run in the tasklet or the hrtimer. The tasklet schedules it
 * after each TX submission, about every 10 ms. */
static void vsnd_refresh_lag_work(struct work_struct *w)
{
	struct vsnd_pcm *vpcm = container_of(w, struct vsnd_pcm, refresh_lag_work);
	u32 extra = 0;
	virtio_cread(vpcm->vsnd->vdev, struct virtio_snd_config,
		     pipeline_extra_frames, &extra);
	WRITE_ONCE(vpcm->host_extra_frames, extra);
}

static void vsnd_stop_work(struct work_struct *w)
{
	struct vsnd_pcm *vpcm = container_of(w, struct vsnd_pcm, stop_work);
	struct virtio_snd_pcm_hdr hdr = {
		.code      = cpu_to_le32(VIRTIO_SND_R_PCM_STOP),
		.stream_id = 0,
	};
	struct vsnd_tx_buf *tb;
	unsigned int len;
	unsigned long flags;

	/*
	 * Do not cancel the hrtimer or kill the tasklet here. During xrun
	 * recovery EP122 runs STOP, PREPARE and START in quick succession, and
	 * TRIGGER_START has already re-armed the hrtimer; cancelling it would
	 * delay the next period callback by several seconds. The hrtimer stops
	 * itself on the next fire when running is false, and TRIGGER_START
	 * restarts it.
	 */

	vsnd_ctl_cmd(vpcm->vsnd, &hdr, sizeof(hdr));

	/* Return the TX buffers that QEMU has finished with to the pool. */
	spin_lock_irqsave(&vpcm->lock, flags);
	while ((tb = virtqueue_get_buf(vpcm->vsnd->vqs[VSND_VQ_TX], &len))) {
		tb->in_flight = false;
		vsnd_pool_push(vpcm, tb);
	}
	/* Return the partly filled accumulation buffer. */
	if (vpcm->accum_buf) {
		vpcm->accum_buf->in_flight = false;
		vsnd_pool_push(vpcm, vpcm->accum_buf);
		vpcm->accum_buf   = NULL;
		vpcm->accum_count = 0;
	}
	spin_unlock_irqrestore(&vpcm->lock, flags);
}

/* ------------------------------------------------------------------ */
/* ALSA PCM ops                                                       */
/* ------------------------------------------------------------------ */

static const struct snd_pcm_hardware vsnd_hw = {
	.info          = SNDRV_PCM_INFO_MMAP      |
	                 SNDRV_PCM_INFO_INTERLEAVED |
	                 SNDRV_PCM_INFO_MMAP_VALID  |
	                 SNDRV_PCM_INFO_BLOCK_TRANSFER,
	.formats       = SNDRV_PCM_FMTBIT_S16_LE | SNDRV_PCM_FMTBIT_S32_LE,
	.rates         = SNDRV_PCM_RATE_44100 | SNDRV_PCM_RATE_48000 | SNDRV_PCM_RATE_96000,
	.rate_min      = 44100,
	.rate_max      = 96000,
	.channels_min  = 2,
	.channels_max  = 2,
	.buffer_bytes_max = 262144,
	/* The default period is 64 frames (512 bytes of S32 stereo);
	 * vsnd_pcm_open replaces it with period_frames. 64 frames matches the
	 * EP122 JUCE callback cadence (one wake every 0.67 ms) and makes a
	 * QEMU TX buffer 8 KiB, a quarter of the 32 KiB host bypass ring,
	 * which leaves room for back-pressure. */
	.period_bytes_min = 64 * 8,
	.period_bytes_max = 64 * 8,
	.periods_min   = 2,
	.periods_max   = 128,
	.fifo_size     = 0,
};

static int vsnd_pcm_open(struct snd_pcm_substream *sub)
{
	struct vsnd_dev *vsnd = sub->pcm->private_data;
	struct vsnd_pcm *vpcm = vsnd->pcm_state;

	vpcm->substream = sub;
	sub->runtime->hw = vsnd_hw;
	sub->runtime->hw.period_bytes_min = period_frames * 8;
	sub->runtime->hw.period_bytes_max = period_frames * 8;
	snd_pcm_hw_constraint_integer(sub->runtime, SNDRV_PCM_HW_PARAM_PERIODS);
	snd_pcm_set_managed_buffer(sub, SNDRV_DMA_TYPE_VMALLOC, NULL,
				   0, vsnd_hw.buffer_bytes_max);
	return 0;
}

static int vsnd_pcm_close(struct snd_pcm_substream *sub)
{
	(void)sub;
	return 0;
}

static int vsnd_pcm_hw_params(struct snd_pcm_substream *sub,
			      struct snd_pcm_hw_params *params)
{
	struct vsnd_dev *vsnd = sub->pcm->private_data;
	struct vsnd_pcm *vpcm = vsnd->pcm_state;
	struct snd_pcm_runtime *r = sub->runtime;
	struct virtio_snd_pcm_set_params cmd = {};
	unsigned int fmt, rate, bps;
	u64 ns;
	int i, err;

	switch (params_format(params)) {
	case SNDRV_PCM_FORMAT_S16_LE: bps = 2; fmt = VSND_FMT_S16; break;
	case SNDRV_PCM_FORMAT_S32_LE: bps = 4; fmt = VSND_FMT_S32; break;
	default: return -EINVAL;
	}
	switch (params_rate(params)) {
	case 44100: rate = VSND_RATE_44100; break;
	case 48000: rate = VSND_RATE_48000; break;
	case 96000: rate = VSND_RATE_96000; break;
	default:    return -EINVAL;
	}

	vpcm->frame_bytes        = params_channels(params) * bps;
	vpcm->period_bytes       = params_period_bytes(params);
	vpcm->period_size        = params_period_size(params);
	vpcm->buffer_size        = params_buffer_size(params);
	vpcm->rate               = params_rate(params);

	vpcm->qemu_factor        = max_t(unsigned int, 1,
	                                 VSND_QEMU_FRAMES / vpcm->period_size);
	vpcm->qemu_period_bytes  = vpcm->period_bytes * vpcm->qemu_factor;

	ns = (u64)vpcm->period_size * 1000000000ULL;
	do_div(ns, params_rate(params));
	vpcm->period_time = ktime_set(0, (long)ns);

	/* Allocate the TX data buffers, each sized for one QEMU period. Set
	 * each pointer to NULL between kfree and kzalloc, so that if kzalloc
	 * fails partway and ALSA retries hw_params, the next pass does not
	 * free a slot twice. */
	for (i = 0; i < VSND_TX_BUFS; i++) {
		kfree(vpcm->bufs[i].data);
		vpcm->bufs[i].data = NULL;
	}
	for (i = 0; i < VSND_TX_BUFS; i++) {
		vpcm->bufs[i].data = kzalloc(vpcm->qemu_period_bytes, GFP_KERNEL);
		if (!vpcm->bufs[i].data)
			return -ENOMEM;
	}

	cmd.code         = cpu_to_le32(VIRTIO_SND_R_PCM_SET_PARAMS);
	cmd.stream_id    = 0;
	cmd.buffer_bytes = cpu_to_le32(vpcm->qemu_period_bytes * VSND_TX_BUFS);
	cmd.period_bytes = cpu_to_le32(vpcm->qemu_period_bytes);
	cmd.channels     = params_channels(params);
	cmd.format       = fmt;
	cmd.rate         = rate;
	err = vsnd_ctl_cmd(vsnd, &cmd, sizeof(cmd));
	if (err) {
		pr_err("virtio_snd: SET_PARAMS failed: %d (ctl_res=0x%x)\n",
		       err, le32_to_cpu(vsnd->ctl_res.code));
		return err;
	}

	{
		struct virtio_snd_pcm_hdr prep = {
			.code      = cpu_to_le32(VIRTIO_SND_R_PCM_PREPARE),
			.stream_id = 0,
		};
		return vsnd_ctl_cmd(vsnd, &prep, sizeof(prep));
	}
}

static int vsnd_pcm_hw_free(struct snd_pcm_substream *sub)
{
	struct vsnd_dev *vsnd = sub->pcm->private_data;
	struct vsnd_pcm *vpcm = vsnd->pcm_state;
	struct snd_pcm_runtime *r = sub->runtime;
	struct virtio_snd_pcm_hdr rel = {
		.code = cpu_to_le32(VIRTIO_SND_R_PCM_RELEASE),
		.stream_id = 0,
	};

	cancel_work_sync(&vpcm->start_work);
	cancel_work_sync(&vpcm->stop_work);
	vsnd_ctl_cmd(vsnd, &rel, sizeof(rel));
	return 0;
}

static int vsnd_pcm_prepare(struct snd_pcm_substream *sub)
{
	struct vsnd_dev *vsnd = sub->pcm->private_data;
	struct vsnd_pcm *vpcm = vsnd->pcm_state;
	unsigned long flags;
	int i;

	/*
	 * Reset pending_periods first; it is atomic, so this needs no lock. A
	 * stale backlog from a previous session would otherwise cause a burst
	 * of snd_pcm_period_elapsed calls after the next START.
	 */
	atomic_set(&vpcm->pending_periods, 0);

	/*
	 * After an xrun, natural or forced by the watchdog, JUCE calls
	 * snd_pcm_prepare. If the pipeline is still deep, force a flush in
	 * QEMU. Otherwise the QEMU stream->queue and the bypass pending list
	 * keep the audio backlog from before the xrun: JUCE submits again,
	 * frames_in_flight stays deep, and the audible pipeline never recovers.
	 *
	 * Sending VIRTIO_SND_R_PCM_RELEASE runs virtio_snd_pcm_flush in QEMU,
	 * which empties stream->queue and calls vsnd_bypass_pending_flush to
	 * release the deferred-return list. The bypass voice and ring stay
	 * active, so there is no AUD_close_out, no PREPARE and no 2 s
	 * reinitialisation. JUCE's next snd_pcm_start, after recovery, sends a
	 * fresh VIRTIO_SND_R_PCM_START that resumes playback. The queues clear
	 * in under 1 ms, the ring plays out what it still holds, and JUCE
	 * writes fresh audio at the recovered rate.
	 *
	 * The flush runs only when there is a real backlog, so the first track
	 * load and harmless re-prepares do not trigger it. The threshold
	 * matches the watchdog's deep state (10 buffers, about 100 ms). */
	{
		int j, in_flight_count = 0;
		for (j = 0; j < VSND_TX_BUFS; j++)
			if (vpcm->bufs[j].in_flight)
				in_flight_count++;

		if (in_flight_count >= 10) {
			struct virtio_snd_pcm_hdr rel = {
				.code = cpu_to_le32(VIRTIO_SND_R_PCM_RELEASE),
				.stream_id = 0,
			};
			pr_info("virtio_snd: prepare is flushing the QEMU backlog "
				"(%d buffers in flight)\n", in_flight_count);
			vsnd_ctl_cmd(vsnd, &rel, sizeof(rel));
		}
	}

	spin_lock_irqsave(&vpcm->lock, flags);
	vpcm->hw_ptr           = 0;
	vpcm->accum_count      = 0;
	vpcm->pool_stall_ticks = 0;
	vpcm->phase2_streak    = 0;
	vpcm->xrun_pending     = false;
	vpcm->watchdog_deep_start = 0;
	/* Keep watchdog_last_fired: the cooldown persists across re-prepares,
	 * so a single bad event does not start a click loop. */

	/*
	 * Reseed frames_in_flight from the number of buffers still in flight.
	 * After an xrun JUCE calls PREPARE while QEMU still owns several TX
	 * buffers, and vsnd_tx_done subtracts each one as it returns. A counter
	 * reset to zero here would go negative, audio_latency_ms_get would
	 * report guest_ms as 0, and the slave-sync auto offset would stay wrong
	 * until reboot.
	 */
	{
		int j, n = 0;
		for (j = 0; j < VSND_TX_BUFS; j++)
			if (vpcm->bufs[j].in_flight)
				n++;
		vpcm->frames_in_flight = vpcm->frame_bytes
			? (long)n * (long)vpcm->qemu_period_bytes /
			  (long)vpcm->frame_bytes
			: 0;
	}

	/*
	 * Release the partly filled accumulation buffer. It was never submitted
	 * to QEMU, so it is not in flight, and the pool rebuild below takes it.
	 */
	if (vpcm->accum_buf) {
		vpcm->accum_buf->in_flight = false;
		vpcm->accum_buf = NULL;
	}

	/*
	 * Rebuild the free pool from the buffers that QEMU has already returned
	 * (in_flight is false). vsnd_tx_done pushes the buffers still in QEMU's
	 * virtqueue when QEMU returns them. Pushing them here as well would
	 * allocate them twice, corrupt the virtio queue and freeze EP122 after
	 * snd-over recovery.
	 */
	vpcm->pool_head = 0;
	vpcm->pool_tail = 0;
	for (i = 0; i < VSND_TX_BUFS; i++) {
		if (!vpcm->bufs[i].in_flight)
			vsnd_pool_push(vpcm, &vpcm->bufs[i]);
	}
	spin_unlock_irqrestore(&vpcm->lock, flags);
	return 0;
}

static int vsnd_pcm_trigger(struct snd_pcm_substream *sub, int cmd)
{
	struct vsnd_dev *vsnd = sub->pcm->private_data;
	struct vsnd_pcm *vpcm = vsnd->pcm_state;

	switch (cmd) {
	case SNDRV_PCM_TRIGGER_START:
	case SNDRV_PCM_TRIGGER_RESUME:
		vpcm->running = true;
		/*
		 * Start the hrtimer here, in atomic context. start_work calls
		 * vsnd_ctl_cmd, which waits for QEMU's reply and can take seconds
		 * under load; if the hrtimer started there, EP122 would get no
		 * snd_pcm_period_elapsed callbacks during the wait, log snd-over
		 * deltas of 1-2 s, run out of recovery retries and freeze.
		 */
		hrtimer_start(&vpcm->timer, vpcm->period_time, HRTIMER_MODE_REL);
		schedule_work(&vpcm->start_work);
		return 0;
	case SNDRV_PCM_TRIGGER_STOP:
	case SNDRV_PCM_TRIGGER_SUSPEND:
		vpcm->running = false;
		schedule_work(&vpcm->stop_work);
		return 0;
	}
	return -EINVAL;
}

static snd_pcm_uframes_t vsnd_pcm_pointer(struct snd_pcm_substream *sub)
{
	struct vsnd_dev *vsnd = sub->pcm->private_data;
	struct vsnd_pcm *vpcm = vsnd->pcm_state;
	return READ_ONCE(vpcm->hw_ptr);
}

static const struct snd_pcm_ops vsnd_pcm_ops = {
	.open      = vsnd_pcm_open,
	.close     = vsnd_pcm_close,
	.hw_params = vsnd_pcm_hw_params,
	.hw_free   = vsnd_pcm_hw_free,
	.prepare   = vsnd_pcm_prepare,
	.trigger   = vsnd_pcm_trigger,
	.pointer   = vsnd_pcm_pointer,
};

/* ------------------------------------------------------------------ */
/* Virtio driver probe / remove                                       */
/* ------------------------------------------------------------------ */

static int vsnd_probe(struct virtio_device *vdev)
{
	vq_callback_t  *cbs[VSND_VQ_MAX]         = { vsnd_ctl_done, vsnd_evt_done,
						       vsnd_tx_done,  vsnd_rx_done };
	static const char * const names[VSND_VQ_MAX] = { "control", "event", "tx", "rx" };
	struct vsnd_dev *vsnd;
	struct vsnd_pcm *vpcm;
	struct snd_card *card;
	struct snd_pcm  *pcm;
	int err;

	if (period_frames < 64 || period_frames > VSND_QEMU_FRAMES ||
	    (period_frames & (period_frames - 1))) {
		pr_warn("virtio_snd: period_frames=%u is out of range; using 64.\n",
			period_frames);
		period_frames = 64;
	}

	err = snd_card_new(&vdev->dev, -1, "vsnd", THIS_MODULE, 0, &card);
	if (err < 0)
		return err;

	vsnd = kzalloc(sizeof(*vsnd), GFP_KERNEL);
	if (!vsnd) { err = -ENOMEM; goto err_card; }

	vpcm = kzalloc(sizeof(*vpcm), GFP_KERNEL);
	if (!vpcm) { err = -ENOMEM; goto err_vsnd; }

	vsnd->vdev      = vdev;
	vsnd->card      = card;
	vsnd->pcm_state = vpcm;
	vpcm->vsnd      = vsnd;

	spin_lock_init(&vpcm->lock);
	INIT_WORK(&vpcm->start_work,       vsnd_start_work);
	INIT_WORK(&vpcm->stop_work,        vsnd_stop_work);
	INIT_WORK(&vpcm->refresh_lag_work, vsnd_refresh_lag_work);
	mutex_init(&vsnd->ctl_lock);
	init_completion(&vsnd->ctl_done);

	atomic_set(&vpcm->pending_periods, 0);
	hrtimer_init(&vpcm->timer, CLOCK_MONOTONIC, HRTIMER_MODE_REL);
	vpcm->timer.function = vsnd_hrtimer_fn;
	tasklet_setup(&vpcm->tasklet, vsnd_tasklet_fn);

	vdev->priv = vsnd;

	err = virtio_find_vqs(vdev, VSND_VQ_MAX, vsnd->vqs, cbs, names, NULL);
	if (err)
		goto err_vpcm;

	{
		u32 cfg_jacks = 0, cfg_streams = 0, cfg_chmaps = 0;
		virtio_cread(vdev, struct virtio_snd_config, jacks,   &cfg_jacks);
		virtio_cread(vdev, struct virtio_snd_config, streams, &cfg_streams);
		virtio_cread(vdev, struct virtio_snd_config, chmaps,  &cfg_chmaps);
		pr_info("virtio_snd: the QEMU config has jacks=%u streams=%u chmaps=%u\n",
			cfg_jacks, cfg_streams, cfg_chmaps);
	}

	strscpy(card->driver,    "virtio_snd",          sizeof(card->driver));
	strscpy(card->shortname, "VirtIO Sound",        sizeof(card->shortname));
	strscpy(card->longname,  "VirtIO Sound Device", sizeof(card->longname));

	err = snd_pcm_new(card, "VirtIO PCM", 0, 1, 0, &pcm);
	if (err < 0)
		goto err_vqs;

	pcm->private_data = vsnd;
	snd_pcm_set_ops(pcm, SNDRV_PCM_STREAM_PLAYBACK, &vsnd_pcm_ops);
	strscpy(pcm->name, "VirtIO PCM", sizeof(pcm->name));

	err = snd_card_register(card);
	if (err < 0)
		goto err_vqs;

	virtio_device_ready(vdev);
	g_vsnd_dev = vsnd;
	pr_info("virtio_snd: registered as ALSA card %d\n", card->number);
	return 0;

err_vqs:   vdev->config->del_vqs(vdev);
err_vpcm:  kfree(vpcm);
err_vsnd:  kfree(vsnd);
err_card:  snd_card_free(card);
	return err;
}

static void vsnd_remove(struct virtio_device *vdev)
{
	struct vsnd_dev *vsnd = vdev->priv;
	struct vsnd_pcm *vpcm = vsnd->pcm_state;
	int i;

	g_vsnd_dev = NULL;

	cancel_work_sync(&vpcm->start_work);
	cancel_work_sync(&vpcm->stop_work);
	cancel_work_sync(&vpcm->refresh_lag_work);
	hrtimer_cancel(&vpcm->timer);
	tasklet_kill(&vpcm->tasklet);

	virtio_reset_device(vdev);
	vdev->config->del_vqs(vdev);

	for (i = 0; i < VSND_TX_BUFS; i++)
		kfree(vpcm->bufs[i].data);

	snd_card_free(vsnd->card);
	kfree(vpcm);
	kfree(vsnd);
}

static const struct virtio_device_id vsnd_id_table[] = {
	{ VIRTIO_ID_SOUND, VIRTIO_DEV_ANY_ID },
	{ 0 },
};
MODULE_DEVICE_TABLE(virtio, vsnd_id_table);

static struct virtio_driver virtio_snd_driver = {
	.driver.name = "virtio_snd",
	.driver.owner = THIS_MODULE,
	.id_table    = vsnd_id_table,
	.probe       = vsnd_probe,
	.remove      = vsnd_remove,
};
module_virtio_driver(virtio_snd_driver);

/* A read-only parameter that gives the live audio pipeline depth in ms. The
 * guest part comes from frames_in_flight (TX buffers submitted minus TX
 * buffers returned) and the current PCM rate; the host part comes from
 * pipeline_extra_frames, which covers the CoreAudio output buffer, USB and
 * the DAC. The clock hook in deck_shim.so (core/clock.c) uses it as the
 * automatic offset for OptFstUdpServer, so slave-mode sync compensates for
 * the current latency. */
static int audio_latency_ms_get(char *buffer, const struct kernel_param *kp)
{
	struct vsnd_dev *vsnd = g_vsnd_dev;
	struct vsnd_pcm *vpcm;
	long inflight;
	unsigned int rate, ms = 0;
	unsigned long flags;
	u32 extra_frames = 0;

	if (!vsnd || !vsnd->pcm_state)
		return scnprintf(buffer, PAGE_SIZE, "0\n");

	vpcm = vsnd->pcm_state;
	spin_lock_irqsave(&vpcm->lock, flags);
	inflight = vpcm->frames_in_flight;
	rate = vpcm->rate;
	extra_frames = vpcm->host_extra_frames;
	spin_unlock_irqrestore(&vpcm->lock, flags);

	{
		unsigned int guest_ms = 0, host_ms = 0;
		if (rate > 0) {
			if (inflight > 0)
				guest_ms = (unsigned int)((inflight * 1000) / rate);
			host_ms = (unsigned int)(((u64)extra_frames * 1000) / rate);
		}
		ms = guest_ms + host_ms;
		if (ms > AUDIO_LATENCY_MS_MAX)
			ms = AUDIO_LATENCY_MS_MAX;
		/* The output is CSV (guest_ms,host_ms,total_ms); cfgd parses it
		 * and pushes the latency to the host UI. */
		return scnprintf(buffer, PAGE_SIZE, "%u,%u,%u\n",
				 guest_ms, host_ms, ms);
	}
}
static const struct kernel_param_ops audio_latency_ms_ops = {
	.get = audio_latency_ms_get,
};
module_param_cb(audio_latency_ms, &audio_latency_ms_ops, NULL, 0444);
MODULE_PARM_DESC(audio_latency_ms,
	"Read-only: the live audio pipeline depth in ms, guest plus host, "
	"capped at 200 ms. deck_shim.so uses it for the slave-mode clock shift "
	"(clock.c) and the master-mode delayed send (link.c), so the slave's "
	"audible output stays aligned. The watchdog forces an xrun when "
	"frames_in_flight stays deep too long, which restores the pipeline depth.");

/* The manual override for the clock shift and the delayed send in
 * deck_shim.so (core/clock.c and core/link.c). When it is non-zero, the shim
 * uses it instead of audio_latency_ms; 0 (the default) follows
 * audio_latency_ms. */
static unsigned int link_pos_offset_ms;
module_param(link_pos_offset_ms, uint, 0644);
MODULE_PARM_DESC(link_pos_offset_ms,
	"Manual override for the slave-mode shift in ms. 0 uses "
	"audio_latency_ms; any other value is used as a fixed shift.");

/* The master switch for all audio-latency compensation.
 *  0 (default): both deck_shim.so hooks pass through. core/clock.c applies
 *     no clock shift to OptFstUdpServer, core/link.c delays no sendto or
 *     sendmsg, and audio plays without Pro DJ Link sync compensation.
 *  1: both hooks are active, with the slave-mode clock shift on
 *     OptFstUdpServer and the master-mode delayed send on Pro DJ Link
 *     broadcasts. */
static unsigned int audio_sync_enabled;
module_param(audio_sync_enabled, uint, 0644);
MODULE_PARM_DESC(audio_sync_enabled,
	"Master switch for the audio-sync hooks in the deck_shim.so LD_PRELOAD. "
	"0 off (no compensation), 1 on.");

/* verbose_stats is declared at the top of the file, because vsnd_tx_submit
 * reads it. */
module_param(verbose_stats, uint, 0644);
module_param(period_frames, uint, 0444);
MODULE_PARM_DESC(period_frames, "The ALSA period in frames (a power of two from 64 to 1024)");
MODULE_PARM_DESC(verbose_stats,
	"Log the TX, pool and xrun counters every 300 TX buffers (about 3 s). 0 off, 1 on.");


MODULE_LICENSE("GPL");
MODULE_AUTHOR("cdj3k-emu");
MODULE_DESCRIPTION("Minimal virtio-sound PCM playback driver with hrtimer timing and TX buffer accumulation");
MODULE_VERSION("1.6");
