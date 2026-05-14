# G2M (Renesas R-Car) - what it is, why we don't support it

This emulator targets the **CDJ-3000 RK3399** (Rockchip aarch64) build of Pioneer
firmware. Older Pioneer UPDs - and a sibling/prototype build pipeline - contain
a different aarch64 kernel that targets the **Renesas R-Car M3-W (r8a7796)** SoC
on the "Salvator-X" reference board. Internally Pioneer calls this build path
**G2M** (after the R-Car generation, *Gen3 M3-W*), and the application product
codename is **EP122**.

The firmware extractor (`cdj3k_emu_firmware::extract_kernel`) now rejects G2M
payloads with `ExtractError::UnsupportedG2M`. This document records what we
learned while investigating G2M, so a future contributor who wants to revive it
doesn't have to re-do the archaeology.

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
- The build-time `CONFIG_INITRAMFS_SOURCE=""` and `# CONFIG_BLK_DEV_RAM is not
  set` lines you can read from the kernel's own `IKCONFIG` blob are misleading:
  Pioneer concatenates the initramfs to the kernel image post-build, so a quick
  config read suggests "no initramfs" when there absolutely is one.
- `usb_update.sh` runs *while the existing OS is already up* (it expects a live
  shell with `sgdisk`, `mkfs.ext4`, etc.), copies `extra.tar.gz` onto the
  running system, then invokes the `update` ELF to do the actual eMMC writes.
  In other words, a G2M UPD is a *delta update* applied by the running EP122
  system - it never boots standalone.

## Subucom firmware (`subucom.mot.gz`)

Motorola S-record file holding the subucom MCU firmware. Decompressed: 54 KB
text, ~22 KB of populated address bytes:

| Address | Size | Contents |
|---------|------|----------|
| `0x00E000` | 4 B | `55 50 01 07` = magic `"UP"` + version 1.07 (matches `sub.rev`) |
| `0x010000-0x010A80` | ~2.7 KB | Application vector table + early code |
| `0x0C0000-0x0C4BEF` | ~19 KB | Main application code |
| `0x0FFFDC+` | 36 B | All-`FF` erased-flash padding |

Target: ARM Cortex-M (Thumb; reset vector has bit 0 set). The first ~56 KB of
flash (`0x0-0xDFFF`) is the on-chip bootloader and is **not** in the .mot - the
bootloader validates the `"UP"` magic + version header before flashing the app
region. This boot model (`"UP"` magic, bootloader at low addresses, app
relocated to `0x10000`, code at `0xC0000`) is identical to the CDJ-3000
subucom; only the application payload differs.

`mot2bin` is a generic Motorola S-record → flat binary converter
(`cat motfile | mot2bin <start> <end> > binfile`). Yocto/Poky 2.1.3 aarch64
ELF, 15 KB, not subucom-specific - it's just bundled because the running OS
needs to convert .mot to .bin before writing it over SPI.

## Booting G2M under QEMU: what works, what doesn't

If you bypass the gate (e.g. by removing the rk3399 marker check), the G2M
kernel does boot cleanly on QEMU `virt` aarch64. The current
`extract_initramfs` already decompresses LZ4 legacy in addition to gzip, so the
G2M initramfs comes out intact. End-to-end you get:

- Yocto 2.1.3 init brings the system up.
- udev populates `/dev/dri/card0` and `/dev/fb0` against virtio-gpu.
- Network comes up via DHCP, SSH (dropbear) reachable.
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
- The launcher is `xserver-nodm.service` → `/etc/X11/Xserver`, not the
  `x11-only.sh` script the RK3399 path uses.

### What G2M support would entail

Roughly, in increasing order of scope:

1. **Patch out `StartFlipMode` in `fbdev_drv.so`** so it returns success without
   touching DRM. Visible tearing, but X comes up. Smallest scope.
2. **Drop in a stock Yocto 2.1.3 `fbdev_drv.so`** (no Pioneer rcar-du logic) -
   either rebuild `xf86-video-fbdev-0.4.4` against the right ABI or recycle one
   from a Poky 2.1.3 image archive.
3. **LD_PRELOAD shim** that intercepts `drmOpen("rcar-du", ...)` and translates
   `DRM_IOCTL_RCAR_DU_PAGE_FLIP` → `DRM_IOCTL_MODE_PAGE_FLIP` against
   virtio-gpu. Fits the existing `ep122_shim.so` pattern (patch 13).
4. **Whatever EP122 itself expects.** This is the unknown - the application
   binary almost certainly speaks to the kernel through more Renesas-specific
   ioctls (display, audio, GPIO) than the fbdev driver alone exposes.

None of this is small. The decision is to leave G2M unsupported for now.

## What stays in tree

The extraction pipeline retains a couple of G2M-friendly capabilities that are
cheap to keep:

- `extract_initramfs` scans for **both** gzip and LZ4 legacy magic and picks
  the largest valid cpio newc/crc result. This makes the extractor robust to
  the kernel's embedded `.config` (small gzip, not a cpio) and would Just Work
  for a G2M kernel if the gate were removed.
- The `read_firmware_info` path tolerates UPDs without `CDJ3K-RK3399.ISO` and
  falls back to the outer ISO, so version metadata can still be read from a
  legacy UPD even when the kernel itself is rejected later.

The gate lives in `extract_image_from_tar_gz` in
`crates/cdj3k-emu-firmware/src/extract.rs` - one branch deep, easy to remove.
