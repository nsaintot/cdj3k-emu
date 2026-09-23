# cdj3k-emu - Models and slates

> What differs between the player models and where each difference lives.

---

## A model is a spec

`cdj3k_emu_panel::Model` names a player; its `ModelSpec` (`cdj3k.rs`,
`cdj3kx.rs`) carries everything that differs: framebuffer size, touch space,
the U-Boot `model` variable, the `.UPD` file names, the MISO/MOSI maps and
the LED profiles. Consumers read the spec rather than match
on the model; the only matches are `Model::spec`, the slate lookup
(`slate::for_model`) and the panel window's autosave name. Adding a player is a
spec, a `Model` variant and a slate.

## Shell

`cdj3k_emu_ui::CdjShell` (`app/shell.rs`) hosts the picker, the setup window
(`SetupStep`) and the panel, and boots through the binary's `RuntimeHost`
(`app/cdj3k-emu/src/launch.rs`). The panel, `CdjApp`, outlives a model switch;
`CdjApp::switch_model` drops the shape caches.

Each slot is its own process and holds one installation; its model is
`model=` in the slot's `settings.txt`. How an install reaches the slot is in
[Storage](storage.md). AppKit frame autosave is keyed by slot and model, since
the two canvases have different aspects.

## Firmware

Both decks boot the same bundled 6.6 kernel through the same QEMU config,
`-smp 6` included: the CDJ-3000X firmware pins Xorg to core 5 and cryptsetup
to core 4, and a missing core fails those `taskset` calls. What differs is
the Pioneer image (rootfs and player app) and what the spec feeds into it.

| | CDJ-3000 | CDJ-3000X |
|---|---|---|
| Player unit / process | `EP122.service` / `EP122` | `EP145.service` / `EP145` |
| `.UPD` name | `CDJ3K…` | `CDJ3000X…` |
| U-Boot `model` | `CDJ3K-RK3399` | `CDJ3000X` |
| Framebuffer (`Model::main_lcd`) | 1280x720 | 1280x800 |
| Touch | the sub-CPU frame, read by the app | a `gt928` input device the shim synthesises |
| `subucom_virt.ko model=` | `cdj3k` | `cdj3kx` (the shifted frame, below) |

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
  `wlan-monitor.sh`, which only the CDJ-3000X's does.
- Patch 14 copies `cabinet.img` onto the settings partition only when the
  install staged one, which it does when the image carries
  `images/cabinet.img`.
- Patch 20 waits on the player's own process name before unmounting.

In the shim, `core/` serves both decks; `cdj3kx/touch.c` acts only when
`deck_model()` resolves to the CDJ-3000X.

## CDJ-3000X sub-CPU frames

Both frames are the CDJ-3000's shifted. The CDJ-3000X's `MisoMap` and `MosiMap`
(`cdj3kx.rs`) say by how much and where each control and lamp sits;
the `miso_frame` and `mosi_frame` codecs are the only code that turns one
into an offset.

- MISO: the whole CDJ-3000 frame sits behind a u16 format version, which must
  be non-zero or the app's `RxDataDispatcher::dispatch` drops the frame; the
  CRC stays at the end. Button bits are the CDJ-3000's, confirmed against the
  CDJ-3000X's reactions and against its `subucom_read`, whose service
  combination `subucom_virt.ko model=cdj3kx` injects for Service Mode. The
  CDJ-3000's SD-cover bit is a button here (`Btn::Usb2Stop`).
- Power: once the power-on bit has been set, the CDJ-3000X runs its shutdown
  preparation when it drops. The guest forwarder checks power at the
  CDJ-3000's position on both models, so `set_power` mirrors it there
  (`MisoMap::power_mirrors`).
- MOSI: the bitfield and the RGB triplets are shifted. The jog-ring level is a
  byte of its own, and PLAY, CUE and the selector ring are RGB lamps where the
  CDJ-3000 has single bits.

Touch: EP145 reads a Goodix `gt928` input device, not the frame's touch words.
The shim's `cdj3kx/touch.c` synthesises it from those words, which the host
fills in screen pixels (`Model::touch_units`) with `TOUCH_DOWN` set in the
x-coordinate word, since pixel 0 is a coordinate.

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
```

A slate exposes `REF_W`/`REF_H` and `draw_panel(app, ui)`: it fits its canvas
(`UiScale::fit`), paints the chassis and its sections - free functions over
`&mut CdjApp`, so two slates can name them alike - and places the shared jog
at `layout::jog_rect()` with a `JogChrome`. Zones are composed in each slate's
`layout.rs`.

