# cdj3k-emu - subucom Protocol

> This page describes the sub-CPU (subucom) SPI protocol that the CDJ-3000
> firmware uses to talk to the deck's input hardware, and how cdj3k-emu
> emulates it end to end with a virtual char device, a guest forwarder and a
> host `ctrl.sock` peer.

---

## End-to-end path

```
                                  host (the emulator UI, egui main thread)
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
        latest MOSI frame + waitqueue  inject_pending (sticky MISO buf)
                       ▲                      │
                       │  ioctl                │ read()
                       │  MOSI_TRANSFER        │ throttled ~850 Hz
                       │                       ▼
                          /dev/subucom_spi1.0   (EP122 polls here)
                                   │
                                   ▼
                                  EP122
```

EP122 polls `/dev/subucom_spi1.0` at about 850 Hz (a 1176 µs period). Each
direction carries 64 B × 850 Hz ≈ 54 KB/s, which is small next to audio or
video. The forwarder and `ctrl.sock` are blocking, and there is no framing:
the receiver reads 64-byte chunks.

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

EP122 talks to it over `ff1d0000.spi` (mode 3, 3.2 MHz, CS0) and polls it at
about 850 Hz. Each poll is a full-duplex 64-byte exchange: **MOSI**
(EP122 to the sub-CPU) carries LED state, and **MISO** (the sub-CPU to EP122)
carries input state. cdj3k-emu replaces the sub-CPU with software but keeps
EP122 unchanged.

---

## Frame anatomy (conceptual)

Both directions are **64 bytes, fixed length**.

| Bytes | Contents |
|-------|----------|
| `[0..2]` | reserved / zero (frame-type header, always `0x0000`) |
| `[2..4]` | constant `0x01 0x04` |
| `[4]` | direction rocker (MISO); `0x03` in the idle frame |
| `[5..62]` | payload (LED PWM for MOSI, input state for MISO) |
| `[62..64]` | **CRC-16/X-25** over bytes `[0..62]`, stored little-endian |

The CRC uses the polynomial `0x8408` (reflected `0x1021`), init `0xFFFF` and
final XOR `0xFFFF`. It is implemented in `crates/cdj3k-emu-panel/src/crc.rs`
(Rust) and `guest/modules/subucom_virt/subucom_virt.c` (kernel).

On the CDJ-3000X (EP145), both frames use this layout, shifted. The MISO frame
sits behind a u16 format version (non-zero, or EP145 drops the frame) with
the same CRC window; the MOSI frame moves the LED bitfield and the RGB lamps
and adds lamps of its own. `cdj3kx.rs` carries the shifts: its MISO encoder
writes the CDJ-3000's fields `MISO_SHIFT` bytes later, and its `MosiMap`
(`cdj3kx::MOSI`) moves the lamps.

The CDJ-1500X (EP166) uses its own 32-byte frames on
`/dev/subucom_spi3.0`, with the same CRC-16/X-25 stored big-endian; see
[Models](models.md) and `cdj1500x.rs`.

### MOSI payload (LED state)

EP122 writes **linear-light PWM bytes**. Three LED classes coexist:

- **Single-bit LEDs** (`(byte, mask)`) - PLAY, CUE, MASTER,
  transport, beat-jump, encoder, …
- **Two-bit "step" LEDs** (`StepLedMask`) - navigation (`SOURCE`,
  `BROWSE`, `TAG LIST`, `PLAYLIST`, `SEARCH`, `MENU`) and
  `SLIP`/`QUANTIZE`. The `medium` bit alone means dim and `medium|full`
  means bright; the decoder maps them to `StepLed::{Off, Medium, Full}`.
- **RGB groups** - eight hot-cue pads at `[12..36]`, plus SD, USB-LIGHT,
  ON-AIR at `[36..45]`, each a raw `(R, G, B)` PWM triple.

The host converts raw PWM into a display colour in `mosi_frame.rs`
(`led_color`): the part's `LedProfile` turns the PWM duty into light
(`led_linear`: its dies, gamma and white gains), then the dominant channel
is normalised to `LED_PEAK` for a saturated colour, rising to 255 as the
colour nears white. This keeps the hue at any drive level;
`led_drive_factor` lets a caller dim the drawn LED to match the drive.

> The `LED_*` bits in `crates/cdj3k-emu-panel/src/cdj3k.rs` map each LED to
> its byte and mask in CDJ-3000 coordinates; the `MosiMap` in `cdj3k.rs` and
> `cdj3kx.rs` shifts them per model. The CDJ-1500X's layout is in
> `cdj1500x.rs`.

### MISO payload (input state)

The payload is mostly bitmasks plus a few scalar fields. By region:

- **Buttons** `[5..12]` - packed bitmasks, one bit per button
  (PLAY/CUE/SEARCH, HOT A..H, BROWSE/MENU/QUANTIZE, POWER_ON, …).
- **Direction rocker** `b04` - **not a bitmask**, a 2-bit value:
  `1` is REV, `2` is SLIP_REV (momentary) and `3` is FWD. Use the `Direction` enum.
- **Rotary encoder** `b14..b16` - 16-bit LE counter.
- **LCD touch** `b16..b20` - two 16-bit LE coordinates; `(0,0)` means no touch.
- **Tempo slider** `b22..b24` - 16-bit LE, dead-band ~`0x7F50..0x7FD0`.
- **Vinyl-speed rotary** `b24` - 8-bit.
- **Jog wheel** `b26..b31` - position (16-bit LE), velocity (16-bit LE,
  inverted: `0xFFFF` is stopped and `0x0000` is maximum speed), then a touch
  byte (`0x00` none, `0x03` press, `0x04` idle baseline, `0x0c` turning).
- **Capacitive sensors** `b32..b44` - 12 bytes; baseline ≈ `0xD5/0xD6`
  per channel; a deviation means a finger is near.
- **Device state** `b12` - `0x80` is power on and `0x01` is SD cover closed.
  The high nibble carries the sub-CPU power-off timer (see *forwarder*).

A pre-baked `IDLE` per model holds the "nothing pressed" baseline. The UI
describes the panel in a model-neutral `PanelState` (pressed and cleared
frame bits, fader travel, jog, rotary, direction, vinyl, touch, power) and
`miso_frame::encode(model, &state)` hands it to the player's `MisoCodec`,
which starts from its `IDLE`, writes each control the way its deck does
(including the CDJ-3000's tempo dead-zone curve) and stamps the CRC.

> The canonical map is in `crates/cdj3k-emu-panel/src/miso_frame.rs` (CDJ-3000
> coordinates); each model's encoder is in its spec file (`cdj3k.rs`,
> `cdj3kx.rs`, `cdj1500x.rs`).

---

## Emulation layers

### 1. Kernel module `subucom_virt.ko`

The module, `guest/modules/subucom_virt/subucom_virt.c`, creates two char
devices under a dynamic major:

| Path | Minor | Used by | Purpose |
|------|-------|---------|---------|
| `/dev/subucom_spi1.0` | 0 | EP122 | Real SPI device replacement |
| `/dev/subucom_ctrl`   | 1 | forwarder | Host bridge |

EP122 talks to `subucom_spi1.0` with the original SPI ioctls. The module
accepts and remembers reads and writes of `TIMER_STATUS`, `BITS_PER_WORD`,
`RX_BYTES` and `INTERVAL`, so EP122's probe succeeds. The main call is **`MOSI_TRANSFER` (`0x40107000`)**: EP122 passes a
16-byte `{magic, size, data_ptr}` struct; the module dereferences
`data_ptr` to copy out the 64-byte LED payload, stashes it under
`mosi_lock`, and wakes `mosi_wq` so any reader of `/dev/subucom_ctrl`
unblocks.

MISO is delivered through `spi_read()`. There is no free-running hrtimer: the
read sleeps in `schedule_timeout_interruptible` for one poll interval
(`interval_us`, default `1176 µs`), then returns either the latest
host-injected frame (`inject_pending`) or the model's idle frame
(`miso_idle_model`, which the module derives from `miso_idle` at load).
Either way it stamps a fresh CRC-16/X-25 before the copy-out. Do not move the
throttle to an hrtimer that wakes a waitqueue: that wake-up races under QEMU
with HVF.

A `write()` on `subucom_spi1.0` is a reset request. Only `subucom_reset` writes
to the device: `apl_end.sh` runs it on the CDJ-3000X when the app stops, and it
sends four 64-byte frames with a valid CRC-16/X-25. The module accepts such a
frame, and when that file is closed it calls `orderly_reboot()`, the way the
real sub-CPU resets the SoC. QEMU runs with `-no-reboot`, so the guest reboot
ends QEMU and the app starts the slot again. A frame of another size or with a
bad CRC fails with `EINVAL`.

Host injection through `/dev/subucom_ctrl` is **sticky**: a 64-byte
write replaces `inject_pending` and stays in effect until the next
write, so the host does not need to re-inject the same frame at 850 Hz.
Writing zero bytes clears the injection, and the module falls back to idle.
`epoll` on `subucom_spi1.0` always reports `POLLIN`; EP122 is throttled by the
blocking `read()`.

The module parameter `inject_testmode=1` holds the model's service-mode button
combination from insmod until the app's first `MOSI_TRANSFER`, so the boot
enters service mode without host input.

### 2. Guest user-space forwarder

The forwarder, `guest/subucom/forwarder.c`, is a single static aarch64 binary
that handles both directions:

- **LED thread** (`led_thread`): a blocking `read(ctrl_rfd, 64)` on
  `/dev/subucom_ctrl`, then a `write(vport_fd, …)` to the `cdj3k.ctrl`
  virtio-serial port, one frame per pair of syscalls.
- **Main loop**: a blocking `read(vport_fd, 64)` from the host, then a
  `write(ctrl_wfd, …)` back into `/dev/subucom_ctrl`, which lands in
  `inject_pending`. A `read()` that returns 0 means `host_connected=false`
  (no UI peer on `ctrl.sock`); the loop sleeps and retries. It does
  **not** close the vport, because reopening it would use up a
  `guest_connected` toggle on the QEMU side.

The forwarder also watches `b12` for the EP122 power-off signal
(high-nibble `bit 3` clearing): on the first matching frame it spawns
`poweroff_thread`, which runs `/bin/systemctl reboot` after 2 s. On real
hardware the sub-CPU cuts the power at this point; the emulator reboots the
guest instead.

The forwarder finds the vport by scanning `/sys/class/virtio-ports/*/name` for
the literal `"cdj3k.ctrl"` and opening the matching `/dev/vportNpM`.

### 3. Host side - `CtrlStream`

`CtrlStream` (`crates/cdj3k-emu-streams/src/ctrl_stream.rs`) connects to
`{sock_dir}/ctrl.sock` and reconnects forever with a 2 s backoff. One
background `ctrl-stream` thread runs `read_loop`, which reads exactly 64 bytes
at a time, with no framing or length prefix. The write half is `try_clone`d,
so `inject(&[u8; 64])` can be called from any thread; in practice the egui
paint thread calls it on a user gesture.

Two snapshot mechanisms coexist:

- `peek_latest_mosi()` and `latest_mosi_arc()` always return the most recent
  wire frame. The debug viewport uses them so that it sees every heartbeat.
- `take()` is repaint-gated: a frame is stored for it only when its bytes
  differ from the frame that last triggered a repaint. EP122 resends identical
  frames at about 100 Hz, and the gate keeps the main viewport asleep when no
  LED changed.

### 4. Decoders - `cdj3k-emu-panel` crate

`MosiFrame::new(bytes, model)` answers for a `Lamp` through the model's
`MosiCodec` (`lamp`, and `lit`, `step`, `rgb` and `drive` built on it), and
`miso_frame::encode(model, &state)` writes the control frame, so the UI never
indexes raw bytes. The `egui-color` feature exposes
`MosiFrame::led_color(part, r, g, b) -> Option<Color32>`, which applies the
gamma and peak normalisation through the part's `LedProfile` in one call.

---

## Why this split (kernel module + forwarder + virtio-serial)

EP122 reaches the sub-CPU through plain `read(2)` and `ioctl(2)` calls on a
char device.

- The module provides `subucom_spi1.0` itself, so it survives EP122 crashes
  and restarts and needs no preload and no patch to the kernel's SPI core.
- `/dev/subucom_ctrl` is plain user space: `cat`, `dd` and `subucom_live`
  (`guest/subucom/live.c`) can read it, and its user side can change without
  rebuilding the module.
- The forwarder is an unprivileged process that can be killed, debugged or
  restarted without touching the module or EP122.

---

## Out of scope

- **Slingshot and scroll-bend** input shaping. It lives in the UI layer, which
  turns gestures into MISO frames. See `README.md` for the controls.
- **Per-byte field tables.** The model spec files (`cdj3k.rs`, `cdj3kx.rs`,
  `cdj1500x.rs`) hold them.
- **`ctrl.sock` framing / reconnect.** See
  [stream-transports.md](stream-transports.md).
