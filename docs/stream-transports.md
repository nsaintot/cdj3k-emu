# cdj3k-emu - Host/Guest Stream Transports

Reference for the four host/guest data streams that move pixels, button/LED
state, and control-plane messages between the QEMU guest and the cdj3k-emu
host process. Audio is covered separately in `docs/audio-stack.md`; the
subucom SPI wire format is documented in `docs/subucom.md`.

---

## Overview

| Stream      | Transport                       | Direction       | Host endpoint                         |
|-------------|---------------------------------|-----------------|---------------------------------------|
| Main LCD    | QEMU `-display shm` (mmap'd file) | guest → host  | `{sock_dir}/main.shm`                 |
| Jog LCD     | ivshmem-plain BAR2 (mmap'd file)  | guest → host  | `{sock_dir}/jog.shm`                  |
| Subucom ctrl | virtio-serial (Unix socket)    | bidirectional   | `{sock_dir}/ctrl.sock`                |
| Config (cfg) | virtio-serial (Unix socket)    | bidirectional   | `{sock_dir}/cfg.sock`                 |

`sock_dir` follows the multi-instance convention
`/tmp/cdj3k-emu/instance-{id}/` (see
`crates/cdj3k-emu-platform/src/runtime_paths.rs`). Authoritative QEMU args
live in `crates/cdj3k-emu-runtime/src/config.rs` around lines 300-337;
`boot.sh --patched` is a dev helper that lays out sockets differently and
is not the production topology.

The legacy virtio-serial main/jog/sub layout (`main.sock`, `jog.sock`,
`sub.sock`, `/dev/vport0p[012]`) is gone. Main is now shm + dirty-rect
publish, jog is ivshmem zero-copy, and the third virtio-serial port has
been deleted (subucom now multiplexes onto `ctrl.sock`).

---

## Stream - Main LCD (`main.shm`)

### Transport

QEMU's in-tree shm display backend, added by
`qemu/patches/06-shm-display-source.patch`. Launched with:

```
-display shm,path={sock_dir}/main.shm
```

QEMU writes the framebuffer plus a small header into a regular file
(`MAP_SHARED`). The host mmaps it read-only and polls a generation
counter; pixel data is read in place - no copies through QEMU's UI
ringbuffer.

### Defined in

- Producer: `qemu/patches/06-shm-display-source.patch` (new file
  `ui/shm-display.c`).
- Consumer: `crates/cdj3k-emu-streams/src/main_stream.rs` (see top-of-file
  doc-comment for the layout).

### Wire layout

Little-endian. Reader polls `generation` with Acquire; on bump, the rest
of the header and the dirty pixels are coherent.

| Offset | Size | Field        | Notes                                       |
|--------|------|--------------|---------------------------------------------|
| 0x00   | u32  | magic        | `0x514D5348` (`QMSH`)                       |
| 0x04   | u32  | generation   | Acquire-load; bumped after each dirty blit  |
| 0x08   | u32  | width        | 1280 in steady state                        |
| 0x0C   | u32  | height       | 720                                         |
| 0x10   | u32  | stride       | bytes per row                               |
| 0x14   | u32  | format       | 1 = RGBA8888 (QEMU swaps XRGB→RGBA host-side) |
| 0x18   | u32  | dirty_x      | bounding box of last update                 |
| 0x1C   | u32  | dirty_y      |                                             |
| 0x20   | u32  | dirty_w      |                                             |
| 0x24   | u32  | dirty_h      |                                             |
| 0x40   | u8[] | pixels       | `stride × height` bytes                     |

Surface dimensions can switch mid-session (e.g. the firmware's 640×480
console handoff to the 1280×720 Xorg surface); the reader re-reads
`width/height/stride` on every generation bump.

### Cadence

QEMU publishes on each dirty blit (effectively ~60 Hz under firmware
activity). Host poll interval is **500 µs**
(`POLL_INTERVAL` in `main_stream.rs`). The reader publishes a
`DisplayDirty` rect carrying an `Arc<Mmap>` so the UI thread uploads to
the GPU texture without a CPU copy.

### Error recovery

The reader checks the magic word every tick. If it disappears (QEMU
exited or replaced the file), the reader drops the mapping and loops on
`open_shm` with a 1 s backoff until the file reappears with a valid
header. A static display keeps the same generation indefinitely - that
is not an error condition and the reader does not time out on it.

---

## Stream - Jog LCD (`jog.shm`)

### Transport

`ivshmem-plain` PCI BAR2, backed by a shared file. The guest's
`ep122_shim.so` writes XRGB pixels straight into the BAR; the host mmaps
the same file and polls a seqlock counter. Zero copies, zero virtqueue
traffic.

QEMU args (config.rs lines 326-337):

```
-object memory-backend-file,id=jogshm,mem-path={sock_dir}/jog.shm,size=1M,share=on
-device ivshmem-plain,memdev=jogshm,master=on
```

### Defined in

- Producer: `guest/ep122_shim/jog_shm.c` (crops the visible 320×240
  region from EP122's stretched 1280×240 DRM jog plane).
- Consumer: `crates/cdj3k-emu-streams/src/jog_stream.rs`.

### Wire layout

| Offset  | Size | Field   | Notes                                                  |
|---------|------|---------|--------------------------------------------------------|
| 0x0000  | u32  | magic   | `0x3147_4F4A` (`JOG1`)                                 |
| 0x0004  | u32  | seq     | seqlock counter - odd = write in progress              |
| 0x0008  | u16  | width   | 320                                                    |
| 0x000A  | u16  | height  | 240                                                    |
| 0x000C  | u32  | format  | 1 = XRGB8888 (B, G, R, X byte order)                   |
| 0x1000  | u8[] | pixels  | `width × height × 4` = 307 200 bytes                   |

### Cadence

Host polls `seq` at **~60 Hz** (`POLL_INTERVAL = 16 ms`). Guest publishes
on every DRM flip (~100 Hz upstream). The seqlock read retries up to 4
times if it observes an in-progress write or a torn `seq_before /
seq_after`.

### Error recovery

- Seqlock retry as above; if all four attempts tear, the reader sleeps
  one poll interval and tries again.
- If the magic word disappears (the runtime zeroes it before unlinking
  the file on cleanup), the reader drops the mapping and re-opens.
- If `seq` hasn't advanced for `ALIVE_TIMEOUT = 2 s`, the stream is
  flagged disconnected so the UI can show a "wait" badge; reads keep
  trying.

---

## Stream - Subucom ctrl (`ctrl.sock`)

### Transport

Bidirectional virtio-serial port `cdj3k.ctrl` exposed as a Unix socket on
the host. Carries the subucom SPI protocol in both directions, 64-byte
frames each way.

QEMU args (config.rs lines 300-324):

```
-device virtio-serial-device,max_ports=8
-chardev socket,id=vserial_ctrl,path={sock_dir}/ctrl.sock,server=on,wait=off
-device virtserialport,chardev=vserial_ctrl,name=cdj3k.ctrl
```

### Defined in

- Host endpoint: `crates/cdj3k-emu-streams/src/ctrl_stream.rs`.
- Frame helpers / CRC: `crates/cdj3k-emu-subucom/src/{mosi_frame.rs,
  miso_frame.rs, crc.rs}`.
- Guest bridge: `guest/subucom/forwarder.c` (connects the kernel
  character device `/dev/subucom_ctrl` to the virtio-serial port; module
  source `guest/modules/subucom_virt/subucom_virt.c`).
- Wire-level field layout: see `docs/subucom.md`.

### Directions

- **MOSI - guest → host (LEDs).** `subucom_virt.ko` exposes
  `/dev/subucom_ctrl`. EP122 writes 64-byte LED-state frames there;
  `forwarder.c` pipes each frame onto the virtio-serial port. The host
  reader (`ctrl_stream.rs::stream_loop`) decodes them through
  `MosiFrame` and drives the on-screen LED simulation.
- **MISO - host → guest (buttons/jog/touch).** The UI builds 64-byte
  `MisoFrame`s representing button bitmasks, jog rotation/touch, and
  performance-pad state. Each frame is CRC-stamped (CRC-16/X-25 over
  bytes `[0..62]`, written little-endian into bytes `[62..64]`) and
  pushed back through the same socket. `forwarder.c` writes them into
  `/dev/subucom_ctrl`, where the firmware consumes them as if they came
  from the real subucom MCU.

### Cadence

MOSI is paced by EP122's SPI loop (~100 Hz). MISO is event-driven: the
UI sends a frame on every input change plus an idle keep-alive.

### Error recovery

The reader/writer pair connects lazily; on EOF or connect failure
(`ECONNREFUSED` while QEMU is still bringing the chardev up) it retries
after `RECONNECT_DELAY = 2 s`. The write half is shared via
`Arc<Mutex<Option<UnixStream>>>` and is dropped on any I/O error so the
next reconnect re-establishes it.

---

## Stream - Config (`cfg.sock`)

### Transport

Second bidirectional virtio-serial port (`cdj3k.cfg`). Line-based ASCII,
`\n`-terminated. No framing beyond newlines; no binary.

QEMU args: same `virtio-serial-device` controller as ctrl, second
chardev/virtserialport pair on it.

### Defined in

- Host: `crates/cdj3k-emu-runtime/src/cfg.rs` (top doc-comment is the
  canonical protocol reference).
- Guest: `guest/cfgd/cfgd.c`.

### Commands

| Direction     | Line                          | Meaning                                                 |
|---------------|-------------------------------|---------------------------------------------------------|
| host → guest  | `usb attach`                  | invoke `/usr/sbin/usb-external-attach.sh` on the guest  |
| host → guest  | `usb detach`                  | no-op on the guest (EP122 owns unmount)                 |
| host → guest  | `set <name> <value>`          | write a whitelisted virtio_snd sysfs param              |
| host → guest  | `get <name>`                  | request the current value of a whitelisted param        |
| guest → host  | `usb_state <0\|1>`            | emitted by the in-guest USB hooks on mount/unmount      |
| guest → host  | `param <name> <value>`        | response to `get`, or unsolicited push on change        |
| guest → host  | `latency <g>,<h>,<t>`         | guest/host/total audio latency, pushed every 3 s        |

`name` and `value` must not contain spaces or newlines; the host-side
setter rejects those at the API boundary.

### Cadence

Largely idle. `latency` push every 3 s; everything else is event-driven.

### Error recovery

Self-healing on both halves: reader reconnects on EOF after
`RECONNECT_DELAY = 500 ms`. Writer fast-path uses the long-lived stream
held by the reader thread; if that's missing (e.g. immediately after
spawn) the writer opens a one-shot connection with up to 20 × 100 ms
retries and a 2 s write timeout (`COLD_WRITER_*` in `cfg.rs`).

---

## QEMU device-arg reference

Concatenated from `crates/cdj3k-emu-runtime/src/config.rs` for the
streams documented here (the audio devices live separately - see
`docs/audio-stack.md`):

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

All Unix sockets use `server=on,wait=off` so they exist before the host
reader connects; readers must tolerate `ECONNREFUSED` / EOF and retry.

---

## Bandwidth budget

Worst-case figures, one instance.

| Stream    | Steady-state                              | Worst case                                | Notes                                         |
|-----------|-------------------------------------------|-------------------------------------------|-----------------------------------------------|
| Main LCD  | Dirty-rect deltas, typically a few MB/s   | ~221 MB/s if the whole 1280×720×4 surface flips every 60 Hz | Zero-copy mmap - no QEMU UI ring traffic       |
| Jog LCD   | Seqlock-gated; host reads at 60 Hz        | 320×240×4 × 60 Hz = ~17.6 MB/s read       | Read-only mmap on host; guest writes ~100 Hz raw |
| ctrl.sock | 64 B × ~100 Hz each direction = ~13 KB/s  | Same - bounded by 100 Hz SPI loop         | virtio-serial, negligible                      |
| cfg.sock  | A handful of ASCII lines per second       | Same                                      | Negligible                                     |

The shm and ivshmem paths don't traverse QEMU's UI/virtqueue plumbing,
so "bandwidth" here is just memory-bus reads on the host side; nothing
competes with the audio path.
