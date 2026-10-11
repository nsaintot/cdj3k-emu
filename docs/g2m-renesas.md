# G2M (Renesas R-Car) - what it is and why it is unsupported

This emulator targets the **RK3399** (Rockchip aarch64) builds of Pioneer's
CDJ-3000 and CDJ-3000X firmware. Older Pioneer UPDs, and a sibling or
prototype build pipeline, contain a different aarch64 kernel that targets the **Renesas R-Car M3-W (r8a7796)** SoC
on the "Salvator-X" reference board. Internally Pioneer calls this build path
**G2M** (after the R-Car generation, *Gen3 M3-W*).

The firmware extractor (`cdj3k_emu_firmware::extract_kernel`) rejects G2M
payloads with `ExtractError::UnsupportedG2M`. This document describes the G2M
format and what supporting it would take.

## How to recognise a G2M UPD

| Signal | G2M build | CDJ-3000 RK3399 build |
|--------|-----------|-----------------------|
| Outer ISO has `IMAGES/CDJ3K-RK3399.ISO` | no | yes (v3.13+) |
| `boot/Image-*.dtb` filename | `Image-r8a7796-salvator-x.dtb` | `Image-rk3399-*.dtb` (inside the inner ISO) |
| Tar path "rk3399" / kernel content "rk3399" | absent | present |
| Build-system path baked into ELFs | `.../ep122/.../make_G2M_sdk/.../salvator-x/...` | `.../make_*RK3399*_sdk/...` |
| Kernel size | ~91 MB (with embedded initramfs) | ~72 MB |
| Embedded initramfs compression | LZ4 legacy | gzip |

The extractor uses the "rk3399" path/content marker as the discriminator (see
`extract_image_from_tar_gz` in `crates/cdj3k-emu-firmware/src/extract.rs`).

## UPD layout (legacy, 2020-09-08 sample)

```
update.upd (decrypted -> outer ISO)
  images/app.rev          (6 B)  "11367"
  images/system.rev       (6 B)  "11366"
  images/sub.rev          (4 B)  "1.07"
  images/release.txt      (5 B)  "1.05"
  images/extra.tar.gz     (650 KB)  DirectFB shared libs (libdirectfb-1.7.so.7, drivers)
  images/images.tar.gz    (75 MB)  -> boot/Image (91 MB G2M kernel)
                                      boot/Image-r8a7796-salvator-x.dtb (60 KB)
                                      system.rev
  images/pdj.tar.gz       (122 B)  cache.conf (empty)
  images/subucom.mot.gz   (16 KB)  subucom MCU firmware (Motorola S-record)
  mot2bin                 (15 KB)  aarch64 ELF, S-record -> flat binary
  update                  (174 KB) aarch64 ELF, runs from USB stick to flash eMMC
  usb_update.sh           (2.7 KB) shell entry point
```

Notes:
- **There is no separate rootfs file.** The whole G2M rootfs is embedded in
  `boot/Image` as a CPIO archive compressed with **LZ4 legacy** framing
  (magic `02 21 4c 18`). In the sample UPD it lives at file offset `0xD68388`
  and decompresses to ~172 MB of cpio newc.
- The kernel's `IKCONFIG` blob shows `CONFIG_INITRAMFS_SOURCE=""` and
  `# CONFIG_BLK_DEV_RAM is not set`, yet the kernel has an initramfs: Pioneer
  appends it to the kernel image after the build.
- `usb_update.sh` runs *while the existing OS is already up* (it expects a live
  shell with `sgdisk`, `mkfs.ext4`, etc.), copies `extra.tar.gz` onto the
  running system, then invokes the `update` ELF to do the actual eMMC writes.
  A G2M UPD is therefore a *delta update* applied by the running EP122
  system; it never boots on its own.

## Subucom firmware (`subucom.mot.gz`)

The file is a Motorola S-record image of the subucom MCU firmware.
Decompressed, it is 54 KB of text that fills about 22 KB of addresses:

| Address | Size | Contents |
|---------|------|----------|
| `0x00E000` | 4 B | `55 50 01 07` = magic `"UP"` + version 1.07 (matches `sub.rev`) |
| `0x010000-0x010A80` | ~2.7 KB | Application vector table + early code |
| `0x0C0000-0x0C4BEF` | ~19 KB | Main application code |
| `0x0FFFDC+` | 36 B | All-`FF` erased-flash padding |

The target is an ARM Cortex-M in Thumb mode: the reset vector has bit 0 set.
The first ~56 KB of flash (`0x0-0xDFFF`) is the on-chip bootloader and is
**not** in the .mot file. The bootloader checks the `"UP"` magic and the
version header before it flashes the app region. This boot model (`"UP"` magic, bootloader at low addresses, app
relocated to `0x10000`, code at `0xC0000`) is identical to the CDJ-3000
subucom; only the application payload differs.

`mot2bin` converts a Motorola S-record file into a flat binary
(`cat motfile | mot2bin <start> <end> > binfile`). It is a generic 15 KB
Yocto/Poky 2.1.3 aarch64 ELF; the update bundles it because the running OS
converts the .mot file to a binary before writing it over SPI.

## Booting G2M under QEMU: what works, what doesn't

With the gate bypassed (for example, by removing the rk3399 marker check), the
G2M kernel boots cleanly on QEMU `virt` aarch64. `extract_initramfs`
decompresses LZ4 legacy as well as gzip, so the G2M initramfs comes out
intact. End to end, the result is:

- Yocto 2.1.3 init brings the system up.
- udev populates `/dev/dri/card0` and `/dev/fb0` against virtio-gpu.
- The network comes up through DHCP, and SSH (dropbear) is reachable.
- systemd reaches the login prompt on `ttyAMA0`.

### The blocker: `fbdev_drv.so` is hard-wired to rcar-du

`/usr/lib/xorg/modules/drivers/fbdev_drv.so` in the G2M rootfs is Pioneer's
*customised* Xorg fbdev driver, not the upstream one. It contains a
`StartFlipMode()` function that calls `drmOpen("rcar-du", NULL)` and uses
`DRM_IOCTL_RCAR_DU_PAGE_FLIP` for tearing-free updates. Strings extracted from
the binary:

```
rcar-du
drmOpen() failed !! errno=%d
StartFlipMode() called. width=%d, height=%d, depth=%d, bpp=%d
DRM_IOCTL_RCAR_DU_PAGE_FLIP error !! errno=%d, ret=%d, fb=%d
.../ep122/MakeUpdateFile/make_G2M_sdk/build/tmp/sysroots/salvator-x/usr/include/xorg/privates.h
```

QEMU `virt` exposes a `virtio_gpu` DRM device, not `rcar-du`, so the open fails
with `ENOENT`. The fbdev driver then aborts:

```
(EE) drmOpen() failed !! errno=2
(II) StopFlipMode() success.
(EE) FBDEV(0): StartFlipMode() error !!
xinit: unable to connect to X server: Bad file descriptor
```

Aggravating factors:
- The G2M rootfs ships **Xorg 1.18.0** (video driver ABI 20.0). Its
  `/usr/lib/xorg/modules/drivers/` directory contains only `fbdev_drv.so` and
  `dummy_drv.so` - no `modesetting`, no `glx`, no `armsoc`. The existing
  `xorg-modesetting-conf.sh` and `xorg-headless.sh` patches assume ABI 24.0 and
  do not apply here.
- The launcher is `xserver-nodm.service`, which runs `/etc/X11/Xserver`, not
  the `x11-only.sh` script that the RK3399 path uses.

### What G2M support would entail

Roughly, in increasing order of scope:

1. **Patch out `StartFlipMode` in `fbdev_drv.so`** so it returns success without
   touching DRM. X comes up, with visible tearing. This option has the
   smallest scope.
2. **Drop in a stock Yocto 2.1.3 `fbdev_drv.so`** (no Pioneer rcar-du logic) -
   either rebuild `xf86-video-fbdev-0.4.4` against the right ABI or recycle one
   from a Poky 2.1.3 image archive.
3. **LD_PRELOAD shim** that intercepts `drmOpen("rcar-du", ...)` and translates
   `DRM_IOCTL_RCAR_DU_PAGE_FLIP` into `DRM_IOCTL_MODE_PAGE_FLIP` for
   virtio-gpu. This fits the existing `deck_shim.so` pattern (patch 13).
4. **Whatever EP122 itself expects.** This part is unknown: the application
   binary probably uses more Renesas-specific ioctls (display, audio, GPIO)
   than the fbdev driver alone exposes.

Each option is a large change, so G2M stays unsupported.

## What stays in tree

The extraction pipeline retains a couple of G2M-friendly capabilities that are
cheap to keep:

- `extract_initramfs` scans for **both** gzip and LZ4 legacy magic and picks
  the largest valid cpio newc/crc result. This makes the extractor robust to
  the kernel's embedded `.config` (small gzip, not a cpio) and would work for a
  G2M kernel if the gate were removed.
- The `read_firmware_info` path tolerates UPDs without `CDJ3K-RK3399.ISO` and
  falls back to the outer ISO, so version metadata can still be read from a
  legacy UPD even when the kernel itself is rejected later.

The gate lives in `extract_image_from_tar_gz` in
`crates/cdj3k-emu-firmware/src/extract.rs`. It is a single branch and easy to
remove.
