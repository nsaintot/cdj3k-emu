# cdj3k-emu - Models and slates

> What differs between the player models and where each difference lives.

---

## A model is a spec

`cdj3k_emu_panel::Model` names a player; its `ModelSpec` (`cdj3k.rs`,
`cdj3kx.rs`, `cdj1500x.rs`) carries everything that differs: framebuffer size,
touch space, the U-Boot `model` and system-revision variables, the eMMC's
device number, the `.UPD` file names, the MISO encoder, the MOSI decoder and
the LED profiles. Consumers read the spec rather than match on the model; the only
matches are `Model::spec`, the slate lookup (`slate::for_model`) and the panel
window's autosave name. Adding a player is a spec with its encoder, a `Model`
variant and a slate.

The MISO encoder is a `MisoCodec`: the UI describes the panel in a
model-neutral `PanelState` and the player's encoder writes its frame, so a
deck's quirks - an offset, a byte order, a tempo curve, where its power flag
rides - live in its own spec file rather than as flags in a shared codec.

The MOSI decoder is its mirror, a `MosiCodec`: the UI names a `Lamp` and the
player's decoder answers with a `LampState` (a bit, a step, a value or a
colour), or `None` for a lamp the player lacks. The CDJ-3000 and the
CDJ-3000X share one decoder over a `MosiMap` each; the CDJ-1500X has its own.

## Shell

`cdj3k_emu_ui::CdjShell` (`app/shell.rs`) hosts the picker, the setup window
(`SetupStep`) and the panel, and boots through the binary's `RuntimeHost`
(`app/cdj3k-emu/src/launch.rs`). The panel, `CdjApp`, outlives a model switch;
`CdjApp::switch_model` drops the shape caches and resets the scripted presses,
cleared bits, last input frame and LED state, which are offsets in the
previous model's frames.

Each slot is its own process and holds one installation; its model is
`model=` in the slot's `settings.txt`. How an install reaches the slot is in
[Storage](storage.md). AppKit frame autosave is keyed by slot and model, since
the two canvases have different aspects.

## Firmware

Every deck boots the same bundled 6.6 kernel through the same QEMU config,
`-smp 6` included: the CDJ-3000X firmware pins Xorg to core 5 and cryptsetup
to core 4, and a missing core fails those `taskset` calls. What differs is
the Pioneer image (rootfs and player app) and what the spec feeds into it.

| | CDJ-3000 | CDJ-3000X | CDJ-1500X |
|---|---|---|---|
| SoC | RK3399 | RK3399 | RK3566 |
| Player unit / process | `EP122.service` / `EP122` | `EP145.service` / `EP145` | `EP166.service` / `EP166` |
| `.UPD` name | `CDJ3K…` | `CDJ3000X…` | `CDJ1500X…`, `EP166…` |
| Initramfs in the `.UPD` | in the kernel of `images.tar.gz` | in the kernel of `images.tar.gz` | the ramdisk of the FIT `images/boot.img` |
| U-Boot `model` | `CDJ3K-RK3399` | `CDJ3000X` | the product UUID `6516d11b-…` |
| System revision variable | `rev_kernel` | `rev_system` | none |
| eMMC (`virtio_blk.emmc_index`) | `mmcblk1` | `mmcblk1` | `mmcblk0` |
| Framebuffer (`Model::main_lcd`) | 1280x720 | 1280x800 | 1280x800 |
| Touch | the sub-CPU frame, read by the app | a `gt928` input device the shim synthesises | as the CDJ-3000X |
| Sub-CPU device | `/dev/subucom_spi1.0` | `/dev/subucom_spi1.0` | `/dev/subucom_spi3.0` |
| `subucom_virt.ko model=` | `cdj3k` | `cdj3kx` (the shifted frame, below) | `cdj1500x` (a frame of its own, below) |

The CDJ-3000X's panel is 800x1280 portrait, rotated by `apl_start.sh` with
xrandr; on the virtual output those calls fall through and virtio-gpu scans
out landscape.

One patch set (`initramfs-patch/patch-rootfs.d/`) serves both images. The
dispatcher (`patch-rootfs.sh`) finds the player's unit and exports `APP_UNIT`,
`APP_NAME` and `APP_SLUG`; the steps order their services against that unit,
install the LD_PRELOAD drop-in under it and pass the slug on. Where the two
images differ, a step follows what the rootfs holds rather than the model:

- Patch 25 keeps the stock Xorg `taskset` pin (core 3 on the CDJ-3000, core 5
  on the CDJ-3000X) only when the guest has that core.
- Patch 23 creates `/var/net_mlan0_state.dat` only when the image carries
  `wlan-monitor.sh` (the CDJ-3000X's and the CDJ-1500X's do), and masks
  whichever unit runs it: `start-mlan0.service` on the CDJ-3000X,
  `start-wlan.service` on the CDJ-1500X.
  It also gives those images an `mlan0`: the kernel's built-in dummy
  interface renamed at boot, down, with eth0's address made local. Without
  one, service mode's version page re-runs `ifconfig mlan0` twice a second.
- Patch 14 copies `cabinet.img` onto the settings partition only when the
  install staged one, which it does when the image carries
  `images/cabinet.img`.
- Patch 20 waits on the player's own process name before unmounting.

A few steps follow the slug, where the images do not tell:

- Patch 02 installs the bundled `dropbear_aarch64` on the CDJ-1500X, which
  ships no SSH server.
- Patch 14 reads the staged cabinet from the partition the deck's layout
  leaves free (see [Storage](storage.md)).
- Patch 16 sets Xorg's `AccelMethod` to `none` on the CDJ-1500X, whose
  modesetting build cannot allocate EXA pixmaps on virtio-gpu.
- Patch 26 makes the CDJ-1500X's `Startup` read a missing ERP_RW GPIO as LOW;
  otherwise it starts the USB updater, which `pre-setting.service` waits on.

In the shim, `core/` serves every deck; `cdj3kx/touch.c` acts only when
`deck_model()` resolves to the CDJ-3000X or the CDJ-1500X.

## CDJ-3000X sub-CPU frames

Both frames are the CDJ-3000's shifted. The CDJ-3000X's MISO encoder
(`cdj3kx::Miso`) writes the CDJ-3000's fields (`cdj3k::write_fields`) and
buttons 4 bytes on, and its `MosiMap` (`cdj3kx::MOSI`) says where each lamp
sits; the codecs are the only code that turns a button or a lamp into an
offset.

- MISO: the whole CDJ-3000 frame sits behind a u16 format version, which must
  be non-zero or the app's `RxDataDispatcher::dispatch` drops the frame; the
  CRC stays at the end. Button bits are the CDJ-3000's, confirmed against the
  CDJ-3000X's reactions and against its `subucom_read`, whose service
  combination `subucom_virt.ko model=cdj3kx` injects for Service Mode. The
  CDJ-3000's SD-cover bit is a button here (`Btn::Usb2Stop`).
- Power: once the power-on bit has been set, the CDJ-3000X runs its shutdown
  preparation when it drops. The guest forwarder checks power at the
  CDJ-3000's position on both RK3399 decks, so the encoder drops it there
  too.
- MOSI: the bitfield and the RGB triplets are shifted. The jog-ring level is a
  byte of its own, and PLAY, CUE and the selector ring are RGB lamps where the
  CDJ-3000 has single bits.

Touch: EP145 reads a Goodix `gt928` input device, not the frame's touch words.
The shim's `cdj3kx/touch.c` synthesises it from those words, which the host
fills in screen pixels (`Model::touch_units`) with `TOUCH_DOWN` set in the
x-coordinate word, since pixel 0 is a coordinate.

## CDJ-1500X sub-CPU frames

Both frames are the CDJ-1500X's own, 32 bytes on `/dev/subucom_spi3.0`; the
codecs (`cdj1500x::Miso`, `cdj1500x::Mosi`) hold the layout.

- MISO: a non-zero format version at byte 0, data to byte 23, the
  CRC-16/X-25 big-endian at 24-25 (`subucom_virt.ko model=cdj1500x` stamps
  the same).
- The platter has no JOG ADJUST knob: its brake is fixed (`JogBrake::Fixed`).

## Slates

```
crates/cdj3k-emu-ui/src/app/ui.rs        colours, UiScale, DebugSnapshot, draw_ui dispatch
crates/cdj3k-emu-ui/src/app/ui/
  draw_primitives{,/buttons}.rs          buttons, rotaries, borders (shared)
  draw_cache.rs                          static + button shape caches (shared)
  draw_jog{,/geometry,/statics}.rs       the jog assembly (shared)
  slate.rs                               Slate, for_model(model)
  slate/cdj3k.rs  + cdj3k/               REF 3185 x 4360
  slate/cdj3kx.rs + cdj3kx/              REF 3336 x 4659
  slate/cdj1500x.rs + cdj1500x/          REF 2440 x 3626
```

A slate exposes `REF_W`/`REF_H` and `draw_panel(app, ui)`: it fits its canvas
(`UiScale::fit`), paints the chassis and its sections - free functions over
`&mut CdjApp`, so two slates can name them alike - and places the shared jog
at `layout::jog_rect()` with a `JogChrome`. Zones are composed in each slate's
`layout.rs`. The CDJ-1500X's slate draws its own jog, keys and knobs instead,
and can tilt to show its front face and USB tray: `ui/tilt.rs` projects its
shapes and `app/tilt_input.rs` maps the pointer back onto the plan.

