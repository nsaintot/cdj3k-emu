# cdj3k-emu - ALC (Audio Latency Compensation)

> This page describes how ALC aligns the audible playback of Pro DJ Link
> masters and slaves across cdj3k-emu instances. It extends
> `docs/audio-stack.md`, which covers the audio pipeline; this page covers
> only the sync settings.

---

## What ALC is

Pro DJ Link slaves time their playback against the beat timeline that the
master broadcasts. An emulated CDJ's audio reaches the speakers about 50 ms
(in steady state; see `docs/audio-stack.md`) after the firmware's beat
position, because it passes through QEMU, virtio_snd, the bypass ring and
CoreAudio. Without compensation, two emulator instances synced to each other
keep a steady offset of about 50 ms between their audible beats: the phase
locks at the firmware level but not at the audible level.

ALC shifts either the local clock (slave) or the outgoing packets (master) by
the live pipeline depth, so the audible beat matches the firmware's beat.

## Demo

![ALC demo](./alc.gif)

---

## The two shims

Both shims are part of `deck_shim.so` and share `audio_sync_enabled` as
their master switch (see [Master switch](#master-switch)).

### Slave mode - `guest/deck_shim/core/clock.c`

The slave shim hooks `clock_gettime` and `gettimeofday` through LD_PRELOAD
and shifts only the `OptFstUdpServer` thread, the thread whose clock moves
slave playback. Every other thread (JUCE ALSA, render, HID, …) returns early
after one lookup that classifies it by name.

- The shim subtracts the latency from that thread's view of
  `CLOCK_MONOTONIC`, `CLOCK_MONOTONIC_RAW` and `CLOCK_REALTIME`.
- The thread then projects the master's broadcast position further ahead, the
  deck plays earlier internally, and the audio pipeline absorbs exactly that
  offset. The deck's audible playback therefore aligns with the master's.
- The shim refreshes the offset at most once per wall-clock second, using the
  timestamp the hooked call returns, so the hot path makes no extra clock
  call.

| sysfs parameter | Role |
|---|---|
| `audio_sync_enabled` | Master switch; `0` turns the shim off |
| `audio_latency_ms` | Live pipeline depth, the default offset |
| `link_pos_offset_ms` | Manual override; a positive value forces a fixed offset |

### Master mode - `guest/deck_shim/core/link.c`

The master shim hooks `sendto` and `sendmsg` through LD_PRELOAD and delays
every outgoing Pro DJ Link broadcast (UDP ports 50001 and 50002, magic header
`Qspt1WmJOL`) by the latency.

- The shim uses a 512-slot ring buffer and a dedicated worker thread.
  `sendto` and `sendmsg` enqueue the socket, destination, flags, payload and
  deadline. The worker takes the oldest entry, sleeps until its deadline
  (`clock_nanosleep` with `TIMER_ABSTIME`) and sends it with a raw
  `SYS_sendto`.
- 512 slots leave headroom: with ABS_POS sent every 3 ms and a delay of about
  100 ms, about 33 packets are in flight in the common case.
- The shim delays whole packets because Pioneer slaves use the packet arrival
  time as the beat-phase reference. Delaying the whole stream keeps ABS_POS
  (waveform), BEAT (beat sync) and PLAYER_STATUS (phase, BPM) consistent.

### Deferred close - `guest/deck_shim/core/link.c` and `syscalls.c`

EP122 opens a socket, calls `sendto()` and closes the socket for each
broadcast. Without a deferred close, EP122 would close the descriptor before
the worker sends the delayed packet.

- `shim_link_intercept_close(fd)`, defined in `link.c` and called from the
  `close()` hook in `syscalls.c`, scans the pending queue under `g_q_mutex`
  for an entry that holds `fd`.
- If it finds one, it sets `p->app_wants_close = 1` and returns `1`. The
  `close()` hook then reports success to EP122 without closing the
  descriptor.
- The worker closes the descriptor after its `sendto`, in the same critical
  section that advances the queue tail.
- If `close()` arrives after the send, the scan finds nothing and the real
  close runs.

---

## Master switch

```
/sys/module/virtio_snd/parameters/audio_sync_enabled
```

| Value | Effect |
|---|---|
| `0` | Both shims are off: `sendto` and `sendmsg` pass through, the clock shim returns early, and nothing is queued. |
| `1` | Both shims are on. |

**The kernel module sets it to 0 at every load**, so the runtime worker sets
it after every boot.

### Push from the host at each boot

`app/cdj3k-emu/src/runtime_worker.rs`:

- `push_alc_once` sends `set audio_sync_enabled <0|1>` with the saved setting
  once per QEMU lifetime. It waits for the first `latency` push (see
  [Live latency source](#live-latency-source)), which shows that cfgd is
  responsive: cfgd sends it only after its handshake on the virtio-serial
  port. `alc_pushed_for_boot` records that the push was made.
- `set_alc` handles the menu toggle: it pushes the new value at once and
  saves it.

No QEMU restart is needed: the sysfs write takes effect immediately, and both
shims pick up the change at their next sysfs refresh, at most 1 s later.

### cfgd whitelist - `guest/cfgd/cfgd.c`

| Parameter | Access |
|---|---|
| `audio_sync_enabled` | read and write |
| `link_pos_offset_ms` | read and write |
| `audio_latency_ms` | read only; also pushed every 3 s |

cfgd answers `param <name> ?` for any other name, so the cfg channel cannot
reach other kernel parameters.

---

## Live latency source

```
/sys/module/virtio_snd/parameters/audio_latency_ms        (read-only)
```

The file reads `guest_ms,host_ms,total_ms`, for example `21,30,51`. The total
is capped at 200 ms; the guest and host values are not.

| Component | Source |
|---|---|
| guest, from `frames_in_flight` | incremented on submit and decremented in `tx_done` in `guest/modules/virtio_snd/virtio_snd.c` |
| host, from `pipeline_extra_frames` | HAL `mHostTime` and the static device latency, exported by the QEMU bypass patch |

cfgd pushes `latency <guest>,<host>,<total>` on `cfg.sock` every 3 s
(`push_latency` in `guest/cfgd/cfgd.c`), and the app shows it as the
**Audio > Pipeline latency** readout without polling.

Both shims read `audio_latency_ms` from sysfs themselves, at most once per
wall-clock second, and use its last field, the total. The cfg push only feeds
the menu.

---

## Manual override

```
/sys/module/virtio_snd/parameters/link_pos_offset_ms
```

The parameter is writable and defaults to 0. When it is positive, **both
shims** use it in place of `audio_latency_ms`. Uses:

- A/B tests of the compensation.
- A fixed external setup, such as a real CDJ master with a known DAC chain.
- A fixed offset while measuring latency.

Set it to 0 to follow `audio_latency_ms` again. Writes from the host go
through `cfg.sock` and the cfgd whitelist.

---

## UI surface

| Layer | File | Notes |
|---|---|---|
| Row | `crates/cdj3k-emu-platform/src/menu/service.rs` | `MenuNode::check(MenuId::Alc, "Enable ALC (Experimental)", snap.alc_enabled)`, rebuilt from state on every sync |
| Container | `crates/cdj3k-emu-platform/src/menu/service.rs` | Audio submenu, in order: Pipeline latency, Enable Audio, Enable ALC (Experimental), Output Device |
| Click handler | `crates/cdj3k-emu-platform/src/menu/action.rs` | Toggles `alc_enabled` and sets `alc_toggle_requested` |
| State mirror | `crates/cdj3k-emu-platform/src/menu_state.rs` | `alc_enabled` and `alc_toggle_requested` |
| Persistence | `crates/cdj3k-emu-storage/src/settings/instance.rs` | `InstanceSettings::alc_enabled`, key `alc_enabled` in the per-instance `settings.txt` |
| Default | `crates/cdj3k-emu-storage/src/settings/instance.rs` | `flag(&map, "alc_enabled", true)`: new instances have ALC on |
| Worker dispatch | `app/cdj3k-emu/src/runtime_worker.rs` | `set_alc` pushes `audio_sync_enabled` and saves the setting when `req.alc_toggle` is set |

ALC is on by default. Turning it off is saved per instance.

---

## Cap and watchdog

- The shims clamp the offset at 5000 ms (`clock.c`, `link.c`).
- The kernel module caps the `audio_latency_ms` total at 200 ms. The cap
  breaks a feedback loop in slave mode: a larger shift makes the deck play
  sooner, which puts more buffers in flight and raises the latency, which
  grows the shift again.
- When the guest's `frames_in_flight` stays above 10240 (about 107 ms at
  96 kHz) for 8 s, the pipeline-depth watchdog in
  `guest/modules/virtio_snd/virtio_snd.c` forces an xrun. JUCE prepares the
  stream again, the prepare path sends `VIRTIO_SND_R_PCM_RELEASE` to the host,
  and the backlog drains. See
  [Pipeline-depth watchdog](audio-stack.md#pipeline-depth-watchdog-and-forced-xrun)
  for the full recovery sequence.

---

## When ALC matters / when it doesn't

| Scenario | Recommendation |
|---|---|
| Single instance, no sync | ALC has no audible effect; leave it on (the default). |
| Two emulator instances synced to each other | Turn ALC on for both; their audible beats then align. |
| Emulator slaved to a real CDJ over a bridged network | Turn ALC on in the emulator. The real CDJ master's audio is the reference, and the emulated slave has to land on it. |
| Real CDJ slaved to an emulator master | Turn ALC on in the emulator master. The delayed packets line its broadcasts up with its own audio; the real CDJ's pipeline latency is close to 0 and predictable, so the slave plays in time. |
| Two emulator instances **not** synced over Pro DJ Link | ALC has nothing to delay on the network; either setting works. |

ALC needs bridged networking to send and receive Pro DJ Link broadcasts. See
`docs/network.md` for the interface choice and the tap and vmnet setup.

---

## Files

| Path | Role |
|---|---|
| `guest/deck_shim/core/clock.c` | LD_PRELOAD slave-mode clock shift on `OptFstUdpServer` |
| `guest/deck_shim/core/link.c` | LD_PRELOAD master-mode delayed send (`sendto`, `sendmsg`) and the deferred-close handler |
| `guest/deck_shim/core/syscalls.c` | `close()` hook; calls `shim_link_intercept_close` |
| `guest/cfgd/cfgd.c` | Guest config daemon: parameter whitelist, 3 s latency push, `set` and `get` dispatch |
| `guest/modules/virtio_snd/virtio_snd.c` | Kernel driver: exposes `audio_sync_enabled`, `audio_latency_ms`, `link_pos_offset_ms` |
| `crates/cdj3k-emu-runtime/src/cfg.rs` | `CfgClient`, the host side of `cfg.sock` (line protocol, latency mirror) |
| `crates/cdj3k-emu-storage/src/settings/instance.rs` | `InstanceSettings::alc_enabled` (default true) |
| `crates/cdj3k-emu-platform/src/menu/{service,action}.rs` | "Enable ALC (Experimental)" row, click dispatch |
| `crates/cdj3k-emu-platform/src/menu_state.rs` | `alc_enabled` and `alc_toggle_requested` |
| `app/cdj3k-emu/src/runtime_worker.rs` | ALC push at each boot (`push_alc_once`), toggle handler (`set_alc`) |
| `docs/audio-stack.md` | Audio pipeline, watchdog, telemetry |

---

## Diagnostics

```bash
# Master switch
ssh root@<guest> 'cat /sys/module/virtio_snd/parameters/audio_sync_enabled'

# Current compensation: guest_ms,host_ms,total_ms
ssh root@<guest> 'cat /sys/module/virtio_snd/parameters/audio_latency_ms'

# Pin the offset to 50 ms
ssh root@<guest> 'echo 50 > /sys/module/virtio_snd/parameters/link_pos_offset_ms'

# Watch the cfg push on the host
nc -U /tmp/cdj3k-emu-$(id -u)/instance-1/cfg.sock   # prints lines such as `latency 21,30,51`

# Shim debug logging (set in EP122's environment before it starts)
DECK_TIME_SHIFT_DEBUG=1   # slave clock shim
DECK_LINK_DEBUG=1         # master delay-send shim
```
