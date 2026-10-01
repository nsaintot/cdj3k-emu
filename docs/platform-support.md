# Platform support

The emulator runs on **macOS** (Apple silicon), **Linux** (aarch64 and
x86_64) and **Windows** (arm64 and x64, native Win32). The guest is the same
aarch64 payload everywhere (kernel `Image`, modules, shim, static tools) and is
built once by `docker/Dockerfile`. Only the host half differs, and it differs
only where the table below says so.

A host with no adapter of its own compiles against each module's
`unsupported.rs`: it builds and boots under TCG with no audio, bridging, USB
passthrough or native window integration.

## What differs per host

| Feature | macOS | Linux | Windows | Other hosts |
| --- | --- | --- | --- | --- |
| Accelerator | HVF; in-kernel vGIC on macOS 15+, emulated GIC otherwise | KVM on aarch64 with a readable `/dev/kvm` | WHPX on arm64 (Windows 11 24H2, build 26100.3915+, Hypervisor Platform on); in-kernel vGICv3 | none |
| No accelerator | — | TCG on x86_64, or when `/dev/kvm` is missing or unreadable | TCG on x64, or when WHPX is off or the build is older | TCG |
| QEMU process | re-exec of the app with `--qemu-worker`, QEMU linked as `libcdj3k-emu-qemu.dylib` | `qemu-system-aarch64` shipped beside the app | `qemu-system-aarch64.exe` in `bin\`, in a kill-on-close job object | shipped executable |
| Audio backend | `coreaudio`, device by UID | `pipewire`, sink by node name, `out.fixed-settings=off` | `wasapi`, endpoint by IMMDevice id (`out.dev=`) | no `-audiodev` |
| Low-latency path | host bypass ring (patch 08), drained by the IOProc | the same ring, drained by the PipeWire process callback (patch 14) | the same ring, drained by the WASAPI event thread (patch 18) | — |
| Output device list | CoreAudio HAL | `pw-dump` | IMMDeviceEnumerator | empty |
| Host-only network | vmnet-host | — | — | — |
| Bridged network | vmnet-bridged on a NIC; a `tap*` through `tapbridge` | macvtap on a NIC, a tap of ours on a bridge, or an existing tap | a TAP-Windows6 adapter per slot, joined to a NIC with `netsh bridge` (Windows 11 22H2 + KB5030310); by hand on older builds | — |
| Physical USB | DiskArbitration unmount; QEMU opens `/dev/diskN` after a one-time `chmod` prompt | udisks unmount; the app opens the disk `O_EXCL` (udisks `OpenDevice` under polkit when refused) and hands QEMU the fd with QMP `add-fd` | volumes locked and dismounted by the app; a one-time elevated ACL grant, then QEMU opens `\\.\PhysicalDriveN` | — |
| Virtual USB format | `hdiutil` + `diskutil` | `sfdisk` + `mkfs.exfat` on the file | VHDX: `CreateVirtualDisk`, then an elevated `Mount-DiskImage` + `Format-Volume` | — |
| Elevation | `osascript` admin prompt | `pkexec` | UAC (`ShellExecuteExW` `runas`) | refused |
| Menu | native menu bar (`muda`) | in-window strip (egui) | in-window strip (egui), also the title bar: an undecorated window, caption answered in `WM_NCHITTEST` | in-window strip |
| Window aspect | `contentAspectRatio` | Wayland: patched winit fit; X11: `WM_NORMAL_HINTS` | `WM_SIZING` on a subclassed HWND | settle snap only |
| Panel frame kept | AppKit autosave: size and position | `window-frames.txt`: size, and position where the window system reports one (not Wayland) | `window-frames.txt`: size and position | — |
| File picker, reveal | `NSOpenPanel`, `open -R` | desktop portal (rfd), `FileManager1.ShowItems` | `IFileDialog` (rfd), `explorer /select,` | — |
| Haptics, PC Link | trackpad actuator; IOHIDUserDevice + CoreMIDI driver | — | — | — |
| App data | `~/Library/Application Support/<bundle id>` | `$XDG_DATA_HOME/<bundle id>` | `%LOCALAPPDATA%\<bundle id>` | — |
| Runtime dir | `/tmp/cdj3k-emu-<euid>`, 0700 | same | `%TEMP%\cdj3k-emu` | — |

Common to every host: user-mode NAT, the shm display and jog streams, virtio
serial over AF_UNIX sockets (Windows 10 1803+ included), the eMMC claim
(`flock`, `LockFileEx`), and guest provisioning inside a throwaway QEMU boot.

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
| a polkit agent on a local seat | bridged networking (`pkexec`), once per bridged start |

A macvtap cannot reach its own host, so rekordbox on the same Linux machine
does not see a deck bridged on a NIC. Pick a bridge built by hand instead.

## Windows requirements

| Requirement | For |
|---|---|
| Windows 10 2004 (build 19041) or later | the installer's floor; AF_UNIX sockets |
| 3 GiB free under `%TEMP%` | guest RAM (`memory-backend-file`, sparse) |
| 29.1 GB free on NTFS | the eMMC image (sparse) |
| Windows 11 24H2 build 26100.3915+, Hypervisor Platform | WHPX on arm64; TCG otherwise |
| Windows 11 22H2 + KB5030310 | `netsh bridge`; on older builds the bridge is made by hand |
| an administrator account | UAC: the bridge, the first USB passthrough, VHDX formatting |

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

**Windows:** one Inno Setup 6.6+ installer per architecture,
`cdj3k-emu-<version>-windows-{x64,arm64}-setup.exe`. The app is cross-built for
`<arch>-pc-windows-gnullvm` by `packaging/windows/cross-build.sh` in the
`docker/Dockerfile.windows-qemu` image, into `dist/windows-bin-<arch>/` with
the DLLs it loads; `packaging/windows/build.ps1` on Windows runs `stage.ps1`,
then ISCC.

- Display name "CDJ3K Emulator"; per-machine under `%ProgramFiles%\cdj3k-emu`,
  administrator required, Windows 10 2004 (19041) or later. The
  tree is the Linux prefix with `bin\` holding the executables beside QEMU's
  DLLs and `share\cdj3k-emu\` the payload, so `bundled::resources()` needs no
  Windows entry. The x64 installer refuses arm64 Windows and the reverse.
- TAP-Windows6 9.27.0 (`OemVista.inf`, `tap0901.{sys,cat}`) comes from the
  pinned OpenVPN release, sha256-checked by `stage.ps1`, and goes in through
  `pnputil` unless the driver store holds that version or newer. The uninstaller removes adapters named `cdj3k-emu-*`; it deletes
  the driver package only if the installer added it (`installer\tap-owned`),
  no other tap0901 device remains and OpenVPN is absent.
- arm64 only: `HypervisorPlatform` is enabled through DISM when off; a pending
  restart (exit 3010) makes Setup ask for one.
- Inbound firewall rules (UDP, TCP, all profiles) for `qemu-system-aarch64.exe`,
  removed on uninstall. Signing is on when `CDJ3K_SIGN_CERT_SHA1` is set.
- The app holds the `Global\cdj3k-emu` mutex while it runs; it is the
  installer's `AppMutex`. Apps & features runs the uninstaller with
  `/SILENT /ASK`: its own confirmation, with an option to delete
  `%LOCALAPPDATA%\com.cdj3k.emu`.
- `stage.ps1` needs Git for Windows: its bash builds the patch dispatcher, so
  `.sh` files must be checked out with LF (`.gitattributes`).

`bundled::resources()` finds the payload in this order: `CDJ3K_RESOURCES` (set
by the AppImage's `AppRun`), `<exe>/../Resources` (the `.app`),
`<exe>/../share/cdj3k-emu` (the Linux prefix), `<exe>/resources` (a dev
mirror).

QEMU is built by `qemu/build.sh` at the pinned `QEMU_REF` with
`qemu/patches/*.patch` applied in order: `--enable-hvf --enable-cocoa` on
macOS, `--enable-kvm --enable-pipewire` on Linux.

For Windows, `qemu/build.sh --windows x86_64|aarch64` builds `--enable-slirp`
(plus `--enable-whpx` on aarch64) into `qemu/install-windows-<arch>/`:
`qemu-system-aarch64.exe`, `qemu-img.exe` and every DLL they load
(`scripts/bundle-dlls.sh` fails on one it cannot find). From Linux or macOS it
cross-builds in the `docker/Dockerfile.windows-qemu` image (llvm-mingw, UCRT,
both architectures); in an MSYS2 UCRT64 or CLANGARM64 shell it builds natively.
Patches 15-17 give Windows `memory-backend-file`, the shm display's file
mapping, and `ivshmem-plain`; `ivshmem-doorbell` and its server are POSIX-only.
Patch 18 adds the `wasapi` audiodev (`out.dev` = the endpoint ID, absent =
default render endpoint): a shared-mode, event-driven render client, low-latency
through `IAudioClient3` where the endpoint has it, draining the same bypass ring
from its MMCSS render thread, and reopening the endpoint when it is invalidated.
Patch 19 gives a Windows host disk a fixed length, so a read does not query
the device size. Patch 20 has a halted TCG vCPU poll before it sleeps and a
mutex spin before it blocks: each Windows wait and wake is a kernel round trip.
Patch 21 (every host) releases a TX buffer that reaches the bypass writer
before the stream's START at once, and has RELEASE flush the deferred-return
list.

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
#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod imp;
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
#[path = "unsupported.rs"]
mod imp;
```

The `cfg` lives in the `mod.rs` alone. Leaf files are plain code for their
host, and `unsupported.rs` answers "not here" rather than borrowing another
host's behaviour.

| Module | Crate | What it selects |
|---|---|---|
| `host/` | platform | accelerators, audio backend, entropy object, key labels |
| `desktop/` | platform | window shape, aspect lock, frame, picker, reveal |
| `net/` | platform | interfaces a guest can be put on, host-only |
| `audio/` | platform | output device enumeration (`coreaudio.rs`, `pipewire.rs`, `windows.rs`) |
| `haptic/` | platform | trackpad actuator |
| `menu/` | platform | native bar (`muda_provider`) or strip (`egui_provider`) |
| `app_dirs/`, `runtime_paths/` | platform | where data and sockets live |
| `local_socket/`, `file_lock/` | platform | chardev sockets, the eMMC claim |
| `disk/` | runtime | removable disks, raw open, image formatting, retry prompt |
| `net/` | runtime | bridging (`vmnet`, `tapbridge`, `linux_net`, `windows_net`), the tap netdev |
| `elevate/` | runtime | running one command as root |
| `pc_link/` | runtime | HID and MIDI endpoints |
| `process/`, `qemu_exec/` | runtime | starting and signalling QEMU, exit hooks |
| `sparse_file/` | platform | sparse guest RAM, jog shm and eMMC scratch files |
| `qmp/fd_passing/` | runtime | QMP `add-fd` (Unix only) |

Capabilities that are a transport rather than an OS (`local_socket`,
`file_lock`, `process`, `sparse_file`) select `unix.rs`, `windows.rs` or
`unsupported.rs`. Pure logic a leaf depends on (`host/whpx.rs`,
`desktop/aspect_fit.rs`, `disk/windows_parse.rs`, `net/winbridge.rs`) compiles
on every host so its tests run on every host.
