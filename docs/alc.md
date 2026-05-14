# cdj3k-emu - ALC (Audio Latency Compensation)

> Reference for the user-facing wiring that aligns Pro DJ Link
> master/slave audible playback across cdj3k-emu instances. Extends
> `docs/audio-stack.md` - the deep audio-pipeline mechanics live there,
> this doc only covers the sync surface.

---

## What ALC is

Pro DJ Link slaves time their playback against the master's broadcast
beat timeline. Naively, an emulated CDJ's audio is delayed by the
QEMU + virtio_snd + bypass-ring + CoreAudio pipeline (roughly 50 ms in
steady state, see `docs/audio-stack.md` for the breakdown) relative to
where the firmware "thinks" the beat is. Without compensation, two
emulator instances syncing to each other have a steady ~50 ms phase
offset between their audible beats - phase locks at the firmware-tick
level, drifts at the audible level.

ALC removes this. It shifts either the local clock (slave) or the
outbound packet stream (master) by the live measured pipeline depth,
so audible-event timing matches firmware-event timing.

## Demo

![ALC demo](./alc.gif)

---

## The two shims

Both ship in the EP122 `LD_PRELOAD` and share `audio_sync_enabled` as
their single master switch (see "Master switch" below).

### Slave mode - `guest/ep122_shim/clock.c`

LD_PRELOAD on `clock_gettime` / `gettimeofday`. Targets the
`OptFstUdpServer` thread only - empirically the thread whose libc
clock-shift moves slave playback. Every other thread (JUCE ALSA,
render, HID, …) hits an early return and pays a single thread-name
classify lookup once.

Subtracts `audio_latency_ms` from the target thread's view of
`CLOCK_MONOTONIC` / `CLOCK_MONOTONIC_RAW` / `CLOCK_REALTIME`. That
thread projects master's broadcast position further along, the deck
plays earlier internally, and the audio pipeline consumes exactly that
compensation. Net effect: audible play aligns with master's audible.

Refresh cadence: at most once per wall-clock second using the
syscall's own returned timestamp (no extra clock call on the hot
path).

| Surface | Source |
|---|---|
| `audio_sync_enabled` | Master switch - 0 makes the shim fully inert |
| `audio_latency_ms` | Auto live tracking (default source) |
| `link_pos_offset_ms` | Manual override - non-zero forces a fixed value |

### Master mode - `guest/ep122_shim/link.c`

LD_PRELOAD on `sendto` / `sendmsg`. Delays every outbound Pro DJ Link
broadcast (UDP ports 50001 / 50002, magic header `Qspt1WmJOL`) by
`audio_latency_ms`.

Architecture: a 512-slot ring buffer + dedicated worker thread.
`sendto` / `sendmsg` enqueue (sockfd, dest, flags, payload, deadline);
the worker pops the head, `clock_nanosleep`s `TIMER_ABSTIME` to the
deadline, then forwards via raw `SYS_sendto`. The queue is sized at
512 slots - ABS_POS at 3 ms cadence × ~100 ms delay = ~33 in flight in
the common case, with headroom.

**Why delay-send (not packet rewrite):** Pioneer slaves use packet
*arrival time* as the implicit beat-phase reference. ABS_POS bytes
alone wouldn't be enough - the receipt event itself must coincide
with the master's audible beat. Delaying the entire packet stream
aligns ABS_POS (waveform), BEAT (beat-sync), and PLAYER_STATUS
(phase / BPM) consistently with one mechanism.

### Defer-close - `guest/ep122_shim/link.c` + `syscalls.c`

EP122 does `socket() → sendto() → close()` for each broadcast - a
socket-per-packet pattern that breaks any naive delay-send: by the
time the worker fires, EP122 has already closed the FD.

`ep122_link_intercept_close(fd)` (declared in `link.c`, called from
the `close()` hook in `guest/ep122_shim/syscalls.c:248`) scans the
pending queue under `g_q_mutex` for any entry holding `fd`. If found,
it marks `p->ep122_wants_close = 1` and returns `1`. The `close()`
hook reports success to EP122 *without* invoking `sys_close`. The
worker performs the real close after `sendto` completes, in the same
critical section that advances the queue tail (`link.c:161`), so a
late-arriving `close()` either wins (flag set in time) or loses
harmlessly (scan misses, real close happens, our late close gets
EBADF).

---

## Master switch

```
/sys/module/virtio_snd/parameters/audio_sync_enabled
```

`0` = both shims fully inert (`sendto`/`sendmsg` pass through, clock
shim early-returns, no queue activity).  
`1` = both shims active.

**The kernel module defaults to 0 on every load.** This is the trap
the runtime worker exists to plaster over.

### Per-boot push from the host

`app/cdj3k-emu/src/runtime_worker.rs:60` declares
`alc_pushed_for_boot = false`. On every QEMU boot path

```rust
if !alc_pushed_for_boot && cfg_client.latency().is_some() {
    let v = if req.alc_enabled { "1" } else { "0" };
    cfg_client.set_param("audio_sync_enabled", v) // ...
    alc_pushed_for_boot = true;
}
```

We use the first incoming `latency` push (see "Live latency source"
below) as the "cfgd is responsive" signal - that line only arrives
after the daemon has handshook the virtio-serial port. The push then
fires exactly once per QEMU lifetime.

Interactive toggles re-push immediately. `runtime_worker.rs:203`:

```rust
if req.alc_toggle {
    let v = if req.alc_enabled { "1" } else { "0" };
    cfg_client.set_param("audio_sync_enabled", v) // ...
    persist_inst(config.instance_id, |s| s.alc_enabled = req.alc_enabled);
}
```

No QEMU restart is needed - sysfs takes effect immediately, both
shims pick up the change on their next 1 s sysfs refresh tick.

### cfgd whitelist - `guest/cfgd/cfgd.c:68`

```c
static const struct param_def PARAMS[] = {
    { "audio_sync_enabled",  1 },   // writable
    { "link_pos_offset_ms",  1 },   // writable
    { "audio_latency_ms",    0 },   // read-only - also pushed every 3 s
};
```

Anything outside this whitelist gets `param <name> ?` back. A host
bug can't poke arbitrary kernel knobs through the cfg channel.

---

## Live latency source

```
/sys/module/virtio_snd/parameters/audio_latency_ms        (read-only)
```

Format: `T (guest=G host=H rate=R)` - headline `T` capped at 200 ms,
breakdown shows raw guest + host components.

| Component | Source |
|---|---|
| `guest` = `frames_in_flight` | `submit++` / `tx_done--` in `guest/modules/virtio_snd/virtio_snd.c` |
| `host` = `pipeline_extra_frames` | HAL `mHostTime` + static device latency, exported by the QEMU bypass patch |

cfgd pushes `latency <g>,<h>,<t>` on `cfg.sock` every 3 s
(`guest/cfgd/cfgd.c:53`, `push_latency` at `:295`), so the host UI's
"Current Latency" label updates without polling.

Both shims read `audio_latency_ms` directly from sysfs (not via the
cfg push) at most once per wall-clock second. The cfg push is purely
for the UI label.

---

## Manual override

```
/sys/module/virtio_snd/parameters/link_pos_offset_ms
```

Writable, defaults to 0. When non-zero, **both shims** use the fixed
override instead of auto-tracking `audio_latency_ms`. Useful for:

- A/B testing the compensation curve.
- Targeting a known-fixed external setup (real CDJ master with a
  known DAC chain).
- Pinning the offset during latency measurement experiments.

Set to 0 to return to auto-tracking. Same cfgd whitelist as the master
switch - writes go through `cfg.sock`.

---

## UI surface

| Layer | File:line | Notes |
|---|---|---|
| `CheckMenuItem` | `crates/cdj3k-emu-platform/src/menu.rs:137` | `with_id("alc", "Enable ALC (Experimental)", true, false, None)` - initial checked state is overwritten on first refresh |
| Container | `crates/cdj3k-emu-platform/src/menu.rs:146` | Audio submenu: Current Latency → Enable Audio → Enable ALC → Output Device |
| Click handler | `crates/cdj3k-emu-platform/src/menu.rs:426` | Toggles `alc_enabled`, sets `alc_toggle_requested = true` |
| State mirror | `crates/cdj3k-emu-platform/src/menu_state.rs:128` | `pub alc_enabled: bool` + `alc_toggle_requested: bool` |
| Check-state sync | `crates/cdj3k-emu-platform/src/menu.rs:308` | `alc_item.set_checked(snap.alc_enabled)` on each menu refresh |
| Persistence | `crates/cdj3k-emu-storage/src/settings.rs:108` | `InstanceSettings::alc_enabled`, key `alc_enabled` in per-instance `settings.txt` |
| Default | `crates/cdj3k-emu-storage/src/settings.rs:163` | `.unwrap_or(true)` - new instances get ALC on |
| Worker dispatch | `app/cdj3k-emu/src/runtime_worker.rs:198` | `req.alc_toggle` → `cfg_client.set_param("audio_sync_enabled", …)` + persist |

**Default is ON.** Sync compensation is the better experience for the
vast majority of users; opting out is a power-user choice that the
toggle persists per instance.

---

## Cap and watchdog

The shims clamp at 5000 ms client-side (`clock.c:93`, `link.c:109`).
The kernel module caps `audio_latency_ms` at 200 ms before exposure.
The 200 ms cap exists to break runaway feedback in slave mode - a
clock-shift that grows without bound feeds back into the very metric
it's compensating against.

When raw guest+host pipeline depth exceeds 200 ms for ≥ 8 s, the
pipeline-depth watchdog in `guest/modules/virtio_snd/virtio_snd.c`
fires a forced xrun. JUCE re-prepares, the host pipeline returns a
`VIRTIO_SND_R_PCM_RELEASE`, the deep backlog drains. See
`docs/audio-stack.md` § "Pipeline-depth watchdog → forced xrun" for
the full recovery dance.

---

## When ALC matters / when it doesn't

| Scenario | Recommendation |
|---|---|
| Solo instance, no syncing | ALC has no audible effect - leave it on (default). |
| Two emulator instances syncing to each other | ALC ON for both - phase-aligned audible. |
| Emulator slaving to a real CDJ over a bridged network | ALC ON on the emulator. Real CDJ master's audible is the reference, emulator's slave needs to land there. |
| Real CDJ slaving to an emulator master | ALC ON on the emulator master. The delayed packet stream lines its broadcasts up with its own audible; real CDJ's pipeline latency is ≈ 0 and predictable, so the slave plays correctly. |
| Two emulator instances **not** synced over Pro DJ Link | ALC inactive on the wire (no broadcasts to delay) - fine either way. |

Requires bridged networking to actually receive/send Pro DJ Link
broadcasts. See `docs/network.md` for the iface selection and tap /
vmnet wiring.

---

## Files

| Path | Role |
|---|---|
| `guest/ep122_shim/clock.c` | LD_PRELOAD slave-mode clock shift on `OptFstUdpServer` |
| `guest/ep122_shim/link.c` | LD_PRELOAD master-mode delay-send (`sendto`/`sendmsg` + defer-close handler) |
| `guest/ep122_shim/syscalls.c` | `close()` hook - calls `ep122_link_intercept_close` |
| `guest/cfgd/cfgd.c` | Guest config daemon: param whitelist, 3 s latency push, `set`/`get` dispatch |
| `guest/modules/virtio_snd/virtio_snd.c` | Kernel driver: exposes `audio_sync_enabled`, `audio_latency_ms`, `link_pos_offset_ms` |
| `crates/cdj3k-emu-runtime/src/cfg.rs` | `CfgClient` - host side of `cfg.sock` (line protocol, latency mirror) |
| `crates/cdj3k-emu-storage/src/settings.rs` | `InstanceSettings::alc_enabled` (default true) |
| `crates/cdj3k-emu-platform/src/menu.rs` | "Enable ALC (Experimental)" `CheckMenuItem`, click dispatch |
| `crates/cdj3k-emu-platform/src/menu_state.rs` | `alc_enabled` / `alc_toggle_requested` state mirror |
| `app/cdj3k-emu/src/runtime_worker.rs` | Per-boot ALC push (`alc_pushed_for_boot` latch), toggle handler |
| `docs/audio-stack.md` | Pipeline mechanics, watchdog, telemetry - the deep audio reference |

---

## Diagnostics

```bash
# Master switch
ssh root@<guest> 'cat /sys/module/virtio_snd/parameters/audio_sync_enabled'

# Current compensation amount
ssh root@<guest> 'cat /sys/module/virtio_snd/parameters/audio_latency_ms'

# Force a manual override (e.g. pin to 50 ms)
ssh root@<guest> 'echo 50 > /sys/module/virtio_snd/parameters/link_pos_offset_ms'

# Watch the cfg push live (host)
nc -U /tmp/cdj3k-emu/instance-1/cfg.sock   # one of: `latency 21,30,51`

# Shim debug (verbose, set before EP122 starts)
EP122_TIME_SHIFT_DEBUG=1   # slave clock shim
EP122_LINK_DEBUG=1         # master delay-send shim
```
