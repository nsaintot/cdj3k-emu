# cdj3k-emu - Audio Stack

> Reference for the host-side audio pipeline, the guest-side driver, the
> realtime scheduling overlay, and the LD_PRELOAD shims that align Pro DJ
> Link slave/master playback with audible output.

---

## End-to-end path

EP122 (Pioneer's CDJ firmware) is a JUCE app inside the QEMU guest. It
writes audio via libasound to `/dev/snd/pcmC0D0p`, served by our in-tree
`virtio_snd.ko`, which speaks virtio-sound to QEMU's patched
`hw/audio/virtio-snd.c`. QEMU forwards to macOS CoreAudio via the
patched `audio/coreaudio.m`. The hot path bypasses QEMU's audio mixer
entirely.

```
EP122 (JUCE, SCHED_FIFO 98 on its own pinned core in the guest)
  └── libasound writei → virtio_snd.ko (guest kernel)
        └── virtio TX queue
              └── QEMU virtio-snd device
                    ├── virtio_snd_handle_tx_xfer (BQL, main thread)
                    │     └── stream->queue (per-stream FIFO, queue_mutex)
                    │
                    │   ┌────────────────────────────────────────────────┐
                    │   │ vsnd_writer_thread_fn (dedicated pthread,      │
                    │   │ Mach THREAD_TIME_CONSTRAINT_POLICY 0.5/2 ms,   │
                    │   │ off-BQL, polls every 1 ms)                     │
                    │   │   pulls bufs from stream->queue                │
                    │   │   resamples if needed (cubic Hermite + biquad) │
                    │   │   applies soft-PLL clock-skew correction       │
                    │   │   writes F32 stereo into the SPSC ring         │
                    │   │   pushes deferred-return record to pending list│
                    │   └────────────────────────────────────────────────┘
                    │             │
                    │             ▼
                    │   ┌────────────────────────────────────────────────┐
                    │   │ vsnd_bypass_ring   128 KiB SPSC, lock-free     │
                    │   │ head ← writer pthread (release)                │
                    │   │ tail ← IOProc      (release)                   │
                    │   └────────────────────────────────────────────────┘
                    │             │
                    │             ▼
                    │   ┌────────────────────────────────────────────────┐
                    │   │ out_device_ioproc  (CoreAudio HAL RT thread)   │
                    │   │   lock-free bypass fast-path                   │
                    │   │   bypass_read into IOProc output buffer        │
                    │   │   memset 0 + note_underrun on ring-empty       │
                    │   │   always fills `out` (never leaves stale data) │
                    │   └────────────────────────────────────────────────┘
                    │             │
                    │             ▼
                    │       USB / built-in DAC → speaker
                    │
                    └── vsnd_drain_timer_cb (BQL, every 2 ms)
                          ├── vsnd_bypass_pending_drain
                          │     → virtqueue_push + virtio_notify (needs BQL)
                          └── periodic xrun-rate logger (1 Hz, gated)
```

The pipeline activates the bypass for any stereo S32/F32 stream the
guest negotiates. When the guest's negotiated rate equals the HAL
rate, the writer takes a passthrough fast-path (one S32→F32 multiply
per sample, then memcpy). When the rates differ, the writer engages a
cubic Hermite resampler with an anti-alias biquad on the downsample
case. The QEMU mixer (`AUD_write`) handles only unsupported guest
formats (non-stereo, non-S32/F32).

---

## Guest driver - `guest/modules/virtio_snd/virtio_snd.c`

### Geometry

- **Period**: 64 frames × 8 bytes/frame = 512 bytes (S32 stereo).
- **Per-buf size**: `VSND_QEMU_FACTOR = 16` ALSA periods → one QEMU TX
  buf = 8 KiB = 1024 frames ≈ 10.67 ms at 96 kHz.
- **Pool**: `VSND_TX_BUFS = 32` TX bufs.
- **Rates advertised**: 44.1 / 48 / 96 kHz. The Pioneer firmware
  negotiates 96 kHz S32 in practice.
- **Hrtimer**: fires per ALSA period (~0.67 ms at 96 kHz). Every 16th
  fire, the accumulated TX buf is submitted to QEMU.

### Defensive mechanisms

#### `VSND_OVERRUN_CLAMP = FACTOR × TX_BUFS / 2 = 256`

Caps `1 + missed_intervals` from `hrtimer_forward_now()` after macOS
preemption. Periods beyond that are dropped - the driver gives up
catching them up rather than cramming them into one tasklet run.

#### Pool-empty handling (Phase 1 retry / extended / Phase-2 silent advance)

When the pool is empty, behaviour is governed by `pool_stall_ticks`:

| `pool_stall_ticks` | Phase | Action |
|---|---|---|
| 0-31 | Phase-1 retry | break out, hope `tx_done` refills |
| 32-511 with `in_flight > 0` | Phase-1 extended | be patient - QEMU is holding our bufs |
| ≥ 512 (`VSND_PHASE1_HARDCAP`) | Phase-2 force | advance hw_ptr without consuming audio |
| any, with `in_flight == 0` | Phase-2 immediate | QEMU transport broken; keep JUCE going |

The reset is gated by `VSND_POOL_HEALTHY = 8` - counter only clears
when pool depth reaches 8, preventing oscillation from wiping the
hardcap.

#### Phase-2-burst → ALSA xrun

After `VSND_XRUN_THRESHOLD = FACTOR × 4 = 64` consecutive Phase-2
advances (~42 ms of dropped audio), the driver fires
`snd_pcm_stop_xrun()`. JUCE's `snd_pcm_recover()` resets `appl_ptr`
and `hw_ptr` cleanly - far better than letting JUCE detect drift via
cascading "snd over delta" messages.

#### Pipeline-depth watchdog → forced xrun

When `frames_in_flight > 10240` (≈107 ms at 96 kHz) for 8 seconds
sustained, the driver forces an xrun. JUCE re-prepares. The prepare
path detects deep backlog and sends `VIRTIO_SND_R_PCM_RELEASE` to
QEMU - see "RELEASE-flushes-pending" in the host section. A 60-second
cooldown caps audible recovery clicks at ≤ 1/min under chronic stress.

### Diagnostics

Every 300 TX submissions (~3.2 s wall-clock):

```
virtio_snd: tx=N stall=A(+a) pop=B(+b) xrun=C wdog=W overrun=D(d) gap=E(e)us lat=g+h=Tms inflight_n=N/32
```

| Field | Meaning |
|---|---|
| `tx` | Total TX bufs submitted |
| `stall` | Phase-1 stall events |
| `pop` | Phase-2 silent advances (audible clicks) |
| `xrun` | `snd_pcm_stop_xrun()` calls (natural + watchdog) |
| `wdog` | Watchdog-forced xruns |
| `overrun` | Max `hrtimer_forward_now()` overrun |
| `gap` | Max inter-TX gap in µs (macOS preemption windows) |
| `lat=g+h=T` | Guest pipeline + host pipeline = total ms |
| `inflight_n` | TX bufs currently in flight |

Healthy steady state: `stall=0(+0) pop=0(+0) xrun=0 wdog=0`, `lat ≈ 50-70 ms`.

---

## Host side - patches under `qemu/patches/`

The whole host audio path is delivered as patches that re-apply
cleanly against the upstream QEMU snapshot. After a fresh
`qemu/build.sh` the `qemu/src/` tree is wiped, the patches are
re-applied in numerical order, and `qemu-system-aarch64` is rebuilt.

| Patch | Component |
|---|---|
| `07-coreaudio-bypass.patch` | `audio/coreaudio.m` + `qapi/audio.json` |
| `08-virtio-snd-bypass.patch` | `hw/audio/virtio-snd.c` |
| `09-system-main-qos.patch` | `system/main.c` (Mach RT on QEMU main loop) |
| `10-virtio-blk-removable.patch` | `hw/block/virtio-blk.c` (NULL-guard on shutdown race) |
| `11-console-vc-quiet.patch` | `ui/console-vc.c` (no-op `vt100_update_cursor`) |
| `12-hvf-vcpu-qos.patch` | `accel/hvf/hvf-accel-ops.c` (QoS on VCPU threads) |

(Patches 01-06 are unrelated infrastructure: ivshmem and the shm
display backend used for the LCD framebuffer.)

### Bypass ring (`08-virtio-snd-bypass.patch`)

- **Size**: 128 KiB (~170 ms at 96 kHz F32 stereo). Big enough to
  absorb HAL-side jitter and brief macOS preemption tails on the
  writer thread.
- **Format in the ring**: F32 stereo, regardless of guest source
  format. The writer converts (S32→F32 multiply, or cubic-interp
  resampler output is already F32).
- **Synchronization**: single-producer / single-consumer atomics.
  Writer publishes `head` with `memory_order_release`; reader
  acquires `head` with `memory_order_acquire`. Tail is published by
  the reader with release; writer reads tail with acquire for the
  free-space calculation. No mutex on the hot path.
- **Cold-start prefill**: 16 KiB of zeros (~21 ms at 96 kHz) is
  loaded on `bypass_set_active(1)` if `src_fmt` changes from the
  cached value. The IOProc has something safe to drain while the
  writer races to produce its first real audio.
- **Warm re-activation**: when the guest's PCM-prepare cycle fires
  `set_active(0)` then `set_active(1)` with the same `src_fmt`
  (steady-state xrun recovery, ~once a minute under load), the ring
  contents are preserved. Resetting head/tail here would inject ~170
  ms of silence into the audible output on every recovery.

### Dedicated writer pthread (`08-virtio-snd-bypass.patch`)

`vsnd_writer_thread_fn`:

- Spawned by `bypass_set_active(1)` (the very first activation per
  process), runs for the lifetime of the QEMU subprocess.
- Polls every 1 ms via `nanosleep`.
- Holds `Mach THREAD_TIME_CONSTRAINT_POLICY` (period 2 ms,
  computation 0.5 ms, constraint 1 ms, preemptible=1). The contract
  is sized comfortably above the actual per-wake work (tens of µs),
  so macOS RT scheduling honors it without throttling.
- **Off-BQL**. Acquires `stream->queue_mutex` briefly to dequeue bufs
  (the BQL thread's `virtio_snd_handle_tx_xfer` also takes this
  mutex, but only for the duration of a `QSIMPLEQ_INSERT_TAIL`).
- Does NOT do `virtqueue_push` or `virtio_notify` - those need BQL,
  so completed bufs go on a pending list (`vsnd_bypass_pending`)
  protected by its own pthread mutex, and the BQL drain timer reaps
  them.

This is the critical architectural decision: the ring refill is
immune to main-thread preemption. The only thing that can stall the
writer is macOS preempting the worker pthread itself, which the RT
contract protects against.

### Resampler (`08-virtio-snd-bypass.patch`)

`vsnd_bypass_write_convert` dispatches to one of two paths based on
the live HAL rate vs. the guest's negotiated rate:

**Passthrough** (`hal_rate == guest_rate` or HAL rate not yet cached):
straight S32→F32 multiply per sample, then memcpy into the ring. Used
in the SMSL @ 96 kHz / guest @ 96 kHz common case - bit-perfect.

**Resampled**: cubic Hermite (Catmull-Rom tangents) over a 4-sample
sliding window per channel. C1-continuous at input-frame boundaries
- no slope-jump aliasing the way linear interpolation produced. When
downsampling (input rate > output rate), a 2nd-order Butterworth
lowpass biquad runs ahead of the cubic with cutoff at 0.46 × output
rate, killing the high-frequency content that would otherwise fold
into the audible band.

**Soft-PLL clock-skew correction**: even with matched nominal rates,
small clock drift between the guest's audio thread and the HAL
device's actual clock would slowly fill or drain the ring. An
integrator-only PLL accumulates the ring-depth error vs. an 80 ms
target depth and modulates the resampler ratio by at most ±0.5%. The
correction is so small per-call (1e-6 × normalized error) that the
ratio modulation is sub-audible, but over seconds it locks the
writer's effective output rate onto the reader's actual clock. No
discrete frame-drop clicks, no buildup-then-watchdog-xrun cycles.

State (sliding window, fractional phase, biquad state, PLL
integrator) is reset only on cold-start (format change). The same
state carries across the guest's periodic xrun-recovery prepare
cycles so the phase doesn't restart on every guest watchdog fire.

### IOProc bypass fast-path (`07-coreaudio-bypass.patch`)

`out_device_ioproc`:

- The bypass-active check is **before** any mutex acquisition. If
  bypass is live, the IOProc never touches `buf_mutex` - the SPSC
  ring read is lock-free, the pipeline-depth `mHostTime` update is a
  single atomic store. This eliminated a whole class of "plastic
  pop" sounds that came from the IOProc blocking on `buf_mutex` while
  QEMU's audio frontend held it on the main thread.
- Every early-return path **fills `out` with `memset 0`** before
  returning. The IOProc's output buffer is owned by CoreAudio and is
  not zeroed between calls; returning without writing meant the DAC
  played whatever stale data was there from a previous fire (the
  "held-audio plateau" artifacts in the iZotope spectrograms). With
  the always-fill rule, the worst case is true silence - and our
  underrun counter reflects that 1:1.

### HAL samplerate cache (`07-coreaudio-bypass.patch`)

`coreaudio_get_active_samplerate()` is now an atomic load of
`g_coreaudio_cached_samplerate` - no HAL property read on the hot
path. The cache is seeded by `init_out_device` and refreshed by a
property listener installed on `kAudioDevicePropertyNominalSampleRate`
of the bound device. Previously this function did a synchronous
`AudioObjectGetPropertyData` call to coreaudiod for every guest read
of virtio-snd's config space - under contention those calls could
balloon to multiple ms, stalling the BQL thread responsible for
refilling the ring. The cache makes it a single instruction.

### Per-instance device binding (`07-coreaudio-bypass.patch`)

QAPI extension adds `device-uid` to `AudiodevCoreaudioPerDirectionOptions`.
QemuConfig forwards as `-audiodev coreaudio,id=audio0,out.device-uid=<UID>`
when the user picks a specific output device from the UI's "Output
Device" submenu. `coreaudio_get_voice_out` resolves the UID via
`kAudioHardwarePropertyTranslateUIDToDevice` before falling back to
the system default output, so multi-instance setups can pin each
QEMU instance to a different physical DAC.

### HAL property listeners (`07-coreaudio-bypass.patch`)

Two listeners are installed:

- `kAudioDevicePropertyNominalSampleRate` on the bound device:
  refreshes the cached samplerate without invalidating the bypass.
  The writer's resampler picks up the new rate on its next call via
  the soft-PLL, so an external rate flip (e.g. a recording tool
  setting the device to a different rate) translates to a sub-audible
  pitch nudge rather than a path switch.
- `kAudioHardwarePropertyDefaultOutputDevice` (system-wide): on
  default-output change, invalidates the bypass and re-runs
  `fini_out_device` + `init_out_device` against the new device. The
  next guest PCM prepare re-engages the bypass against it.

### `VIRTIO_SND_R_PCM_RELEASE` handler

When the guest watchdog detects deep pipeline depth (`inflight_n ≥ 10`)
it sends `RELEASE`. The handler:

- Returns every TX buf still in `stream->queue` (the standard virtio
  spec requirement).
- Calls `vsnd_bypass_pending_flush` so the guest's `frames_in_flight`
  counter resets to 0 - required for the watchdog cooldown.
- **Leaves the bypass ring contents intact.** The IOProc keeps
  playing the buffered audio while the writer fills behind it with
  the guest's recovered stream. The transition between old and new
  ring content is at worst a small position jump (typically < 0.4
  beats of program material).

### Realtime scheduling

- **QEMU main loop** (`09-system-main-qos.patch`): `THREAD_TIME_CONSTRAINT_POLICY`
  with period 10 ms, computation 1 ms, constraint 5 ms. Keeps the
  drain timer firing reliably under macOS load.
- **HVF VCPU threads** (`12-hvf-vcpu-qos.patch`): `QOS_CLASS_USER_INTERACTIVE`.
  Bumps the threads up macOS's QoS ladder so the host preempts them
  less aggressively under load. QoS is the right tool here rather
  than a hard time-constraint contract: VCPU threads run for
  variable-length uninterrupted spans inside `hv_vcpu_run`, which
  violates the periodic-compute model an RT contract assumes and
  causes macOS to throttle the thread.
- **Bypass writer pthread** (`08-virtio-snd-bypass.patch`):
  `THREAD_TIME_CONSTRAINT_POLICY` with period 2 ms, computation
  0.5 ms, constraint 1 ms - sized for the worker's actual workload
  (tens of µs per wake) with headroom.

Together these reduce - but cannot eliminate - macOS scheduler
preemption windows.

### Telemetry (`08-virtio-snd-bypass.patch`)

Lock-free atomic counters:

- `vsnd_bypass_underrun_frames` - every IOProc cycle that finds the
  ring empty adds `frame_size` (one IOProc buffer's worth at the HAL
  rate).
- `vsnd_bypass_overrun_count` - every writer-thread attempt that
  finds the ring too full to fit the upcoming batch.

A periodic logger on the drain timer emits **at most one stderr line
per second**, gated on whether the deltas are non-zero:

```
virtio-snd: xrun report - underrun frames=N (~U us of silence in last second), overrun count=M (writer dropped batches)
```

Silent on a clean run. The cumulative totals are exported via the
weak-import C symbols `virtio_snd_bypass_underrun_total_frames` /
`virtio_snd_bypass_overrun_total` for any external integration that
wants to show them in the UI.

---

## Live audio-latency surface

Single sysfs file, read-only, capped at 200 ms:

```
/sys/module/virtio_snd/parameters/audio_latency_ms
```

Format: `200 (guest=309 host=34 rate=96000)` - headline number is
capped, the breakdown shows raw guest + host components.

| Component | Source |
|---|---|
| Guest `frames_in_flight` | `submit++` / `tx_done--` in `virtio_snd.c` |
| Host `pipeline_extra_frames` | HAL `mHostTime` + static device latency, from QEMU config |

When raw exceeds 200 ms for ≥ 8 s, the pipeline-depth watchdog fires
a forced xrun to recover. The 200 ms cap on the exported headline is
sized for ALC's downstream use - see `docs/alc.md` for the consumer
side.

---

## Pro DJ Link sync

The audio pipeline exposes two sysfs surfaces that the user-facing
sync feature consumes:

- `audio_latency_ms` (read-only headline, capped at 200 ms) - the
  live compensation amount.
- `audio_sync_enabled` (writable master switch) - `0` makes the
  consumers fully inert.

The consumer side - LD_PRELOAD shims that shift the
`OptFstUdpServer` clock (slave mode) or delay-send Pro DJ Link
broadcasts (master mode), the `link_pos_offset_ms` manual override,
the per-boot push from the runtime worker, and the UI toggle - lives
in `docs/alc.md`.

---

## Files

| Path | Role |
|---|---|
| `guest/modules/virtio_snd/virtio_snd.c` | Kernel driver: timing, pool, defenses, watchdog, prepare-flush, sysfs |
| `qemu/patches/07-coreaudio-bypass.patch` | Host: IOProc lock-free path, samplerate cache, UID binding, listeners, QAPI extension |
| `qemu/patches/08-virtio-snd-bypass.patch` | Host: bypass ring, writer pthread, resampler, soft-PLL, telemetry |
| `qemu/patches/09-system-main-qos.patch` | Host: Mach RT on QEMU main loop |
| `qemu/patches/11-console-vc-quiet.patch` | Host: BQL relief (no-op `vt100_update_cursor`) |
| `qemu/patches/12-hvf-vcpu-qos.patch` | Host: `QOS_CLASS_USER_INTERACTIVE` on HVF VCPU threads |
| `crates/cdj3k-emu-runtime/src/config.rs` | `-audiodev coreaudio,out.buffer-length=5000,out.device-uid=…` |
| `crates/cdj3k-emu-platform/src/audio_devices.rs` | HAL output device enumeration for the UI picker |
| `crates/cdj3k-emu-platform/src/menu.rs` | Audio → Output Device submenu, 5 s refresh |
| `crates/cdj3k-emu-storage/src/settings.rs` | Per-instance `audio_device_uid` persistence |

---

## Diagnostic protocol

```bash
# Live latency (single most useful number)
ssh root@<guest> 'while :; do cat /sys/module/virtio_snd/parameters/audio_latency_ms; sleep 1; done'

# Guest driver counters (fires every 3.2 s) - most diagnostic single source
ssh root@<guest> 'dmesg -w | grep virtio_snd:'

# Host xrun report (silent on clean run, one line per second when active)
# Appears in the QEMU subprocess stderr captured by the parent .app

# Verify which path the writer took (passthrough vs resampled vs AUD_write)
# - All clean & inst at 96 kHz → passthrough
# - Rate mismatch → resampler (visible: PLL integrator moving)
# - Bypass invalidated → AUD_write (visible: "INVALIDATED" stderr line)
```

Healthy steady-state guest line:

```
virtio_snd: tx=N stall=0(+0) pop=0(+0) xrun=0 wdog=0 overrun=K(1-3) gap=K(11000±500)us lat=21+30=51ms inflight_n=5/32
```

Healthy host xrun report:

```
(no output - silent gate when underrun=0 and overrun delta=0)
```

Stress / preemption signatures:

| Symptom | Source | Mitigation |
|---|---|---|
| `gap=2000000us` in guest log | macOS preempted the HVF VCPU thread | `12-hvf-vcpu-qos.patch` raises priority; environmental tuning beyond that |
| Sustained `wdog++` every ~60 s | Clock skew filled the ring, watchdog forced recovery | Soft-PLL in `08-…bypass.patch` keeps depth at 80 ms target; ensure that patch is applied |
| `underrun frames > 0` in host xrun report | Ring went empty | Writer thread preemption; check `12-hvf-vcpu-qos.patch`, host CPU load |
| `overrun count > 0` sustained | Writer outpacing reader | Soft-PLL not pulling depth down fast enough; check guest vs HAL rates |
| Pops on music but not on silence | Guest-side DSP (JUCE / Pioneer firmware) | Outside our scope; verified host pipeline is bit-perfect via sine-wave test (see below) |
| Pops on a pure sine | Host pipeline or HAL | Compare with stress test; isolate via the sine-direct test |

### Bit-perfect verification

The host pipeline can be proved bit-perfect by piping a host-generated
sine through `aplay` on the guest, bypassing the Pioneer firmware
entirely:

```bash
# 1. Free virtio-snd on the guest
ssh root@<guest> 'pkill -9 EP122; sleep 1'

# 2. Generate clean S32 stereo 96 kHz 1 kHz sine on the macOS host
#    and pipe through ssh to aplay
python3 -c '
import struct, math, sys
r, f, amp = 96000, 1000, 0x40000000
for n in range(r * 30):
    v = int(amp * math.sin(2*math.pi*f*n/r))
    sys.stdout.buffer.write(struct.pack("<ii", v, v))
' | ssh root@<guest> 'aplay -f S32_LE -c 2 -r 96000 -D hw:0'
```

If the recorded output is a clean horizontal line in a spectrogram
(no vertical disturbances), the host pipeline is bit-perfect for the
matched-rate case. Any pops in the recording are produced
downstream of our IOProc (HAL or DAC).

To restore EP122 after the test, restart the QEMU instance via the
.app launcher.

---

## Known limitations

- **JUCE/Pioneer firmware DSP**: produces sample-level micro-pops on
  loud transients even with a bit-perfect transport path (verified
  by the sine-direct test below). The bypass transmits these intact.
- **macOS HAL preemption under heavy host stress**: when every CPU
  core is saturated, RT/QoS-elevated threads can miss deadlines.
  CoreAudio's HAL IO thread is managed by macOS itself and may emit
  broadband impulses on the DAC stream during such events.
- **Clock skew bounds**: the soft-PLL integrator is clamped to
  ±0.5% adjustment. Typical consumer DAC drift relative to the host
  crystal sits at 10-50 ppm (0.001-0.005%), well within the clamp.
  Drift beyond ~5000 ppm would saturate the PLL and let the ring
  slowly fill or empty over many minutes.
- **48 kHz downsample**: with the macOS device at 48 kHz and the
  guest at 96 kHz, the 2:1 downsample + Butterworth biquad
  pre-filter still folds some near-Nyquist content into the audible
  band. Setting the device to 96 kHz in Audio MIDI Setup hits the
  passthrough fast path and removes the downsample step.
