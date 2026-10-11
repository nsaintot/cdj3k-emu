# cdj3k-emu - Audio Stack

> This page describes the host-side audio pipeline, the guest-side driver,
> the realtime scheduling on the host, and the sysfs values that the Pro DJ
> Link sync shims read (the shims themselves are in `docs/alc.md`).

---

## End-to-end path

EP122 (Pioneer's CDJ firmware) is a JUCE app inside the QEMU guest. It
writes audio through libasound to `/dev/snd/pcmC0D0p`, which the project's
`virtio_snd.ko` serves. The driver speaks virtio-sound to QEMU's patched
`hw/audio/virtio-snd.c`. QEMU forwards to CoreAudio on macOS (patched
`audio/coreaudio.m`), PipeWire on Linux (patched `audio/pwaudio.c`) or WASAPI
on Windows (`audio/wasapiaudio.c`, added by patch 18). The hot path bypasses
QEMU's audio mixer. The diagram below shows the macOS path;
[Linux](#linux-pipewire) and [Windows](#windows-wasapi) list what differs.

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

The pipeline activates the bypass for any stereo S32 or F32 stream that the
guest negotiates.

- When the guest's rate equals the HAL rate, the writer takes a passthrough
  path: one S32-to-F32 multiply per sample, then a memcpy.
- When the rates differ, the writer uses a cubic Hermite resampler, with an
  anti-alias biquad when it downsamples.
- The QEMU mixer (`AUD_write`) handles only the formats the bypass does not
  support (not stereo, or neither S32 nor F32).

---

## Guest driver - `guest/modules/virtio_snd/virtio_snd.c`

### Geometry

- **Period**: 64 frames × 8 bytes/frame = 512 bytes (S32 stereo).
- **Buffer size**: one QEMU TX buffer holds `VSND_QEMU_FACTOR = 16` ALSA
  periods, which is 8 KiB or 1024 frames (about 10.67 ms at 96 kHz).
- **Pool**: `VSND_TX_BUFS = 32` TX buffers.
- **Rates advertised**: 44.1, 48 and 96 kHz. The firmware negotiates 96 kHz
  S32.
- **Hrtimer**: fires once per ALSA period (about 0.67 ms at 96 kHz). Every
  16th time, the driver submits the filled TX buffer to QEMU.

The thresholds below that count periods are given for the default 64-frame
period. With a larger period (see [Linux](#linux-pipewire)) they scale so that
each keeps its duration.

### Defensive mechanisms

#### `VSND_OVERRUN_CLAMP = FACTOR × TX_BUFS / 2 = 256`

Caps the missed intervals that `hrtimer_forward_now()` reports after a host
preemption. Periods beyond the cap are dropped, so one tasklet run never
catches up more than half the pool (about 170 ms).

#### Pool-empty handling

When the pool is empty, `pool_stall_ticks` (consecutive tasklet runs with an
empty pool) selects the action:

| `pool_stall_ticks` | Phase | Action |
|---|---|---|
| 0-31 | Phase 1, retry | stop and wait for `tx_done` to refill the pool |
| 32-511, with `in_flight > 0` | Phase 1, extended | keep waiting; QEMU still holds the buffers |
| 32-511, with `in_flight == 0` | Phase 2 | the QEMU transport is broken; advance `hw_ptr` to keep JUCE running |
| ≥ 512 (`VSND_PHASE1_HARDCAP`, about 340 ms) | Phase 2 | advance `hw_ptr` and drop the period |

The counter clears only when the pool depth reaches `VSND_POOL_HEALTHY = 8`,
so a pool that oscillates around empty still reaches the hard cap.

#### Phase-2 burst and ALSA xrun

After `VSND_XRUN_THRESHOLD = 64` consecutive Phase-2 advances (about 42 ms
of dropped audio), the driver calls `snd_pcm_stop_xrun()`, and JUCE's
`snd_pcm_recover()` resets `appl_ptr` and `hw_ptr` together. Without the
xrun, JUCE detects the drift through repeated "snd over delta" messages and
can spin EP122's HuiProcessor thread.

#### Pipeline-depth watchdog and forced xrun

When `frames_in_flight` stays above 10240 (about 107 ms at 96 kHz) for
8 seconds, the driver forces an xrun and JUCE prepares the stream again. When
10 or more TX buffers are still in flight at prepare time, the prepare path
sends `VIRTIO_SND_R_PCM_RELEASE` to QEMU (see
[the RELEASE handler](#virtio_snd_r_pcm_release-handler)). A 60-second
cooldown limits recovery clicks to one per minute under sustained stress.

### Diagnostics

With `verbose_stats` set to 1 (it is 0 by default), the driver logs this
line every 300 TX submissions (about 3.2 s):

```
virtio_snd: tx=N stall=A(+a) pop=B(+b) xrun=C wdog=W overrun=D(d) gap=E(e)us lat=g+h=Tms inflight_n=N/32
```

| Field | Meaning |
|---|---|
| `tx` | Total TX buffers submitted |
| `stall` | Phase-1 stall events |
| `pop` | Phase-2 silent advances (audible clicks) |
| `xrun` | `snd_pcm_stop_xrun()` calls, from Phase-2 bursts and from the watchdog |
| `wdog` | Watchdog-forced xruns |
| `overrun` | Max `hrtimer_forward_now()` overrun |
| `gap` | Max gap between TX submissions in µs (host preemption windows) |
| `lat=g+h=T` | Guest pipeline, host pipeline and their total, in ms (not capped) |
| `inflight_n` | TX buffers currently in flight |

In a healthy steady state the line shows `stall=0(+0) pop=0(+0) xrun=0 wdog=0`
and a `lat` total of 50 to 70 ms.

---

## Host side - patches under `qemu/patches/`

The whole host audio path is a set of patches against the pinned upstream
QEMU. `qemu/build.sh` wipes the `qemu/src/` tree, applies the patches in
numerical order and rebuilds `qemu-system-aarch64`.

| Patch | Component |
|---|---|
| `07-coreaudio-bypass.patch` | `audio/coreaudio.m`, `qapi/audio.json` |
| `08-virtio-snd-bypass.patch` | `hw/audio/virtio-snd.c` |
| `09-system-main-qos.patch` | `system/main.c` (Mach RT on QEMU main loop) |
| `10-virtio-blk-removable.patch` | `hw/block/virtio-blk.c` (NULL-guard on shutdown race) |
| `11-console-vc-quiet.patch` | `ui/console-vc.c` (no-op `vt100_update_cursor`) |
| `12-hvf-vcpu-qos.patch` | `accel/hvf/hvf-accel-ops.c` (QoS on VCPU threads) |
| `14-pipewire-bypass.patch` | `audio/pwaudio.c` (bypass drain on Linux) |
| `18-wasapi-bypass.patch` | `audio/wasapiaudio.c` and `qapi/audio.json` (the `wasapi` audiodev, bypass drain on Windows), `hw/audio/virtio-snd.c` (MMCSS writer), thread priority on the main loop and vCPUs |
| `21-virtio-snd-parked-release.patch` | `hw/audio/virtio-snd.c`: a buffer parked before START is released at once (`end_head` 0; the reader takes whole periods and can stop short of the head), and RELEASE flushes the deferred-return list |

(Patches 01-06 are unrelated infrastructure: ivshmem and the shm
display backend used for the LCD framebuffer. 15-17, 19 and 20 are the
same for Windows: file-backed memory, the shm display's file mapping,
`ivshmem-plain`, a fixed-length host disk, and vCPU halt polling.)

### Bypass ring (`08-virtio-snd-bypass.patch`)

- **Size**: 128 KiB (about 170 ms at 96 kHz F32 stereo), enough to absorb
  HAL jitter and short host preemptions of the writer thread.
- **Format in the ring**: F32 stereo, whatever the guest's format. The writer
  converts S32 samples to F32 with a multiply; the resampler already outputs
  F32.
- **Synchronization**: single-producer, single-consumer atomics. The writer
  publishes `head` with `memory_order_release` and the reader loads it with
  `memory_order_acquire`. The reader publishes `tail` with release, and the
  writer loads it with acquire to compute the free space. The hot path takes
  no mutex.
- **Cold-start prefill**: when `bypass_set_active(1)` sees a `src_fmt` that
  differs from the cached one, it resets the ring and loads 16 KiB of zeros
  (about 21 ms at 96 kHz), so the IOProc has silence to play until the writer
  produces audio.
- **Warm re-activation**: when xrun recovery calls `set_active(0)` and then
  `set_active(1)` with the same `src_fmt`, the ring contents are kept, so
  recovery adds no silence to the output.

### Dedicated writer pthread (`08-virtio-snd-bypass.patch`)

`vsnd_writer_thread_fn`:

- The first `bypass_set_active(1)` in the process starts it, and it runs for
  the lifetime of the QEMU process.
- It polls every 1 ms with `nanosleep`.
- It holds the Mach `THREAD_TIME_CONSTRAINT_POLICY` (period 2 ms, computation
  0.5 ms, constraint 1 ms, preemptible). The budget is well above the work
  done per wake (tens of µs), so macOS does not throttle the thread.
- It runs without the BQL. It takes `stream->queue_mutex` briefly to dequeue
  buffers; the BQL thread's `virtio_snd_handle_tx_xfer` holds the same mutex
  only for a `QSIMPLEQ_INSERT_TAIL`.
- It does not call `virtqueue_push` or `virtio_notify`, which need the BQL.
  Completed buffers go on a pending list (`vsnd_bypass_pending`) with its own
  pthread mutex, and the drain timer on the BQL thread returns them.

Main-thread preemption therefore cannot stall the ring refill. Only macOS
preempting the writer thread itself can, and the RT policy guards against
that.

### Resampler (`08-virtio-snd-bypass.patch`)

`vsnd_bypass_write_convert` picks one of two paths from the live HAL rate
and the guest's rate:

**Passthrough** (`hal_rate == guest_rate`, or the HAL rate is not cached
yet): one S32-to-F32 multiply per sample, then a memcpy into the ring. This is
the common case, with the device and the guest both at 96 kHz, and it is
bit-perfect.

**Resampled**: cubic Hermite interpolation (Catmull-Rom tangents) over a
4-sample sliding window per channel. It is C1-continuous at input-frame
boundaries, so it adds no slope-jump aliasing. When it downsamples (input rate
above output rate), a 2nd-order Butterworth lowpass biquad with its cutoff at
0.46 × the output rate runs before the interpolation, so high frequencies do
not fold into the audible band.

**Soft-PLL clock-skew correction** (resampled path only): a small drift
between the guest's audio clock and the device's clock would slowly fill or
drain the ring. An integrator-only PLL accumulates the error between
the ring depth and an 80 ms target and adjusts the resampler ratio by at most
±0.5%. Each call moves the integrator by only 1e-6 × the normalized error, so
the change is inaudible, but over seconds it locks the writer's output rate to
the reader's clock. This avoids both the clicks of dropped frames and the
cycle of a filling ring followed by a watchdog xrun. The passthrough path has
no correction, so drift there ends in a watchdog xrun.

The resampler state (sliding window, fractional phase, biquad state and PLL
integrator) is reset only on a cold start, when the format changes. It carries
across the guest's xrun-recovery prepares, so the phase does not restart on
each recovery.

### IOProc bypass fast-path (`07-coreaudio-bypass.patch`)

`out_device_ioproc`:

- The bypass check comes **before** any mutex. While the bypass is active,
  the IOProc never takes `buf_mutex`: the SPSC ring read is lock-free and the
  pipeline-depth `mHostTime` update is one atomic store. The IOProc therefore
  never blocks on `buf_mutex` while QEMU's audio frontend holds it on the main
  thread, which would cause pops.
- Every early return **fills `out` with zeros** first. CoreAudio owns the
  output buffer and does not clear it between calls, so returning without
  writing would make the DAC play stale data from a previous call. The worst
  case is therefore silence, and the underrun counter counts every silent
  frame.

### HAL samplerate cache (`07-coreaudio-bypass.patch`)

`coreaudio_get_active_samplerate()` is an atomic load of
`g_coreaudio_cached_samplerate`, so the hot path reads no HAL property.
`init_out_device` seeds the cache, and a listener on the bound device's
`kAudioDevicePropertyNominalSampleRate` refreshes it. Do not read the rate
with `AudioObjectGetPropertyData` on each guest config read: under contention
the call takes several milliseconds and stalls the BQL thread.

### Per-instance device binding (`07-coreaudio-bypass.patch`)

The QAPI extension adds `device-uid` to
`AudiodevCoreaudioPerDirectionOptions`. When the user picks a device in
**Audio > Output Device**, `QemuConfig` passes it as
`-audiodev coreaudio,id=audio0,out.device-uid=<UID>`.
`coreaudio_get_voice_out` resolves the UID through
`kAudioHardwarePropertyTranslateUIDToDevice` and falls back to the system
default output, so each instance can play to a different DAC.

### HAL property listeners (`07-coreaudio-bypass.patch`)

Two listeners are installed:

- `kAudioDevicePropertyNominalSampleRate` on the bound device refreshes the
  cached sample rate and leaves the bypass active. The writer's resampler uses
  the new rate on its next call, so when another app changes the device rate,
  the output keeps playing through the bypass.
- `kAudioHardwarePropertyDefaultOutputDevice` (system-wide) invalidates the
  bypass when the default output changes, and runs `fini_out_device` and
  `init_out_device` against the new device. The next guest PCM prepare
  activates the bypass on it.

### `VIRTIO_SND_R_PCM_RELEASE` handler

The guest sends `RELEASE` from its prepare path when 10 or more TX buffers
are still in flight. The handler:

- Returns every TX buffer still in `stream->queue`, as the virtio spec
  requires.
- Calls `vsnd_bypass_pending_flush`, which returns the deferred buffers so
  that the guest's `frames_in_flight` drops back to 0.
- **Leaves the bypass ring contents intact.** The IOProc keeps playing the
  buffered audio while the writer fills in the guest's recovered stream
  behind it. The step from old to new content is at worst a small position
  jump, typically under 0.4 beats.

### Realtime scheduling

- **QEMU main loop** (`09-system-main-qos.patch`):
  `THREAD_TIME_CONSTRAINT_POLICY` with period 10 ms, computation 1 ms and
  constraint 5 ms, so the drain timer keeps firing under host load.
- **HVF vCPU threads** (`12-hvf-vcpu-qos.patch`): `QOS_CLASS_USER_INTERACTIVE`,
  so the host preempts them less under load. A time-constraint policy does
  not fit vCPU threads: they run for variable-length spans inside
  `hv_vcpu_run`, which breaks the periodic model the policy assumes, and
  macOS then throttles the thread.
- **Bypass writer thread** (`08-virtio-snd-bypass.patch`):
  `THREAD_TIME_CONSTRAINT_POLICY` with period 2 ms, computation 0.5 ms and
  constraint 1 ms, well above its work of tens of µs per wake.

These shorten macOS preemption windows but cannot remove them.

### Telemetry (`08-virtio-snd-bypass.patch`)

Lock-free atomic counters:

- `vsnd_bypass_underrun_frames`: each IOProc call that finds the ring empty
  adds `frame_size`, one IOProc buffer at the HAL rate.
- `vsnd_bypass_overrun_count`: counts each writer attempt that finds the ring
  too full for the next batch.

A logger on the drain timer writes **at most one stderr line per second**,
and only when a counter has changed:

```
virtio-snd: xrun report: underrun frames=N (~U us of silence in the last second), overrun count=M (writes deferred because the ring was full)
```

A clean run logs nothing. The totals are exported through the C symbols
`virtio_snd_bypass_underrun_total_frames` and
`virtio_snd_bypass_overrun_total`, which other code can weak-import.

---

## Live audio latency

The latency is exposed in one read-only sysfs file:

```
/sys/module/virtio_snd/parameters/audio_latency_ms
```

It reads `guest_ms,host_ms,total_ms`, for example `21,30,51`. Only the total
is capped, at 200 ms. It reads `0` while no device is bound.

| Component | Source |
|---|---|
| Guest `frames_in_flight` | incremented on submit and decremented in `tx_done` in `virtio_snd.c` |
| Host `pipeline_extra_frames` | HAL `mHostTime` and the static device latency, read from the QEMU config space |

The pipeline-depth watchdog (see above) acts on the guest's
`frames_in_flight`, not on this total. ALC reads the capped total; see
`docs/alc.md`.

---

## Pro DJ Link sync

The driver exposes two sysfs parameters for the sync feature:

- `audio_latency_ms` (read-only, total capped at 200 ms) is the live
  compensation amount.
- `audio_sync_enabled` (writable) is the master switch; `0` turns both shims
  off.

`docs/alc.md` describes the rest: the LD_PRELOAD shims that shift the
`OptFstUdpServer` clock (slave) or delay Pro DJ Link broadcasts (master), the
`link_pos_offset_ms` override, the push from the runtime worker at each boot,
and the menu toggle.

---

## Files

| Path | Role |
|---|---|
| `guest/modules/virtio_snd/virtio_snd.c` | Kernel driver: timing, pool, defenses, watchdog, prepare flush, sysfs |
| `qemu/patches/07-coreaudio-bypass.patch` | Host: lock-free IOProc path, sample-rate cache, UID binding, listeners, QAPI extension |
| `qemu/patches/08-virtio-snd-bypass.patch` | Host: bypass ring, writer thread, resampler, soft PLL, telemetry |
| `qemu/patches/09-system-main-qos.patch` | Host: Mach RT policy on the QEMU main loop |
| `qemu/patches/11-console-vc-quiet.patch` | Host: less BQL time (no-op `vt100_update_cursor`) |
| `qemu/patches/12-hvf-vcpu-qos.patch` | Host: `QOS_CLASS_USER_INTERACTIVE` on HVF vCPU threads |
| `qemu/patches/14-pipewire-bypass.patch` | Host: the bypass ring drained from PipeWire's process callback |
| `qemu/patches/18-wasapi-bypass.patch` | Host: the `wasapi` audiodev, draining the bypass ring from its MMCSS render thread |
| `crates/cdj3k-emu-platform/src/host/{macos,linux,windows}.rs` | `-audiodev` driver, options and device selector per host |
| `crates/cdj3k-emu-platform/src/audio/{coreaudio,pipewire,windows}.rs` | Output device enumeration for the UI picker |
| `crates/cdj3k-emu-platform/src/menu/service.rs` | **Audio > Output Device** submenu, refreshed every 5 s |
| `crates/cdj3k-emu-storage/src/settings/instance.rs` | Per-instance `audio_device_uid` persistence |

---

## Diagnostic protocol

```bash
# Live latency: guest_ms,host_ms,total_ms
ssh root@<guest> 'while :; do cat /sys/module/virtio_snd/parameters/audio_latency_ms; sleep 1; done'

# Guest driver counters, logged every 3.2 s once verbose_stats is on
ssh root@<guest> 'echo 1 > /sys/module/virtio_snd/parameters/verbose_stats; dmesg -w | grep virtio_snd:'

# Host xrun report: nothing on a clean run, at most one line per second
# otherwise. It goes to the QEMU process's stderr, which the app captures.

# Which path the writer takes:
# - With the device and the guest both at 96 kHz, it takes the passthrough path.
# - With different rates it resamples, and the PLL integrator moves.
# - With the bypass turned off it uses AUD_write and logs "the host audio bypass is off" to stderr.
```

A healthy guest line in steady state:

```
virtio_snd: tx=N stall=0(+0) pop=0(+0) xrun=0 wdog=0 overrun=K(1-3) gap=K(11000±500)us lat=21+30=51ms inflight_n=5/32
```

A healthy host logs no xrun report: the logger stays silent while the
underrun and overrun deltas are 0.

Signs of stress or preemption:

| Symptom | Source | Mitigation |
|---|---|---|
| `gap=2000000us` in the guest log | macOS preempted the HVF vCPU thread | `12-hvf-vcpu-qos.patch` raises its priority; beyond that, reduce host load |
| `wdog` rising every ~60 s | Clock skew filled the ring and the watchdog forced a recovery | When the guest and HAL rates differ, the soft PLL in `08-virtio-snd-bypass.patch` holds the depth at 80 ms; check that the patch is applied. At matching rates the passthrough path has no correction |
| `underrun frames > 0` in the host xrun report | The ring ran empty | The writer thread was preempted; check `12-hvf-vcpu-qos.patch` and host CPU load |
| `overrun count > 0`, sustained | The writer outpaces the reader | The soft PLL is not pulling the depth down fast enough; compare the guest and HAL rates |
| Pops on music but not on silence | Guest-side DSP (JUCE, Pioneer firmware) | Out of scope; the sine test below shows that the host pipeline is bit-perfect |
| Pops on a pure sine | Host pipeline or HAL | Compare with a stress test; isolate with the sine test |

### Bit-perfect verification

To check that the host pipeline is bit-perfect, pipe a sine generated on the
host through `aplay` in the guest, without the Pioneer firmware:

```bash
# 1. Free virtio-snd on the guest
ssh root@<guest> 'pkill -9 EP122; sleep 1'

# 2. Generate a 1 kHz S32 stereo sine at 96 kHz on the macOS host and
#    pipe it through ssh to aplay
python3 -c '
import struct, math, sys
r, f, amp = 96000, 1000, 0x40000000
for n in range(r * 30):
    v = int(amp * math.sin(2*math.pi*f*n/r))
    sys.stdout.buffer.write(struct.pack("<ii", v, v))
' | ssh root@<guest> 'aplay -f S32_LE -c 2 -r 96000 -D hw:0'
```

If a recording of the output shows a clean horizontal line in a spectrogram,
with no vertical disturbances, the host pipeline is bit-perfect at matched
rates. Any pops in the recording come from after the IOProc (HAL or DAC).

To bring EP122 back after the test, restart the emulation from the app.

---

## Known limitations

- **JUCE and Pioneer firmware DSP**: the firmware produces sample-level
  micro-pops on loud transients even though the transport is bit-perfect (see
  the sine test above). The bypass passes them through unchanged.
- **macOS HAL preemption under heavy host load**: when every CPU core is
  saturated, threads with raised RT or QoS priority can miss deadlines.
  macOS manages CoreAudio's HAL IO thread itself, and it may emit broadband
  impulses on the DAC stream during such events.
- **Clock skew bounds**: the soft-PLL integrator is clamped to ±0.5%.
  Consumer DACs typically drift 10-50 ppm (0.001-0.005%) from the host
  crystal, well within the clamp. Drift beyond about 5000 ppm would saturate
  the PLL, and the ring would slowly fill or empty over many minutes.
- **48 kHz downsample**: with the macOS device at 48 kHz and the guest at
  96 kHz, the 2:1 downsample and its Butterworth pre-filter still fold some
  content near Nyquist into the audible band. Setting the device to 96 kHz in
  Audio MIDI Setup selects the passthrough path and removes the downsample.

---

## Linux (PipeWire)

The guest side and patch 08's bypass ring are the same; the host end differs.

| | macOS | Linux |
|---|---|---|
| `-audiodev` | `coreaudio,out.buffer-length=5000,out.device-uid=<UID>` | `pipewire,out.fixed-settings=off,out.name=<node>` |
| Ring drained by | the CoreAudio IOProc (patch 07) | the stream's process callback (patch 14) |
| Stream rate | the device's; the bypass writer resamples | the guest's (96 kHz); PipeWire resamples once to the device |
| Real-time threads | Mach time-constraint policy, QoS (patches 09, 12) | PipeWire's own data thread |
| Device list | CoreAudio HAL | `pw-dump` |

Patch 14 plays all or nothing per quantum, as the IOProc does. After an
underrun it waits for the ring to refill to a 20 ms cushion
(`PW_BYPASS_CUSHION_MS`) before playing again, and gives up on the cushion
after 50 silent callbacks (`PW_BYPASS_STARVED_MAX`) for a guest whose buffer is
smaller. The reported pipeline depth is PipeWire's graph delay plus what the
stream's resampler holds.

Under software emulation the guest runs a 512-frame ALSA period
(`cdj3k.snd_period=512`, read by `insmod-virtio-snd`) instead of the player's
64; each TX buffer stays 1024 frames, so every threshold counted in periods
keeps its duration.

---

## Windows (WASAPI)

The guest side and patch 08's bypass ring are the same. Patch 18 adds a
`wasapi` audiodev, which upstream QEMU lacks. It is a shared-mode,
event-driven render client with one thread per output voice, and that thread
owns every COM object of its stream.

| | macOS | Windows |
|---|---|---|
| `-audiodev` | `coreaudio,out.buffer-length=5000,out.device-uid=<UID>` | `wasapi,out.dev=<endpoint id>` (the `IMMDevice::GetId` string; without it, the default render endpoint) |
| Ring drained by | the CoreAudio IOProc (patch 07) | the voice's render thread, woken by the stream's event (patch 18) |
| Stream rate | the device's; the bypass writer resamples | the endpoint's mix format; see below |
| Real-time threads | Mach time-constraint policy, QoS (patches 09, 12) | MMCSS "Pro Audio" at `AVRT_PRIORITY_HIGH` for the render thread and the bypass writer; above-normal priority with power throttling off for the main loop and the TCG or WHPX vCPU threads |
| Device list | CoreAudio HAL | `IMMDeviceEnumerator`, active render endpoints |

The stream opens through `IAudioClient3::InitializeSharedAudioStream` at the
engine's minimum period, in the endpoint's mix format, and falls back to
`IAudioClient::Initialize` at the default period. When the mix format is one
the converter does not produce, or the endpoint reopens at a rate other than
the one the voice first reported, the stream is stereo F32 at the voice's rate
(the mix rate on the first open) with `AUTOCONVERTPCM` and the engine's
resampler, so the ring's rate never changes under the bypass writer's
resampler. An invalidated endpoint is
reopened; a missing preferred endpoint falls back to the default one. While no
endpoint is open the ring is still drained, so the guest's clock keeps
running.

Each wake fills the free part of the endpoint buffer, and the bypass read is
all or nothing per fill, as the IOProc's is. After an underrun the ring
refills to a 20 ms cushion (`WASAPI_BYPASS_CUSHION_MS`) before play resumes,
and gives up on the cushion after 50 silent fills
(`WASAPI_BYPASS_STARVED_MAX`). The reported pipeline depth is the endpoint
buffer's padding when the last period was filled; the static latency is
`GetStreamLatency` plus one period.

The bypass writer ticks on a high-resolution waitable timer: `Sleep(1)` rounds
up to the system timer tick, 15.6 ms by default.
