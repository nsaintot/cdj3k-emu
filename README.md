# cdj3k-emu

A macOS desktop app that boots Pioneer DJ's **CDJ-3000** firmware inside QEMU on
Apple Silicon, surfacing the device's main LCD, jog LCD, jog wheel, faders,
buttons, and Pro DJ Link network in a native window.

<img src="docs/preview.png" alt="cdj3k-emu screenshot" width="450"/>

## Why this exists

Built primarily for the technical challenge: figuring out what it takes to put
a piece of proprietary, tightly-coupled DJ hardware on a host hypervisor and
keep its real firmware happy. The interesting parts are the small surfaces
between a stock Linux kernel and the Pioneer userspace — DRM/KMS for the jog
LCD, the SPI sub-CPU protocol for the controls, the virtio-snd cadence shaping
that EP122's audio callback expects.

A secondary goal is having a reproducible bench for studying the **Pro DJ Link
(DJPL)** network protocol — beat broadcasts, master/slave hand-off, abs-pos
sync, the relationship between audible playback and what other players see on
the wire. Multiple emulator slots on the same host let you observe both ends of
a sync transaction with a packet capture between them, which is much harder to
do with real CDJs on a physical LAN when you're on vacation.

> [!TIP]
>
> ## What it is
>
> - A user-space wrapper around QEMU `aarch64-virt` running a vanilla Linux 6.6
>   kernel and a small set of out-of-tree modules + LD_PRELOAD shim that emulate
>   the Pioneer-specific hardware:
> - **virtio_snd** for audio (custom kernel module sized for the EP122 audio
>   callback cadence).
> - **subucom_virt** for the SPI sub-CPU protocol (buttons, jog encoder,
>   rotary, slider, capacitive sensors).
> - **ep122_shim** (LD_PRELOAD) that emulates the Rockchip-specific DRM/KMS
>   ABI EP122 expects (jog LCD as DSI-2, vsync_time, etc.) and routes jog-LCD
>   pixels through ivshmem so the host renders them as an egui texture.

> [!CAUTION]
>
> ## What it is **not**
>
> - **It does not ship Pioneer firmware.** You need to supply your own copy
>   of a CDJ-3000 update file (`.UPD`) and decryption key at first launch.
>   The in-app **Install Firmware** wizard decrypts it locally and provisions
>   a per-instance eMMC qcow2 disk image under `~/Library/Application Support/com.cdj3k.emu/`.
>   Nothing Pioneer-owned is in this repository or the distributed bundle.
> - Not endorsed by, affiliated with, or sponsored by AlphaTheta Corporation /
>   Pioneer DJ. CDJ, rekordbox, and Pro DJ Link are trademarks of their
>   respective owners.
> - Not a frame-accurate hardware simulator. Pixel output and SPI timings are
>   faithful enough to drive EP122 (the stock firmware), but the goal is a
>   daily-driver emulator, not a forensic recreation.
> - Not a turntable replacement. Jog wheel and rotary feel are reproduced via
>   pointer drag + scroll wheel; there's no support for an external MIDI
>   controller bridging into the emulated SPI bus.
> - Not currently portable. Apple Silicon macOS only (HVF, vmnet.framework,
>   AppKit window/menu, CoreAudio). Linux/Windows are out of scope for v0.1.
> - **Not compatible with pre-3.00 firmware.** See _Firmware compatibility_
>   below — only CDJ-3000 firmware 3.00 and newer are accepted.

## Firmware compatibility

> [!IMPORTANT]  
> cdj3k-emu accepts **CDJ-3000 firmware version 3.00 or newer** only.

Pioneer shipped two different system-on-chip families across the CDJ-3000's
lifetime:

- **Renesas G2M** — used in firmware **1.x and 2.x**. Different kernel, very
  different driver stack, different DRM/KMS path for the jog LCD. cdj3k-emu
  does **not** support this hardware target.
- **Rockchip RK3399** — used in firmware **3.00 and newer**. This is the
  target cdj3k-emu emulates: vanilla Linux 6.6 aarch64 kernel, the
  `ep122_shim` LD_PRELOAD, the `subucom_virt` SPI emulation, the
  `virtio_snd` audio pipeline are all written against the RK3399 userspace
  that the 3.00+ firmware ships.

If your `.UPD` decrypts to a Renesas-G2M kernel + rootfs the firmware wizard
will reject it before provisioning the eMMC image. Use a 3.00+ update file.

## About the decryption key

Pioneer ships CDJ-3000 firmware updates as `.UPD` files whose payload is
LUKS-encrypted. The "decryption key" the firmware wizard asks for is the LUKS
keyfile that unwraps that payload so we can mount it and extract the kernel +
rootfs.

This repository does **not** ship that key, for the same reason it doesn't
ship the `.UPD` itself: it is Pioneer-controlled material and we have no right
to redistribute it. cdj3k-emu's posture is strict — nothing Pioneer-owned in
the repo, nothing Pioneer-owned in the bundle. Acquiring the keyfile is the
user's responsibility, by whatever means they are themselves entitled to.

## System requirements

- **macOS 13 (Ventura) or newer**, Apple Silicon (M1/M2/M3/M4).
  HVF needs the `com.apple.security.hypervisor` entitlement; `bundle.sh`
  signs the binary with it.
- **macOS 15 (Sequoia) is strongly recommended.** cdj3k-emu auto-detects
  the host version at every spawn:
  - 15+ → QEMU uses Apple's **in-kernel ARM vGIC** (`hv_gic_create`).
    Drops the per-IRQ vCPU-exit cost dramatically. EP122's chatty IRQ
    pattern is the dominant cost driver for HVF guests, so this is a
    big perf win on 15+.
  - 13 / 14 → QEMU falls back to **userspace GIC emulation**
    (`kernel-irqchip=off`). Functional but noticeably more CPU per
    instance; you'll feel it most when running multiple slots at once.
- Roughly 8 GB free RAM if you plan to run two instances at once
  (1.5 GB guest each + host overhead).
- A copy of a Pioneer CDJ-3000 firmware update file and its decryption key.

## Known limitations

- Audio is not a bit-perfect reproduction of the real hardware. The CDJ-3000
  ships dedicated audio silicon with its own clock domain and DSP path; we
  re-route the firmware's PCM through `virtio_snd` -> a QEMU bypass ring ->
  CoreAudio, which is a fundamentally different pipeline. Occasional pops
  and transient artifacts can occur, especially under host CPU pressure or
  when the macOS HAL device clock drifts against the guest. A large amount
  of work has gone into mitigations (lock-free SPSC ring, dedicated RT
  writer thread, soft-PLL clock-skew correction, pipeline-depth watchdog,
  RELEASE-flush on recovery) - see `docs/audio-stack.md` for the full
  pipeline. Steady-state output on a matched-rate device is clean, but the
  result is not forensically identical to a physical CDJ-3000.
- Audio latency depends on macOS CoreAudio queue depth - typically 30-80 ms
  end-to-end with the bundled bypass-ring patches. ALC ("Enable ALC") shifts
  the audible / Pro DJ Link broadcast alignment to compensate but adds a
  bit of jitter; default-on.
- Headphone monitor, mixer crossfade, and EQ are emulated up to the audio
  pipeline boundary; FX engine is not.
- Jog wheel feel is "good enough for cueing" — no torque feedback.
  Brake stop-time map matches the device's `jog_adjust` rotary, so
  the _cadence_ of stops is faithful even if the touch isn't.
- Service mode (EP122TestMode) is reachable via the Emulation menu but some
  test routines that touch hardware-only registers (e.g. fan-RPM read) return
  fixed values.

## Controls

### Jog wheel

You can control the jog wheel a few different ways:

- **Click and drag the center** — for scratching effects.
- **Click and drag the outside ring** — for a subtle pitch bend.
- **Scroll the mouse wheel** anywhere on the jog — also bends pitch.
- **Hold Ctrl, click-and-drag, then release ("slingshot")** — gives the wheel a flick, which is shown as a line. The faster you pull, the more spin you get.

### Using multiple controls at once

On a real CDJ, you can hold a button or touch the screen while doing something else, like turning the jog wheel. With a mouse, there's only one pointer, so you can hold **Ctrl** to "latch" a button or touchscreen press:

- **Ctrl + click a button** — the button stays pressed until you let go of Ctrl.
- **Ctrl + click the LCD touchscreen** — touch stays down until you release Ctrl.

This lets you, for example, keep `Search Forward` held down while moving the jog wheel to search faster, just like on real hardware.

## Building

```bash
# 1. Build QEMU (clones upstream, applies our shm-display patch, ~10 min).
./qemu/build.sh

# 2. Build the aarch64 kernel, out-of-tree modules, and guest tools (Docker).
./build.sh

# 3. Assemble the .app bundle.
./bundle.sh                # ad-hoc signed (HVF works, FDA does not)
./bundle.sh --sign "Apple Development"   # real cert (enables Full Disk Access)
./bundle.sh --sign "..." --dmg           # also produce a distributable .dmg
```

## First-run privilege prompts

cdj3k-emu asks for the macOS admin password in three specific situations,
**only when you ask it to**:

| Action                                                          | Why it elevates                                                                                                                                                                                                        |
| --------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Selecting a physical Wi-Fi / Ethernet interface for Pro DJ Link | Starts the bundled `socket_vmnet` daemon as root — the only way to bridge an L2 network on macOS without writing a kernel extension. Re-used across instances; one prompt per host interface.                          |
| Selecting a TAP interface (OpenVPN etc.)                        | Creates a macOS `bridge` device + assigns a `tap` device to QEMU via Authorization Services. Torn down automatically when the app exits.                                                                               |
| Attaching a physical USB drive in pass-through mode             | `chmod 660` on `/dev/diskN` so QEMU can open it `O_RDWR`. The exact device path is validated against `/dev/disk[0-9]+(s[0-9]+)?` before elevation — see `crates/cdj3k-emu-runtime/src/usb.rs::is_valid_bsd_disk_path`. |

All three use the native macOS password dialog (TouchID / Apple Watch eligible)
via `AuthorizationServices`, not a CLI prompt. None of them grant ongoing
privileges — every elevation is scoped to one command.


## Repository layout

```
app/cdj3k-emu/       binary (eframe egui app + runtime worker)
crates/cdj3k-emu-*   Rust workspace: subucom, streams, platform, ui,
                     runtime, storage, firmware
guest/               C sources built for the guest:
  cfgd/                cdj3k-cfgd  - virtio-serial config daemon
  ep122_shim/          ep122_shim.so - LD_PRELOAD shim
  subucom/             subucom_forwarder, subucom_live
  modules/             out-of-tree kernel modules (subucom_virt, virtio_snd, udev_usb1)
  kernel-patches/      vanilla 6.6 patches + the guest kernel .config
qemu/                upstream QEMU source + our overlay patches
docker/              Alpine + Ubuntu build pipeline for guest artefacts
initramfs-patch/     numbered rootfs patch scripts (concatenated by bundle.sh)
.github/workflows/   CI (build + bundle on every push)
```

## License

cdj3k-emu's original sources are dual-licensed at your option under either of:

- **Apache License, Version 2.0** ([`LICENSE-APACHE`](LICENSE-APACHE))
- **MIT License** ([`LICENSE-MIT`](LICENSE-MIT))

Linux kernel modules in `guest/modules/`, kernel patches in
`guest/kernel-patches/`, the QEMU embedding shim in `qemu/shim/`, and QEMU
patches in `qemu/patches/` are derivatives of GPL-2.0 upstreams and retain
**GPL-2.0-or-later**. See [`NOTICE`](NOTICE) for a full inventory of bundled
third-party components and the firmware-acquisition expectation.

## Contributing

Issues and pull requests welcome. By submitting a contribution you agree it
is dual-licensed Apache-2.0 / MIT per [`LICENSE-APACHE`](LICENSE-APACHE) and
[`LICENSE-MIT`](LICENSE-MIT), except for contributions inside the GPL-2.0
carve-outs listed in [`NOTICE`](NOTICE), which remain GPL-2.0-or-later.
