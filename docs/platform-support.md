# Platform support

The emulator runs on **macOS** (Apple silicon) and **Linux** (aarch64 and
x86_64). The guest is the same aarch64 payload everywhere (kernel `Image`,
modules, shim, static tools) and is built once by `docker/Dockerfile`. Only the
host half differs, and it differs only where the table below says so.

A host with no adapter of its own compiles against each module's
`unsupported.rs`: it builds and boots under TCG with no audio, bridging, USB
passthrough or native window integration. The workspace checks clean for
`x86_64-pc-windows-msvc` on that basis. It does not run there yet.

## What differs per host

| Feature | macOS | Linux | Other hosts |
|---|---|---|---|
| Accelerator | HVF; in-kernel vGIC on macOS 15+, emulated GIC otherwise | KVM on aarch64 with a readable `/dev/kvm` | none |
| No accelerator | — | TCG on x86_64, or when `/dev/kvm` is missing or unreadable | TCG |
| QEMU process | re-exec of the app with `--qemu-worker`, QEMU linked as `libcdj3k-emu-qemu.dylib` | `qemu-system-aarch64` shipped beside the app | shipped executable |
| Audio backend | `coreaudio`, device by UID | `pipewire`, sink by node name, `out.fixed-settings=off` | no `-audiodev` |
| Low-latency path | host bypass ring (patch 08), drained by the IOProc | the same ring, drained by the PipeWire process callback (patch 14) | — |
| Output device list | CoreAudio HAL | `pw-dump` | empty |
| Host-only network | vmnet-host | — | — |
| Bridged network | vmnet-bridged on a NIC; a `tap*` through `tapbridge` | macvtap on a NIC, a tap of ours on a bridge, or an existing tap | — |
| Physical USB | DiskArbitration unmount; QEMU opens `/dev/diskN` after a one-time `chmod` prompt | udisks unmount; the app opens the disk `O_EXCL` (udisks `OpenDevice` under polkit when refused) and hands QEMU the fd with QMP `add-fd` | — |
| Virtual USB format | `hdiutil` + `diskutil` | `sfdisk` + `mkfs.exfat` on the file | — |
| Elevation | `osascript` admin prompt | `pkexec` | refused |
| Menu | native menu bar (`muda`) | in-window strip (egui) | in-window strip |
| Window aspect | `contentAspectRatio` | Wayland: patched winit fit; X11: `WM_NORMAL_HINTS` | settle snap only |
| Panel frame kept | AppKit autosave: size and position | `window-frames.txt`: size, and position where the window system reports one (not Wayland) | — |
| File picker, reveal | `NSOpenPanel`, `open -R` | desktop portal (rfd), `FileManager1.ShowItems` | — |
| Haptics, PC Link | trackpad actuator; IOHIDUserDevice + CoreMIDI driver | — | — |
| App data | `~/Library/Application Support/<bundle id>` | `$XDG_DATA_HOME/<bundle id>` | `%LOCALAPPDATA%` (Windows) |
| Runtime dir | `/tmp/cdj3k-emu-<euid>`, 0700 | same | `%LOCALAPPDATA%\cdj3k-emu` (Windows) |

Common to every host: user-mode NAT, the shm display and jog streams, virtio
serial over local sockets, the eMMC `flock`, and guest provisioning inside a
throwaway QEMU boot.

## Software emulation (TCG)

`host::software_emulation()` names why a host runs under TCG; the in-window
strip shows it as a gauge beside the slot pill, with the reason on hover. A TCG guest gets
`cdj3k.snd_period=512` on its kernel command line: `insmod-virtio-snd` loads
`virtio_snd` with a 512-frame ALSA period instead of the player's 64, which a
TCG guest cannot wake for in time. Each QEMU TX buffer stays 1024 frames.

`CDJ3K_EMU_TCG` forces TCG on any host.

The guest kernel has `CONFIG_RANDOMIZE_BASE` off: KASLR switches on KPTI on
an A72, and under TCG every exception from EL0 then flushes the softmmu TLB.

## Linux requirements

| Requirement | For |
|---|---|
| `/dev/shm` with 3 GiB free | guest RAM (`memory-backend-file`) |
| 29.1 GB free on a sparse-capable filesystem (`chattr +C` on btrfs) | the eMMC image |
| GL 3.3 / GLES 3.0, Wayland or X11 | the chassis |
| PipeWire | audio |
| udisks2 | USB passthrough and unmounting; polkit asks once per open |
| `exfatprogs`, `sfdisk` | formatting a virtual USB image |
| `kvm` group | acceleration on aarch64 |
| a polkit agent on a local seat | bridged networking (`pkexec`), once per host boot |

A macvtap cannot reach its own host, so rekordbox on the same Linux machine
does not see a deck bridged on a NIC. Pick a bridge built by hand instead.

## Packaging

**macOS:** one `.app` in a DMG. The `--qemu-worker` re-exec keeps the vmnet
entitlement on the process that opens vmnet; a nested helper binary cannot
carry it.

**Linux:** one staged tree (`packaging/linux/stage.sh`) packaged three ways
per architecture: `.deb`, `.rpm` (nfpm, `packaging/linux/nfpm.yaml.in`) and an
AppImage.

- Prefix `/opt/cdj3k-emu`, so the patched QEMU never lands in `/usr/bin`. The
  only files outside it are a `/usr/bin/cdj3k-emu` symlink, the `.desktop`
  entry and the icons.
- Everything that is ours or QEMU's is vendored with `$ORIGIN` rpaths
  (`scripts/bundle-sos.sh`). Nothing that talks to a driver, display server or
  sound daemon is, and those are the declared dependencies: GL/EGL, Wayland,
  xkbcommon, X11, PipeWire. `udisks2` and `exfatprogs` are recommended.
- One build per architecture; the x86_64 package describes itself as software
  emulation. There is no Flatpak or Snap: QEMU needs `/dev/kvm`, and bridging
  needs host privileges a sandbox would take away.

`bundled::resources()` finds the payload in this order: `CDJ3K_RESOURCES` (set
by the AppImage's `AppRun`), `<exe>/../Resources` (the `.app`),
`<exe>/../share/cdj3k-emu` (the Linux prefix), `<exe>/resources` (a dev
mirror).

QEMU is built by `qemu/build.sh` at the pinned `QEMU_REF` with
`qemu/patches/*.patch` applied in order: `--enable-hvf --enable-cocoa` on
macOS, `--enable-kvm --enable-pipewire` on Linux.

Patch 21 releases a TX buffer that reaches the bypass writer before the
stream's START at once, and has RELEASE flush the deferred-return list.

winit comes the same way: `winit/fetch.sh` downloads the pinned crates.io
release, checks its sha256, and applies `winit/patches/*.patch` into
`winit/src`, which `[patch.crates-io]` points at. Every cargo command needs it
first.

## Code layout

Every per-host difference sits behind a module that selects one file per
host:

```rust
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod imp;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
#[path = "unsupported.rs"]
mod imp;
```

The `cfg` lives in the `mod.rs` alone. Leaf files are plain code for their
host, and `unsupported.rs` answers "not here" rather than borrowing another
host's behaviour. Capabilities that are a transport rather than an OS
(`local_socket`, `file_lock`, `process`) select `unix.rs` or `unsupported.rs`.

| Module | Crate | What it selects |
|---|---|---|
| `host/` | platform | accelerators, audio backend, entropy object, key labels |
| `desktop/` | platform | window shape, aspect lock, frame, picker, reveal |
| `net/` | platform | interfaces a guest can be put on, host-only |
| `audio/` | platform | output device enumeration (`coreaudio.rs`, `pipewire.rs`) |
| `haptic/` | platform | trackpad actuator |
| `menu/` | platform | native bar (`muda_provider`) or strip (`egui_provider`) |
| `app_dirs/`, `runtime_paths/` | platform | where data and sockets live |
| `local_socket/`, `file_lock/` | platform | chardev sockets, the eMMC claim |
| `disk/` | runtime | removable disks, raw open, image formatting, retry prompt |
| `net/` | runtime | bridging (`vmnet`, `tapbridge`, `linux_net`) |
| `elevate/` | runtime | running one command as root |
| `pc_link/` | runtime | HID and MIDI endpoints |
| `process/`, `qemu_exec` | runtime | starting and signalling QEMU, exit hooks |

Adding Windows means a `windows.rs` beside each `unsupported.rs` it replaces,
and an arm for it in that module's `mod.rs`.
