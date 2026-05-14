// SPDX-License-Identifier: GPL-2.0
/*
 * virtio_snd.c - Minimal virtio-sound PCM playback driver, Linux 6.6 / AArch64.
 *
 * ALSA timing: hrtimer fires every VSND_QEMU_FACTOR ALSA periods, advancing
 * hw_ptr and waking the EP122/JUCE application.  Firing at the TX submission
 * rate (~93 Hz at 48 kHz / 64-frame) rather than the ALSA period rate
 * (~750 Hz) cuts EP122 wakeup frequency and PCM lock contention by 8×,
 * preventing CPU saturation in EP122's audio thread.
 *
 * Audio forwarding: each hrtimer tick copies VSND_QEMU_FACTOR ALSA periods
 * from the ring buffer (at OLD hw_ptr positions) into one TX buffer and
 * submits it to QEMU.  hw_ptr is only advanced for periods that are actually
 * copied - if the TX pool is exhausted the driver stalls hw_ptr so EP122
 * receives honest back-pressure instead of silent data corruption.
 *
 * Buffer sizing: periods_min=2 lets EP122 negotiate its preferred buffer.
 * The TX submission count is capped at buffer_size/period_size - 1 so
 * hw_ptr never fully laps appl_ptr within one tasklet run.
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

/* PCM format codes - from virtio_snd.h enum virtio_snd_pcm_fmt */
#define VSND_FMT_S16    5   /* VIRTIO_SND_PCM_FMT_S16  (0=IMA_ADPCM,1=MU_LAW,2=A_LAW,3=S8,4=U8,5=S16) */
#define VSND_FMT_S32    17  /* VIRTIO_SND_PCM_FMT_S32  (...,17=S32) */

/* PCM rate codes - from virtio_snd.h enum virtio_snd_pcm_rate */
#define VSND_RATE_44100 6   /* VIRTIO_SND_PCM_RATE_44100 (0=5512,1=8000,...,5=32000,6=44100) */
#define VSND_RATE_48000 7   /* VIRTIO_SND_PCM_RATE_48000 */
#define VSND_RATE_96000 10  /* VIRTIO_SND_PCM_RATE_96000 (8=64000,9=88200,10=96000) */

/* Cap reported latency. Beyond this, the pipeline is in a degraded state
 * (post-preemption backlog) and the raw value would feed runaway in the
 * slave-mode shim (shift grows → audible plays sooner → more bufs in flight
 * → shift grows further). Saturating breaks the loop. */
#define AUDIO_LATENCY_MS_MAX 200u

/*
 * VSND_QEMU_FACTOR - number of ALSA periods accumulated into one QEMU TX buf.
 *
 * QEMU's CoreAudio backend returns each TX buf approximately one TX-buf-duration
 * after submission (the audio must be consumed by CoreAudio before the buf is
 * marked "used").  Empirically, with FACTOR=8 (5.33 ms/buf), QEMU takes ~10 ms
 * to return each buf → pool-empty window of ~5 ms per TX cycle → ~1600 Phase-1
 * stalls/s → EP122 writei blocks for up to 10 ms per period → JUCE audio
 * callbacks take 100–800 ms → "snd over" cascade and audible pops.
 *
 * Setting FACTOR=16 (10.67 ms/buf) matches the QEMU return latency so that
 * each buf is returned right as the next submission is due.  Pool stays at
 * ~31/32 bufs, Phase-1 stalls vanish, writei never blocks, snd-over events
 * disappear and audio is clean.
 */
#define VSND_QEMU_FACTOR    16

/* Pool of QEMU TX buffers.  Each holds VSND_QEMU_FACTOR ALSA periods
 * (= 10.67 ms of audio at FACTOR=16, period_size=32, rate=48 kHz).
 *
 * With the host-side deferred-return mechanism (return_tx_buffer is held
 * back until CoreAudio's IOProc has actually consumed the bytes), the
 * round-trip becomes "submit → playback complete → ack" ≈ 30-40 ms - much
 * longer than buf duration. Pool size must cover that round-trip plus
 * headroom, otherwise we get pool-empty Phase-1 cycles that show up as
 * latency jitter on Pro DJ Link.
 *
 * 4 bufs × 10.67 ms = ~43 ms max guest-side in-flight, comfortably above
 * the ~30 ms round-trip. Combined with a 32 KiB host ring, end-to-end is
 * ~85 ms but steady - JUCE reports it honestly via snd_pcm_status_get_delay
 * (since hw_ptr advances at deferred-return time) and the network sync to
 * the other CDJ tracks our true playback position.
 *
/* With deferred TX-buffer return on the QEMU side, each submitted buf stays
 * in flight until CoreAudio actually consumes its frames (~50–100ms for
 * HAL + static device latency + bypass ring, depending on warmup). At
 * 1024 frames per buf @ 96kHz = 10.7ms each, steady state holds 5–10
 * bufs. 32 gives margin for macOS scheduler preemption events (the QEMU
 * main loop can be paused for up to ~25ms; during that window the guest
 * keeps submitting at its own clock, drain timer can't run). */
#define VSND_TX_BUFS        32

/* Verbose periodic stats line in the TX path. Off by default; flip at runtime
 * with `echo 1 > /sys/module/virtio_snd/parameters/verbose_stats` when
 * debugging audio glitches.  Declared up here so vsnd_tx_submit can read it
 * (the matching module_param() goes at the bottom of the file with the rest). */
static unsigned int verbose_stats;

/* ------------------------------------------------------------------ */
/* Wire structures                                                      */
/* ------------------------------------------------------------------ */

struct virtio_snd_config {
	__le32 jacks;
	__le32 streams;
	__le32 chmaps;
	/* QEMU's struct virtio_snd_config has this 4th field (VIRTIO_SND_F_CTLS,
	 * v1.2 spec). MUST be present here for offset alignment - without it,
	 * the next field would land at offset 12 instead of 16, and we'd read
	 * QEMU's controls value (0) instead of pipeline_extra_frames. */
	__le32 controls;
	/* Extension carried by our QEMU overlay (config space size +4 bytes):
	 * depth in frames of the host audio pipeline downstream of the virtio
	 * queue (CoreAudio bypass ring + DAC). The standard upstream virtio
	 * spec doesn't have this; populated only when running against our
	 * patched QEMU. Reads as 0 against vanilla QEMU. */
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
	bool                         in_flight; /* submitted to QEMU, not yet returned */
};

struct vsnd_dev;

/* Set in vsnd_probe, cleared in vsnd_remove. Used by the audio_latency_ms
 * sysfs getter to reach the running stream's frames_in_flight + rate. */
static struct vsnd_dev *g_vsnd_dev;

struct vsnd_pcm {
	struct vsnd_dev          *vsnd;
	struct snd_pcm_substream *substream;

	/* TX buffer free pool (circular).  Accessed from hardirq + tasklet. */
	struct vsnd_tx_buf  bufs[VSND_TX_BUFS];
	struct vsnd_tx_buf *pool[VSND_TX_BUFS + 1];
	int                 pool_head;
	int                 pool_tail;

	/* Accumulation: VSND_QEMU_FACTOR ALSA periods → one QEMU TX buf */
	struct vsnd_tx_buf *accum_buf;   /* TX buf currently being filled */
	unsigned int        accum_count; /* ALSA periods written into accum_buf */

	/* ALSA ring-buffer geometry */
	snd_pcm_uframes_t   hw_ptr;
	snd_pcm_uframes_t   period_size;
	snd_pcm_uframes_t   buffer_size;
	unsigned int        rate;             /* current PCM rate (Hz), set in hw_params */
	long                frames_in_flight; /* submitted - returned; reflects guest pipeline depth */
	u32                 host_extra_frames;/* cached pipeline_extra_frames from QEMU config; refreshed in tasklet */
	unsigned int        period_bytes;     /* bytes per ALSA period */
	unsigned int        qemu_period_bytes;/* bytes per QEMU TX buffer = VSND_QEMU_FACTOR × period_bytes */
	unsigned int        frame_bytes;

	/* hrtimer drives hw_ptr and snd_pcm_period_elapsed */
	struct hrtimer        timer;
	ktime_t               period_time;
	struct tasklet_struct tasklet;
	atomic_t              pending_periods; /* overrun count from hrtimer_forward_now */
	u32                   pool_stall_ticks; /* consecutive tasklet runs with empty pool */
	u32                   phase2_streak;   /* consecutive Phase-2 advances; reset on pool recovery */
	bool                  xrun_pending;    /* set under lock; checked after lock release in tasklet */
	u64                   dbg_xrun_signals;/* total snd_pcm_stop_xrun calls issued */

	/* Pipeline-depth watchdog: when frames_in_flight stays high for
	 * sustained time, force an xrun. Only mechanism that drains the
	 * stream_q backlog (production pause during JUCE recovery lets
	 * the queue empty). Cooldown limits clicks under chronic stress. */
	unsigned long         watchdog_deep_start; /* jiffies when deep state entered, 0 = not in deep */
	unsigned long         watchdog_last_fired; /* jiffies of last forced xrun */
	u64                   dbg_watchdog_fires;  /* total forced xruns */

	/* Diagnostics - written from tasklet/hrtimer, racy but ok for stats */
	ktime_t  dbg_last_tx_time;      /* ktime of last vsnd_tx_submit call */
	u64      dbg_tx_count;          /* total TX buffers submitted */
	u64      dbg_pool_drop;         /* total periods dropped due to pool-empty */
	u32      dbg_max_overrun;       /* all-time largest hrtimer overrun */
	u32      dbg_max_tx_gap_us;     /* all-time largest TX gap (µs) */
	u64      dbg_phase2_drop;       /* total Phase-2 silent hw_ptr advances (audible pops) */
	/* Per-interval (reset on each 300-TX log print) */
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
/* Pool helpers (lock must be held by caller)                         */
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
	 * virtqueue_add_sgs maps req via the DMA API. Stack memory is in
	 * the kernel linear map but some DMA paths (SWIOTLB, non-coherent
	 * mappings) require kmalloc-zone memory. Copy req to a fresh heap
	 * buffer so the DMA mapping always operates on a safe physical page.
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
/* TX queue - submit accumulated buffer to QEMU                       */
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

	/* Track host pipeline depth: this many frames are now in flight
	 * downstream (QEMU bypass ring + CoreAudio + USB + DAC).
	 * vsnd_tx_done decrements when the buffer returns. */
	if (vpcm->frame_bytes)
		vpcm->frames_in_flight +=
			(long)vpcm->qemu_period_bytes / vpcm->frame_bytes;

	/* Diagnostics: track TX cadence */
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
			/* lat = guest pipeline (frames_in_flight) + host pipeline
			 * (HAL + static device latency cached from QEMU). Same value
			 * as audio_latency_ms sysfs but uncapped. inflight_n is the
			 * actual TX-buf-in-flight count (sanity-check vs frames_in_flight).
			 * wdog is the count of watchdog-triggered xrun recoveries. */
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
			/* Reset per-interval counters */
			vpcm->dbg_interval_drop    = 0;
			vpcm->dbg_interval_phase2  = 0;
			vpcm->dbg_interval_overrun = 0;
			vpcm->dbg_interval_gap_us  = 0;
		}
	}
}

/*
 * TX completion: QEMU is done with the buffer - return it to the free pool.
 * Does NOT touch hw_ptr; hrtimer owns ALSA timing.
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
/* hrtimer + tasklet - ALSA timing and audio accumulation             */
/* ------------------------------------------------------------------ */

static void vsnd_tasklet_fn(struct tasklet_struct *t)
{
	struct vsnd_pcm *vpcm = from_tasklet(vpcm, t, tasklet);
	struct snd_pcm_runtime *r;
	snd_pcm_uframes_t old_hw_ptr;
	unsigned long flags;
	int count, i;

	/*
	 * Consume pending periods accumulated by the hrtimer.
	 * Cap at buffer_size/period_size - 1 so we never advance past appl_ptr.
	 * If the TX pool is empty mid-loop we break early and put the unprocessed
	 * remainder back so the next tasklet/hrtimer fire can retry - prevents
	 * silent period loss that caused xruns during track loading.
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

	/* Clamp: never process more periods than the ring can hold minus one guard.
	 * Excess is restored to pending_periods now (before the loop) so the next
	 * hrtimer tick picks it up; otherwise the discarded excess accumulates
	 * into a slow clock drift that turns into a JUCE spin after 1–2 minutes. */
	{
		int cap = (int)(vpcm->buffer_size / vpcm->period_size) - 1;
		if (count > cap) {
			atomic_add(count - cap, &vpcm->pending_periods);
			count = cap;
		}
	}

	for (i = 0; i < count; i++) {
		/* Grab an accumulation buffer when starting a new TX batch.
		 * If the pool is empty (all 8 bufs in flight - should be rare with
		 * 85ms headroom), advance hw_ptr anyway to keep EP122's clock ticking
		 * and let JUCE continue - the dropped period causes a brief audio
		 * glitch but prevents the JUCE spin-loop that results from a stalled
		 * clock. */
		if (vpcm->accum_buf == NULL) {
			if (vsnd_pool_empty(vpcm)) {
				/* Inline drain: QEMU may have marked TX bufs done in
				 * the used ring but the interrupt hasn't fired yet.
				 * Draining here avoids a stall when QEMU is slightly
				 * behind flushing its virtqueue callbacks. */
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
				 * Hybrid pool-empty strategy:
				 *
				 * Phase 1 - retry (no drift): break without advancing
				 * hw_ptr for up to VSND_QEMU_FACTOR*2 consecutive
				 * tasklet runs (~2 × TX period).  Releasing the lock
				 * lets vsnd_tx_done refill the pool.  In normal
				 * operation QEMU returns a buf within ~5 ms - zero
				 * clock drift, zero ep122 cascade.
				 *
				 * Phase 1 extended - in-flight bufs present: if the
				 * short Phase-1 window expires but QEMU still holds
				 * our TX buffers (in_flight > 0), QEMU is consuming
				 * audio normally and will return the bufs shortly.
				 * Stay in Phase 1 indefinitely - Phase 2 here would
				 * cause spurious pops during legitimate recovery after
				 * a long EP122 thread stall (e.g. 19.9 s freeze).
				 *
				 * Phase 2 - drop (no deadlock): only when the pool is
				 * empty AND no bufs are in flight.  This means QEMU's
				 * virtio transport is truly broken or QEMU crashed.
				 * Advance hw_ptr to keep EP122's writei from blocking
				 * indefinitely (blocking → guest CPU idle → macOS
				 * deprioritises QEMU vCPU → QEMU never returns bufs
				 * → permanent deadlock).
				 */
				vpcm->pool_stall_ticks++;
				vpcm->dbg_pool_drop++;
				vpcm->dbg_interval_drop++;
				if (vpcm->pool_stall_ticks < VSND_QEMU_FACTOR * 2) {
					break; /* Phase 1: short retry */
				}
				/* Phase 2 gate: only fire if no bufs are in flight,
				 * OR we've been stalled long enough (>VSND_PHASE1_HARDCAP
				 * tasklet runs ≈ 340 ms wall-clock at normal hrtimer rate)
				 * that "QEMU will return a buf shortly" is no longer a
				 * reasonable assumption.  Without the hard cap, sustained
				 * CoreAudio drain slowness leaves all 32 bufs in_flight,
				 * Phase-1 extended runs forever, hw_ptr never advances,
				 * JUCE writei blocks, wall-clock keeps moving → JUCE logs
				 * "snd over delta" and enters the recovery spin observed
				 * with stall counter rising at ~1000/s.
				 * Scan is O(VSND_TX_BUFS) but only runs on pool-empty
				 * paths which are rare in normal operation. */
#define VSND_PHASE1_HARDCAP  (VSND_QEMU_FACTOR * 32)  /* ~340 ms */
				if (vpcm->pool_stall_ticks < VSND_PHASE1_HARDCAP) {
					int j, in_flight = 0;
					for (j = 0; j < VSND_TX_BUFS; j++)
						if (vpcm->bufs[j].in_flight)
							in_flight++;
					if (in_flight > 0)
						break; /* Phase 1 extended: wait for QEMU */
				}
				/* Phase 2 fallback - QEMU transport too slow or broken.
				 * Do NOT reset pool_stall_ticks here: once we've decided
				 * to give up on QEMU we want to keep advancing hw_ptr at
				 * real-time pace (one period per hrtimer tick) until the
				 * pool actually refills.  Resetting would cause Phase-1
				 * to ramp back up to the hard cap before each Phase-2,
				 * yielding ~3 pops/s and a ~94 Hz effective sample rate
				 * during sustained stalls - audibly broken.  The counter
				 * is reset at line ~448 once vsnd_pool_pop succeeds.
				 *
				 * Track consecutive Phase-2 advances; if we cross
				 * VSND_XRUN_THRESHOLD periods of dropped audio, arm an
				 * explicit ALSA xrun signal.  JUCE's xrun-recovery path
				 * (snd_pcm_recover) resets appl_ptr and hw_ptr in lockstep
				 * - much cleaner than letting JUCE detect the drift via
				 * snd-over-delta cascades, which empirically lock the
				 * EP122 HuiProcessor thread into a userspace tight loop
				 * that the kernel can't escape. */
#define VSND_XRUN_THRESHOLD  (VSND_QEMU_FACTOR * 4)  /* 64 periods ≈ 42 ms */
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
			 * Only reset stall_ticks when the pool has recovered to a
			 * healthy depth (>= VSND_TX_BUFS/4 ≈ 85 ms headroom).
			 * Resetting on every successful pop would hide chronic
			 * near-empty oscillation: when QEMU's TX-return rate just
			 * barely matches the guest submission rate, the pool cycles
			 * between 0 and 1-2 bufs at ~1000 stalls/sec, but each pop
			 * wipes the counter so the Phase-1 hardcap never fires and
			 * Phase-2 never gets to drop audio to break the loop. By
			 * gating the reset on a depth threshold, the counter
			 * accumulates across the oscillation, the hardcap eventually
			 * triggers, hw_ptr advances at real-time pace via Phase-2,
			 * JUCE's writei unblocks, and the system escapes the spin in
			 * ~340 ms instead of ~5 minutes (empirically observed).
			 */
#define VSND_POOL_HEALTHY  (VSND_TX_BUFS / 4)
			if (vsnd_pool_depth(vpcm) >= VSND_POOL_HEALTHY) {
				vpcm->pool_stall_ticks = 0;
				vpcm->phase2_streak    = 0;  /* clear xrun streak - we're healthy again */
			}
			vpcm->accum_buf = vsnd_pool_pop(vpcm);
		}

		/* Safe to advance now: EP122 has data and we have a TX buffer. */
		old_hw_ptr = vpcm->hw_ptr;
		vpcm->hw_ptr = (old_hw_ptr + vpcm->period_size) % vpcm->buffer_size;

		char *dst = (char *)vpcm->accum_buf->data +
			    vpcm->accum_count * vpcm->period_bytes;
		char *src = (char *)r->dma_area +
			    old_hw_ptr * vpcm->frame_bytes;
		memcpy(dst, src, vpcm->period_bytes);
		vpcm->accum_count++;

		if (vpcm->accum_count == VSND_QEMU_FACTOR) {
			vsnd_tx_submit(vpcm, vpcm->accum_buf);
			vpcm->accum_buf   = NULL;
			vpcm->accum_count = 0;
			/* Refresh cached host_extra_frames in process context.
			 * One submit ≈ 10.7 ms - fast enough that the cache
			 * tracks pipeline depth changes without lag. */
			schedule_work(&vpcm->refresh_lag_work);
		}
	}

	{
		/* Pipeline-depth watchdog. If frames_in_flight stays above
		 * VSND_WATCHDOG_DEEP_FRAMES (≈100 ms) for VSND_WATCHDOG_DEEP_JIFFIES
		 * sustained, and we haven't fired one in VSND_WATCHDOG_COOLDOWN,
		 * arm an xrun. JUCE re-prepares; the production pause during
		 * recovery lets QEMU's stream_q drain. Single audible click but
		 * pipeline returns to baseline.
		 *
		 * Tuning rationale:
		 *  THRESHOLD: > steady-state max (~50 ms) but well below ceiling
		 *    (~340 ms = TX_BUFS × period). Triggers when truly stuck.
		 *  SUSTAIN: long enough that single preemption events (which
		 *    typically self-recover within a few seconds via natural
		 *    drain) don't trip it.
		 *  COOLDOWN: caps clicks at ≤1/min even under chronic stress. */
#define VSND_WATCHDOG_DEEP_FRAMES   (10 * 1024)        /* ~107 ms at 96 kHz */
#define VSND_WATCHDOG_DEEP_JIFFIES  (8 * HZ)           /* 8 seconds */
#define VSND_WATCHDOG_COOLDOWN      (60 * HZ)          /* 60 seconds */
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

		/* Put back any periods we couldn't process (pool was empty mid-loop). */
		if (i < count)
			atomic_add(count - i, &vpcm->pending_periods);

		/*
		 * Signal an explicit ALSA xrun once we've crossed the Phase-2 burst
		 * threshold.  Must be called outside vpcm->lock since
		 * snd_pcm_stop_xrun acquires the substream's stream lock and may
		 * call back into our trigger op.  Skip the period_elapsed below in
		 * the same tasklet run - the xrun handler stops the stream and any
		 * userspace wake on a stopped stream is wasted.
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
	 * Clamp burst to half the pool capacity (= 16 TX buffers = 256 ALSA
	 * periods = 170 ms of audio) so a single macOS preemption stall can
	 * never drain the entire pool in one shot.
	 *
	 * Prior value (VSND_QEMU_FACTOR * 2 = 32 periods = 2 TX bufs) was set
	 * with the goal of keeping each per-tick hw_ptr jump <30 ms (EP122's
	 * snd-over threshold), but in practice it DROPS overrun excess instead
	 * of catching up: hrtimer_forward_now returns N missed intervals, the
	 * clamp truncates to 32, the other N-32 are silently lost from
	 * pending_periods.  Result observed empirically: snd-over deltas of
	 * 30-500 ms during play (because hw_ptr drifts behind wall-clock by
	 * up to (N-32) × period_time per preemption event), JUCE recovery
	 * cascade fires repeatedly.
	 *
	 * Bumping to FACTOR * TX_BUFS / 2 = 16 * 32 / 2 = 256 periods aligns
	 * with the original comment's stated design ("half the pool capacity")
	 * and lets a single tasklet run absorb a typical 170 ms preemption
	 * window without drift.  Per-burst CPU cost: 256 memcpy + 16 TX submits,
	 * roughly 1 ms of guest CPU - acceptable.
	 */
#define VSND_OVERRUN_CLAMP  (VSND_QEMU_FACTOR * VSND_TX_BUFS / 2)  /* 256 periods ≈ 170 ms */
	if (overrun > VSND_OVERRUN_CLAMP)
		overrun = VSND_OVERRUN_CLAMP;
	atomic_add((int)overrun, &vpcm->pending_periods);
	tasklet_schedule(&vpcm->tasklet);
	return HRTIMER_RESTART;
}

/* ------------------------------------------------------------------ */
/* Workqueue - START / STOP (process context, can sleep)              */
/* ------------------------------------------------------------------ */

static void vsnd_start_work(struct work_struct *w)
{
	struct vsnd_pcm *vpcm = container_of(w, struct vsnd_pcm, start_work);
	struct virtio_snd_pcm_hdr hdr = {
		.code      = cpu_to_le32(VIRTIO_SND_R_PCM_START),
		.stream_id = 0,
	};

	/*
	 * Tell QEMU to start consuming TX buffers.  The hrtimer is already
	 * running (started in TRIGGER_START) so EP122 gets period callbacks
	 * immediately - not after this blocking ctl command completes.
	 */
	vsnd_ctl_cmd(vpcm->vsnd, &hdr, sizeof(hdr));
}

/* Periodically refresh host_extra_frames (HAL + static device latency) from
 * QEMU's config space. Runs in process context - virtio_cread can't be
 * called from the tasklet/hrtimer (may sleep). Scheduled from the tasklet
 * every ~10 ms. */
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
	 * Do NOT hrtimer_cancel/tasklet_kill here: if EP122 rapidly cycles
	 * STOP→PREPARE→START (xrun recovery), TRIGGER_START has already
	 * re-armed the hrtimer.  Cancelling it here would kill the fresh
	 * timer and cause a multi-second gap before the next period callback.
	 * The hrtimer self-stops on the next fire when running==false; it
	 * restarts when TRIGGER_START sets running=true.
	 */

	vsnd_ctl_cmd(vpcm->vsnd, &hdr, sizeof(hdr));

	/* Drain any QEMU-returned TX bufs back to the pool */
	spin_lock_irqsave(&vpcm->lock, flags);
	while ((tb = virtqueue_get_buf(vpcm->vsnd->vqs[VSND_VQ_TX], &len))) {
		tb->in_flight = false;
		vsnd_pool_push(vpcm, tb);
	}
	/* Return the partially-filled accumulation buffer */
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
	/* Period locked at 64 frames (= 512 bytes S32 stereo, 8 bytes/frame):
	 * matches JUCE/EP122's preferred audio-callback cadence (0.67ms wake
	 * interval) and keeps qemu_period_bytes (= 512 × FACTOR = 8 KiB) at
	 * 1/4 of the 32 KiB host bypass ring - comfortable backpressure. */
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

	vpcm->qemu_period_bytes  = vpcm->period_bytes * VSND_QEMU_FACTOR;

	/* hrtimer period duration */
	ns = (u64)vpcm->period_size * 1000000000ULL;
	do_div(ns, params_rate(params));
	vpcm->period_time = ktime_set(0, (long)ns);

	/* (Re)allocate TX data buffers sized for one QEMU period.  NULL the
	 * pointer between kfree and kzalloc so that, if kzalloc fails partway
	 * and ALSA retries hw_params, the next pass doesn't double-free the
	 * already-freed slot (or any slot beyond `i` that still holds its
	 * stale, freed pointer from the previous call). */
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
	 * Reset pending_periods first (outside lock - atomic).
	 * Prevents a stale backlog from a previous session causing an immediate
	 * burst of snd_pcm_period_elapsed calls after the next START.
	 */
	atomic_set(&vpcm->pending_periods, 0);

	/*
	 * Detect post-xrun deep-pipeline state and force a QEMU-side flush.
	 *
	 * After an xrun (natural or watchdog-forced), JUCE calls snd_pcm_prepare.
	 * Without intervention, QEMU's stream->queue + bypass pending list
	 * keep the pre-xrun audio backlog. JUCE re-submits, frames_in_flight
	 * stays deep, audible pipeline never recovers.
	 *
	 * Sending VIRTIO_SND_R_PCM_RELEASE invokes virtio_snd_pcm_flush in
	 * QEMU which empties stream->queue AND calls vsnd_bypass_pending_flush
	 * to release the deferred-return list. The bypass voice and ring stay
	 * active - no AUD_close_out, no PREPARE, no 2s reinit blocker. JUCE's
	 * subsequent snd_pcm_start (which fires after recovery) sends a fresh
	 * VIRTIO_SND_R_PCM_START that resumes things. Net effect: queues
	 * cleared in <1ms, ring continues playing whatever's left in it, JUCE
	 * starts writing fresh audio at the recovered rate.
	 *
	 * Conditional on actual backlog so initial track-load and benign
	 * re-prepares don't fire it. Threshold matches the watchdog deep-state
	 * level (~10 bufs ≈ 100 ms). */
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
			pr_info("virtio_snd: prepare flushing QEMU backlog "
				"(%d bufs in flight)\n", in_flight_count);
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
	/* Don't reset watchdog_last_fired - cooldown should persist across
	 * re-prepares so a single bad event doesn't trigger a click loop. */

	/*
	 * Reseed frames_in_flight from the actual in_flight buf count rather
	 * than zeroing it. After an xrun JUCE calls PREPARE while QEMU still
	 * owns several TX bufs; those bufs will be returned via vsnd_tx_done
	 * which decrements frames_in_flight unconditionally. Zeroing here
	 * caused the counter to go negative buf-by-buf as QEMU drained, which
	 * made audio_latency_ms_get clamp guest_ms to 0 permanently
	 * (slave-sync auto offset broken until reboot).
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
	 * Release the partially-filled accumulation buffer back to the pool
	 * (don't mark it in_flight - it was never submitted to QEMU).
	 */
	if (vpcm->accum_buf) {
		vpcm->accum_buf->in_flight = false;
		vpcm->accum_buf = NULL;
	}

	/*
	 * Rebuild the free pool - but only include bufs that QEMU has already
	 * returned (in_flight == false).  Bufs still in QEMU's virtqueue will
	 * be pushed by vsnd_tx_done when QEMU returns them.  Pushing them here
	 * too would cause double-allocation, corrupting the virtio queue and
	 * triggering the EP122 freeze observed after snd-over recovery.
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
		 * Start the hrtimer HERE in atomic context - not in start_work.
		 * start_work calls vsnd_ctl_cmd which blocks waiting for QEMU's
		 * response; under load this can take seconds.  If the hrtimer
		 * were started there, EP122 would receive no snd_pcm_period_elapsed
		 * callbacks during that wait and log snd-over deltas of 1–2 s,
		 * eventually exhausting its recovery retries and freezing.
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
		pr_info("virtio_snd: QEMU config: jacks=%u streams=%u chmaps=%u\n",
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
	pr_info("virtio_snd: registered as card %d\n", card->number);
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

/* Read-only: live audio pipeline depth in ms, computed from the
 * frames-in-flight counter (TX submitted - TX returned) and the
 * current PCM rate. Reflects the full host pipeline latency:
 * QEMU bypass ring + CoreAudio output buffer + USB + Focusrite DAC.
 * The ep122_shim_clock hook reads this and uses it as the auto offset
 * for OptFstUdpServer, so slave-mode sync compensates whatever the
 * actual measured latency is at any given moment. */
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
		/* CSV: guest_ms,host_ms,total_ms - easily parsed by the
		 * cfg daemon that pushes latency to the host UI. */
		return scnprintf(buffer, PAGE_SIZE, "%u,%u,%u\n",
				 guest_ms, host_ms, ms);
	}
}
static const struct kernel_param_ops audio_latency_ms_ops = {
	.get = audio_latency_ms_get,
};
module_param_cb(audio_latency_ms, &audio_latency_ms_ops, NULL, 0444);
MODULE_PARM_DESC(audio_latency_ms,
	"Read-only: live audio pipeline depth in ms (frames-in-flight + host). "
	"Capped at 200ms. Used by both ep122_shim_clock (slave-mode clock "
	"shift) and ep122_shim_link (master-mode delay-send). Tracks actual "
	"pipeline so slave audible aligns. The watchdog forces an xrun when "
	"frames_in_flight stays deep too long, recovering pipeline depth.");

/* Manual override for ep122_shim_clock. When non-zero, the shim uses
 * this value instead of audio_latency_ms. Useful for testing fixed
 * offsets. Set to 0 (default) for automatic tracking. */
static unsigned int link_pos_offset_ms;
module_param(link_pos_offset_ms, uint, 0644);
MODULE_PARM_DESC(link_pos_offset_ms,
	"Manual override for slave-mode shift in ms. 0 = auto (use "
	"audio_latency_ms). Non-zero = force this fixed value.");

/* Master switch for ALL audio-latency compensation.
 *  0 (default) = full no-op. Both LD_PRELOAD shims pass through:
 *                - ep122_shim_clock: no clock shift on OptFstUdpServer
 *                - ep122_shim_link:  no sendto/sendmsg delay-send
 *                Audio plays raw with no Pro DJ Link sync compensation.
 *  1          = both shims active. Slave-mode clock-shift on
 *                OptFstUdpServer, master-mode delay-send on Pro DJ Link
 *                broadcasts. */
static unsigned int audio_sync_enabled;
module_param(audio_sync_enabled, uint, 0644);
MODULE_PARM_DESC(audio_sync_enabled,
	"Master switch for both LD_PRELOAD audio-sync shims. "
	"0=off (no compensation, raw audio path), 1=on.");

/* Storage declared at the top of the file (vsnd_tx_submit reads it). */
module_param(verbose_stats, uint, 0644);
MODULE_PARM_DESC(verbose_stats,
	"Emit a per-second pr_info with TX/pool/xrun counters. 0=off, 1=on.");


MODULE_LICENSE("GPL");
MODULE_AUTHOR("cdj3k-emu");
MODULE_DESCRIPTION("Minimal virtio-sound PCM playback - hrtimer timing, 16× accumulation, in-flight Phase-2 gate");
MODULE_VERSION("1.6");
