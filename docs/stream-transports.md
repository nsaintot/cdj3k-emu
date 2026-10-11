# cdj3k-emu - Host/Guest Stream Transports

This page describes the four data streams that move pixels, button and LED
state, and control messages between the QEMU guest and the cdj3k-emu host
process. Audio is covered in `docs/audio-stack.md`, and the subucom SPI wire
format in `docs/subucom.md`.

---

## Overview

| Stream      | Transport                       | Direction       | Host endpoint                         |
|-------------|---------------------------------|-----------------|---------------------------------------|
| Main LCD    | QEMU `-display shm` (mmap'd file) | guest → host  | `{sock_dir}/main.shm`                 |
| Jog LCD     | ivshmem-plain BAR2 (mmap'd file)  | guest → host  | `{sock_dir}/jog.shm`                  |
| Subucom ctrl | virtio-serial (Unix socket)    | bidirectional   | `{sock_dir}/ctrl.sock`                |
| Config (cfg) | virtio-serial (Unix socket)    | bidirectional   | `{sock_dir}/cfg.sock`                 |

`sock_dir` follows the multi-instance convention
`/tmp/cdj3k-emu-{uid}/instance-{id}/` (`%TEMP%\cdj3k-emu\instance-{id}\` on
Windows). `crates/cdj3k-emu-platform/src/runtime_paths/`
is the source of truth for the layout and the length limit on it; the copy in
`tools/midi-driver/link.c` must agree with it. The QEMU
arguments are built by `QemuConfig::build_argv` in
`crates/cdj3k-emu-runtime/src/config.rs`.

`{sock_dir}/window.sock` is not a guest stream. The window that holds the slot
listens on it for commands from other windows, such as `restart-mods` or
`quit`; `crates/cdj3k-emu-platform/src/window_socket.rs` documents the
protocol.

The sockets are AF_UNIX on Windows too (Windows 10 1803 and later; the host
side is `uds_windows`, `platform::local_socket`), so the chardev lines are the
same on every host; the path limit there is 108 bytes. Patches 15-17 give
Windows QEMU `memory-backend-file` (behind `jog.shm`), the shm display's file
mapping (`main.shm`) and `ivshmem-plain`; `ivshmem-doorbell` is refused there.

QEMU's monitor is `-qmp tcp:localhost:<4445 + slot>`. Where the disk adapter
hands a raw USB disk to QEMU as a descriptor (Linux), a second monitor listens
on `{sock_dir}/qmp-fd.sock`: `add-fd` needs a Unix socket to carry the
descriptor, and is Unix only (`qmp/fd_passing/`). The Windows adapter does
not pass descriptors (`PASSES_FDS = false`); QEMU opens the disk by path.

---

## Stream - Main LCD (`main.shm`)

### Transport

The main LCD uses the shm display backend that
`qemu/patches/06-shm-display-source.patch` adds to QEMU. QEMU runs with:

```
-display shm,path={sock_dir}/main.shm
```

QEMU writes the framebuffer plus a small header into a regular file
(`MAP_SHARED`). The host maps it read-only and polls a generation counter;
the pixels are read in place, with no copy through QEMU's UI ring buffer.

### Defined in

- Producer: `qemu/patches/06-shm-display-source.patch` (new file
  `ui/shm-display.c`).
- Consumer: `crates/cdj3k-emu-streams/src/main_stream.rs` (its module
  documentation describes the layout).

### Wire layout

All fields are little-endian. `generation` is a sequence counter: QEMU adds
1 before it writes an update, which makes it odd, and adds 1 with Release
ordering when the update is written, which makes it even again. The reader
polls it with an Acquire load and skips a tick while it is odd. When it is
even and has changed, the header and the dirty pixels are consistent. The
reader loads the counter again after it reads the rect; if it moved, the
reader uploads the whole surface. A step of 2 is exactly one update; a larger
step means that the reader missed an update, and it uploads the whole surface.

| Offset | Size | Field        | Notes                                       |
|--------|------|--------------|---------------------------------------------|
| 0x00   | u32  | magic        | `0x514D5348` (`QMSH`)                       |
| 0x04   | u32  | generation   | sequence counter; odd during an update      |
| 0x08   | u32  | width        | 1280 in steady state                        |
| 0x0C   | u32  | height       | 720                                         |
| 0x10   | u32  | stride       | bytes per row                               |
| 0x14   | u32  | format       | `1`, RGBA8888; QEMU converts XRGB to RGBA on the host |
| 0x18   | u32  | dirty_x      | bounding box of last update                 |
| 0x1C   | u32  | dirty_y      |                                             |
| 0x20   | u32  | dirty_w      |                                             |
| 0x24   | u32  | dirty_h      |                                             |
| 0x40   | u8[] | pixels       | `stride × height` bytes                     |

The surface size can change during a session (for example, when the
firmware's 640×480 console hands over to the 1280×720 Xorg surface); the
reader re-reads
`width/height/stride` on every generation bump.

### Cadence

QEMU publishes on each dirty blit, about 60 Hz while the firmware draws. The
host polls every 500 µs (`POLL_INTERVAL` in `main_stream.rs`). The reader publishes a
`DisplayDirty` rect carrying an `Arc<Mmap>` so the UI thread uploads to
the GPU texture without a CPU copy.

### Error recovery

The reader checks the magic word every tick. If it disappears (QEMU
exited or replaced the file), the reader drops the mapping and loops on
`open_shm` with a 1 s backoff until the file reappears with a valid
header. A static display keeps the same generation indefinitely; this is not
an error, and the reader does not time out on it.

---

## Stream - Jog LCD (`jog.shm`)

### Transport

The jog LCD uses `ivshmem-plain` PCI BAR2, backed by a shared file. The
guest's `deck_shim.so` writes XRGB pixels straight into the BAR; the host maps
the same file and polls a seqlock counter. Nothing is copied and no virtqueue
traffic is involved.

QEMU arguments:

```
-object memory-backend-file,id=jogshm,mem-path={sock_dir}/jog.shm,size=1M,share=on
-device ivshmem-plain,memdev=jogshm,master=on
```

### Defined in

- Producer: `guest/deck_shim/core/jog_shm.c` (crops the visible 320×240
  region from EP122's stretched 1280×240 DRM jog plane).
- Consumer: `crates/cdj3k-emu-streams/src/jog_stream.rs`.

### Wire layout

| Offset  | Size | Field   | Notes                                                  |
|---------|------|---------|--------------------------------------------------------|
| 0x0000  | u32  | magic   | `0x3147_4F4A` (`JOG1`)                                 |
| 0x0004  | u32  | seq     | seqlock counter; an odd value means a write is in progress |
| 0x0008  | u16  | width   | 320                                                    |
| 0x000A  | u16  | height  | 240                                                    |
| 0x000C  | u32  | format  | 1 = XRGB8888 (B, G, R, X byte order)                   |
| 0x1000  | u8[] | pixels  | `width × height × 4` = 307 200 bytes                   |

### Cadence

The host polls `seq` at about 60 Hz (`POLL_INTERVAL = 16 ms`). The guest
publishes on every DRM flip, about 100 Hz. The seqlock read retries up to 4
times when it sees a write in progress or a `seq_before`/`seq_after` mismatch.

### Error recovery

- The seqlock read retries as described above; if all four attempts tear,
  the reader sleeps one poll interval and tries again.
- If the magic word disappears (the runtime zeroes it before unlinking
  the file on cleanup), the reader drops the mapping and re-opens.
- If `seq` has not advanced for `ALIVE_TIMEOUT = 2 s`, the stream is
  flagged disconnected so the UI can show a "wait" badge; reads keep
  trying.

---

## Stream - Subucom ctrl (`ctrl.sock`)

### Transport

The stream uses the bidirectional virtio-serial port `cdj3k.ctrl`, which the
host sees as a Unix socket. It carries the subucom SPI protocol as 64-byte
frames in each direction.

QEMU arguments:

```
-device virtio-serial-device,max_ports=8
-chardev socket,id=vserial_ctrl,path={sock_dir}/ctrl.sock,server=on,wait=off
-device virtserialport,chardev=vserial_ctrl,name=cdj3k.ctrl
```

### Defined in

- Host endpoint: `crates/cdj3k-emu-streams/src/ctrl_stream.rs`.
- Frame helpers / CRC: `crates/cdj3k-emu-panel/src/{mosi_frame.rs,
  miso_frame.rs, crc.rs}`.
- Guest bridge: `guest/subucom/forwarder.c` (connects the kernel
  character device `/dev/subucom_ctrl` to the virtio-serial port; module
  source `guest/modules/subucom_virt/subucom_virt.c`).
- Wire-level field layout: see `docs/subucom.md`.

### Directions

- **MOSI, guest to host (LEDs).** EP122 sends 64-byte LED-state frames with
  the `MOSI_TRANSFER` ioctl on `/dev/subucom_spi1.0`. `forwarder.c` reads
  each frame from `/dev/subucom_ctrl` and writes it to the virtio-serial
  port. The host reader (`ctrl_stream.rs`) decodes the frames through
  `MosiFrame` and drives the on-screen LEDs.
- **MISO, host to guest (buttons, jog, touch).** The UI describes the panel
  in a `PanelState` and the player's `MisoCodec` encodes it into a 64-byte
  frame, CRC-stamped where that deck's sub-CPU checks it (CRC-16/X-25 over
  `[0..62]`, little-endian at `[62..64]` on the RK3399 decks; over `[0..24]`,
  big-endian at `[24..26]` on the CDJ-1500X), and pushes it back through the
  same socket. `forwarder.c` writes each frame into `/dev/subucom_ctrl`, and
  the firmware reads it as if it came from the real subucom MCU.

### Cadence

MOSI is paced by EP122's SPI loop (~100 Hz). MISO is event-driven: the
UI sends a frame on every input change, and `subucom_virt` repeats the last
one to the app.

### Error recovery

The reader and writer connect lazily. On EOF or a failed connect
(`ECONNREFUSED` while QEMU is still bringing the chardev up), the reader
retries after `RECONNECT_DELAY = 2 s`. The write half is shared through
`Arc<Mutex<Option<UnixStream>>>` and is dropped on any I/O error, so the next
reconnect replaces it.

---

## Stream - Config (`cfg.sock`)

### Transport

The config stream uses a second bidirectional virtio-serial port, `cdj3k.cfg`.
It carries ASCII lines that end in `\n`, with no other framing and no binary
data.

QEMU adds a second chardev and virtserialport pair on the same
`virtio-serial-device` controller as ctrl.

### Defined in

- Host: `crates/cdj3k-emu-runtime/src/cfg.rs`.
- Guest: `guest/cfgd/cfgd.c`. Its header comment is the full protocol
  reference, including the mods report lines.

### Commands

| Direction     | Line                          | Meaning                                                 |
|---------------|-------------------------------|---------------------------------------------------------|
| host → guest  | `usb attach`                  | run `/usr/sbin/usb-external-attach.sh` on the guest     |
| host → guest  | `usb detach`                  | write `umount /dev/sdb` to `/proc/udev_usb1`, lazy-unmount `/media/usb/sd*`, emit `usb_state 0` |
| host → guest  | `set <name> <value>`          | write a whitelisted virtio_snd sysfs param              |
| host → guest  | `get <name>`                  | request the current value of a whitelisted param        |
| host → guest  | `pc_link on\|off`             | start or stop `cdj3k-pc-link-bridge.service`            |
| host → guest  | `mods status`, `mods log <name>` | request the mods report or a mod's journal for this boot |
| host → guest  | `ping`                        | ask cfgd to answer `pong`                               |
| guest → host  | `usb_state <0\|1>`            | emitted when the in-guest USB hooks mount or unmount    |
| guest → host  | `param <name> <value>`        | response to `get`, or unsolicited push on change        |
| guest → host  | `latency <g>,<h>,<t>`         | guest, host and total audio latency, pushed every 3 s   |
| guest → host  | `pc_link on\|off\|on_failed\|off_failed` | the result of a `pc_link` command          |
| guest → host  | `mods …`, `mod …`, `log_begin` / `log` / `log_end` | the mods report and mod journals |
| guest → host  | `pong`                        | answer to `ping`                                        |

`name` and `value` must not contain spaces or newlines; the host-side setter
rejects them.

### Cadence

The stream is mostly idle. cfgd pushes `latency` every 3 s; everything else
is event-driven.

### Error recovery

- The reader thread connects lazily and reconnects after an EOF or a failed
  connect, every `RECONNECT_DELAY = 500 ms`.
- The writer uses a clone of the reader's stream. Commands are queued until
  cfgd sends its first line on the connection; a ping thread sends `ping`
  every second (`PING_PERIOD`) until cfgd answers. The queue is then written
  in order.
- On each new connection the host queues `mods status`, so a mods report
  pushed while nothing was connected is not lost.

---

## QEMU device-arg reference

These are the arguments that `crates/cdj3k-emu-runtime/src/config.rs` adds for
the streams on this page. The audio devices are described in
`docs/audio-stack.md`.

```
# Main LCD - shm display
-display shm,path={sock_dir}/main.shm

# Jog LCD - ivshmem-plain BAR2
-object memory-backend-file,id=jogshm,mem-path={sock_dir}/jog.shm,size=1M,share=on
-device ivshmem-plain,memdev=jogshm,master=on

# virtio-serial controller for ctrl + cfg
-device virtio-serial-device,max_ports=8

# Subucom MOSI/MISO bridge
-chardev socket,id=vserial_ctrl,path={sock_dir}/ctrl.sock,server=on,wait=off
-device  virtserialport,chardev=vserial_ctrl,name=cdj3k.ctrl

# Config / control plane
-chardev socket,id=vserial_cfg,path={sock_dir}/cfg.sock,server=on,wait=off
-device  virtserialport,chardev=vserial_cfg,name=cdj3k.cfg
```

All Unix sockets use `server=on,wait=off`, so they exist before the host reader
connects; readers must tolerate `ECONNREFUSED` and EOF and retry.

---

## Bandwidth budget

The figures are for one instance.

| Stream    | Steady-state                              | Worst case                                | Notes                                         |
|-----------|-------------------------------------------|-------------------------------------------|-----------------------------------------------|
| Main LCD  | Dirty-rect deltas, typically a few MB/s   | ~221 MB/s if the whole 1280×720×4 surface flips every 60 Hz | Zero-copy mmap - no QEMU UI ring traffic       |
| Jog LCD   | Seqlock-gated; host reads at 60 Hz        | 320×240×4 × 60 Hz = ~17.6 MB/s read       | Read-only mmap on host; guest writes ~100 Hz raw |
| ctrl.sock | MOSI: 64 B × ~100 Hz = ~6.4 KB/s; MISO: one 64-byte frame per input change | ~6.4 KB/s MOSI; MISO bounded by the pointer event rate | virtio-serial, negligible |
| cfg.sock  | A handful of ASCII lines per second       | Same                                      | Negligible                                     |

The shm and ivshmem paths do not go through QEMU's UI or virtqueues, so their
bandwidth is memory reads on the host; none of it competes with the audio
path.
