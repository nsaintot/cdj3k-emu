# cdj3k-emu - subucom Protocol

> Reference for the sub-CPU (subucom) SPI protocol the EP122 firmware uses
> to talk to the deck's input hardware, and how cdj3k-emu emulates it
> end-to-end through a virtual char device, a guest forwarder, and a host
> `ctrl.sock` peer.

---

## End-to-end path

```
                                  host (cdj-ui, egui main thread)
                                   │
                                   │ inject() MISO   ▲ take() MOSI
                                   ▼                 │
                              CtrlStream  (ctrl_stream.rs)
                                   │   64-byte frames, no framing header
                                   ▼
                           {sock_dir}/ctrl.sock     (UNIX socket, QEMU side)
                                   │
                                   ▼
                          virtio-serial port "cdj3k.ctrl"  (vportNpM)
─────────────── QEMU / guest boundary ───────────────────────────────
                                   │
                                   ▼
                      subucom_forwarder  (guest/subucom/forwarder.c)
                       two pthread halves, blocking r/w
                       │                      │
              read     │                      │     write
                       ▼                      ▼
         /dev/subucom_ctrl  (read MOSI)   /dev/subucom_ctrl  (write MISO)
                       │                      │
                       │       subucom_virt.ko (kernel module)
                       │                      │
                       ▼                      ▼
              MOSI ring + waitqueue    inject_pending (sticky MISO buf)
                       ▲                      │
                       │  ioctl                │ read()
                       │  MOSI_TRANSFER        │ throttled ~850 Hz
                       │                       ▼
                          /dev/subucom_spi1.0   (EP122 polls here)
                                   │
                                   ▼
                                  EP122
```

Cadence: EP122 polls `/dev/subucom_spi1.0` at ~850 Hz (1176 µs period).
Each direction is 64 B × 850 Hz ≈ 54 KB/s - negligible compared to audio
or video. The forwarder and ctrl.sock are blocking; there is no
framing - the receiver simply reads in 64-byte chunks.

For the `ctrl.sock` virtio-serial transport (host UNIX-socket side,
reconnect/blocking semantics, `host_connected` 0-read behaviour), see
[stream-transports.md](stream-transports.md).

---

## What subucom is

On real CDJ-3000 hardware the **subucom** is the deck's sub-CPU: a
dedicated micro-controller that scans every input (buttons, jog wheel,
rotary encoder, tempo slider, vinyl-speed rotary, direction rocker,
capacitive sensors) and drives every LED on the chassis (mono GPIO LEDs,
IC6003 dim-channel LEDs, hot-cue RGB pads, SOURCE/SD/USB/ON-AIR
indicators).

EP122 talks to it over `ff1d0000.spi` in Mode 3 at 3.2 MHz, CS0,
~850 Hz polling. Each poll is a full-duplex 64-byte exchange: **MOSI**
(EP122 → sub-CPU) carries LED state; **MISO** (sub-CPU → EP122) carries
input state. cdj3k-emu replaces the sub-CPU with software but keeps
EP122 unchanged.

---

## Frame anatomy (conceptual)

Both directions are **64 bytes, fixed length**.

| Bytes | Contents |
|-------|----------|
| `[0..2]` | reserved / zero (frame-type header, always `0x0000`) |
| `[2..5]` | constant magic `0x01 0x04 0x03` |
| `[5..62]` | payload (LED PWM for MOSI, input state for MISO) |
| `[62..64]` | **CRC-16/X-25** over bytes `[2..62]`, stored little-endian |

The CRC is poly `0x8408` (reflected `0x1021`), init `0xFFFF`, final XOR
`0xFFFF`. Implementation: `crates/cdj3k-emu-subucom/src/crc.rs` (Rust)
and `guest/modules/subucom_virt/subucom_virt.c:120` (kernel). The first
two bytes are deliberately outside the CRC window.

### MOSI payload (LED state)

EP122 writes **linear-light PWM bytes**. Three LED classes coexist:

- **Single-bit LEDs** (`LedBit = (byte, mask)`) - PLAY, CUE, MASTER,
  transport, beat-jump, encoder, …
- **Two-bit "step" LEDs** (`StepLedMask`) - navigation (`SOURCE`,
  `BROWSE`, `TAG LIST`, `PLAYLIST`, `SEARCH`, `MENU`) and
  `SLIP`/`QUANTIZE`. `medium` bit alone = dim, `medium|full` = bright,
  decoded to `StepLed::{Off, Medium, Full}`.
- **RGB groups** - eight hot-cue pads at `[12..36]`, plus SD, USB-LIGHT,
  ON-AIR at `[36..45]`, each a raw `(R, G, B)` PWM triple.

The host converts raw PWM → display colour in `mosi_frame.rs:50`
(`led_color`): apply `1/LED_GAMMA` (`2.2`) per channel, then normalise
so the dominant channel reaches `LED_PEAK` (`220`). This preserves hue
at any drive level; `led_drive_factor` is exposed for callers that want
to dim the visual to match drive.

> Canonical LED → byte/mask map: `crates/cdj3k-emu-subucom/src/mosi_frame.rs`.

### MISO payload (input state)

Mostly bitmasks plus a handful of scalar fields. By region:

- **Buttons** `[5..12]` - packed bitmasks, one bit per button
  (PLAY/CUE/SEARCH, HOT A..H, BROWSE/MENU/QUANTIZE, POWER_ON, …).
- **Direction rocker** `b04` - **not a bitmask**, a 2-bit value:
  `1`=REV, `2`=SLIP_REV (momentary), `3`=FWD. Use the `Direction` enum.
- **Rotary encoder** `b14..b16` - 16-bit LE counter.
- **LCD touch** `b16..b20` - two 16-bit LE coords; `(0,0)` = no touch.
- **Tempo slider** `b22..b24` - 16-bit LE, dead-band ~`0x7F50..0x7FD0`.
- **Vinyl-speed rotary** `b24` - 8-bit.
- **Jog wheel** `b26..b31` - position (16-bit LE), velocity (16-bit LE,
  **inverse**: `0xFFFF`=stopped, `0x0000`=max), touch byte
  (`0x00`=none / `0x03`=press / `0x04`=idle baseline / `0x0c`=turning).
- **Capacitive sensors** `b32..b44` - 12 bytes; baseline ≈ `0xD5/0xD6`
  per channel, deviations = finger proximity.
- **Device state** `b12` - `0x80`=power-on, `0x01`=SD-cover-closed.
  High nibble carries the sub-CPU power-off timer (see *forwarder*).

A pre-baked `IDLE_PAYLOAD` (`miso_frame.rs:13`) holds the "nothing
pressed" baseline. `MisoFrame::idle()` clones it; per-gesture setters
(`set_btn`, `set_jog`, `set_touch`, `set_tempo`, `set_vinyl`,
`set_rotary`, `set_direction`, `set_power`) patch individual fields.
`finalize()` stamps the CRC and returns the 64-byte wire frame.

> Canonical map: `crates/cdj3k-emu-subucom/src/miso_frame.rs`.

---

## Emulation layers

### 1. Kernel module `subucom_virt.ko`

`guest/modules/subucom_virt/subucom_virt.c`. Creates two char devices
under a dynamic major:

| Path | Minor | Used by | Purpose |
|------|-------|---------|---------|
| `/dev/subucom_spi1.0` | 0 | EP122 | Real SPI device replacement |
| `/dev/subucom_ctrl`   | 1 | forwarder | Host bridge |

EP122 talks to `subucom_spi1.0` with the original SPI ioctls - the
module accepts and remembers `TIMER_STATUS`, `BITS_PER_WORD`,
`RX_BYTES`, `INTERVAL` reads/writes so EP122's probe succeeds. The
interesting call is **`MOSI_TRANSFER` (`0x40107000`)**: EP122 passes a
16-byte `{magic, size, data_ptr}` struct; the module dereferences
`data_ptr` to copy out the 64-byte LED payload, stashes it under
`mosi_lock`, and wakes `mosi_wq` so any reader of `/dev/subucom_ctrl`
unblocks.

MISO is delivered through `spi_read()`. There is **no free-running
hrtimer** - the read blocks on `schedule_timeout_interruptible` for one
poll interval (`interval_us`, default `1176 µs`), then returns either
the latest host-injected frame (`inject_pending`) or the baked-in
`miso_idle` template; either way it stamps a fresh CRC-16/X-25 before
copy-out (`subucom_virt.c:215-237`). Throttling in the caller's
context (instead of via hrtimer→waitqueue) was a deliberate fix for a
QEMU/HVF wake-up race.

Host injection through `/dev/subucom_ctrl` is **sticky**: a 64-byte
write replaces `inject_pending` and stays in effect until the next
write, so the host doesn't need to spin re-injecting the same frame at
850 Hz. Writing zero bytes clears the injection and the module falls
back to idle. `epoll` on `subucom_spi1.0` always reports `POLLIN` -
EP122 throttles itself via `read()` rather than waiting on poll
(`subucom_virt.c:307`).

A module-param `inject_testmode=1` pre-presses `BTN_CALL_PREV +
BTN_TEMPO_RANGE` for 5 s on insmod via a `delayed_work` so the boot
sequence enters service mode without host interaction.

### 2. Guest user-space forwarder

`guest/subucom/forwarder.c`. Single static aarch64 binary, two
directions:

- **LED thread** (`led_thread`, `forwarder.c:121`): blocking
  `read(ctrl_rfd, 64)` on `/dev/subucom_ctrl`, then `write(vport_fd, …)`
  to the `cdj3k.ctrl` virtio-serial port. One frame per syscall pair.
- **Main loop**: blocking `read(vport_fd, 64)` from the host, then
  `write(ctrl_wfd, …)` back into `/dev/subucom_ctrl` (which lands in
  `inject_pending`). A `read() == 0` means `host_connected=false` (no
  cdj-ui peer on `ctrl.sock`); the loop just sleeps and retries - it
  does **not** close the vport, because reopening would burn a
  `guest_connected` toggle on the QEMU side.

The forwarder also watches `b12` for the EP122 power-off signal
(high-nibble `bit 3` clearing): on the first matching frame it spawns
`poweroff_thread` which `execl`s `/bin/systemctl reboot` after a 2 s
delay. (Real hardware sends a sub-CPU power-off pulse; we map it to a
guest reboot.)

Locating the vport: `/sys/class/virtio-ports/*/name` is scanned for
literal `"cdj3k.ctrl"`, then the corresponding `/dev/vportNpM` is
opened (`forwarder.c:34-69`).

### 3. Host side - `CtrlStream`

`crates/cdj3k-emu-streams/src/ctrl_stream.rs`. Connects to
`{sock_dir}/ctrl.sock` with infinite reconnect (2 s backoff). One
background `ctrl-stream` thread runs `read_loop` - strictly 64-byte
reads, no framing, no length prefix. The write half is `try_clone`d so
`inject(&[u8; 64])` can be called from any thread (in practice, the
egui paint thread on user gesture).

Two snapshot mechanisms coexist:

- `peek_latest_mosi()` / `latest_mosi_arc()` - **unconditional**, always
  reflects the most recent wire frame. Used by the debug viewport so it
  sees every heartbeat.
- `take()` - repaint-gated: only stored when bytes changed since the
  last gate-trigger. EP122 retransmits visually-identical frames at
  ~100 Hz; gating keeps the main viewport asleep when no LED actually
  changed (`ctrl_stream.rs:160-200`).

### 4. Decoders - `cdj3k-emu-subucom` crate

`MosiFrame::from_bytes` and `MisoFrame::idle()` give name-based
accessors (`led_bit`, `step_led`, `pad_rgb`, `sd_rgb`, `set_btn`,
`set_jog`, …) so the UI never indexes raw bytes. The `egui-color`
feature exposes `led_color(r,g,b) -> Color32` directly, performing the
gamma + peak-normalise conversion in one call.

---

## Why this split (kernel module + forwarder + virtio-serial)

EP122 reaches the sub-CPU via plain `read(2)` / `ioctl(2)` on a char
device. LD_PRELOAD'ing those syscalls would work but couples subucom
emulation to EP122's process lifecycle (in-place restarts lose state)
and forces the preload to reimplement epoll/ioctl redirection.
Patching kernel SPI core is invasive and version-tied. Shipping our
own char device side-steps both: it *is* `subucom_spi1.0`, survives any
EP122 crash, needs no preload, and exposes a `/dev/subucom_ctrl` side
that's plain user-space - debuggable with `cat`/`dd`/`subucom_live`
(`guest/subucom/live.c`), and replaceable without rebuilding the
kernel module. The forwarder being unprivileged user-space means it
can be killed, gdb'd, or restarted at runtime without touching either
the module or EP122.

---

## Out of scope

- **Slingshot / scroll-bend** input shaping. That lives in the UI layer
  (gesture → MISO frame synthesis), not the subucom protocol itself.
  See `README.md` for user-facing controls.
- **Per-byte field tables.** Treat `mosi_frame.rs` / `miso_frame.rs` as
  the canonical reference; this doc deliberately doesn't duplicate the
  constants so it doesn't go stale.
- **`ctrl.sock` framing / reconnect.** See
  [stream-transports.md](stream-transports.md).
