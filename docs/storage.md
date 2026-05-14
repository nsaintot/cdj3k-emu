# cdj3k-emu - Storage

> Reference for how the emulator persists state on the host: the per-instance
> eMMC qcow2 image, the hot-swappable USB drive slot, and the plain-text
> settings files.

---

## Overview

Three classes of persistent storage:

| Class | Scope | Backing | Hot-swap |
|---|---|---|---|
| eMMC qcow2 | One per instance (0..3) | qcow2 file under app data | No (boot-time) |
| USB drive slot | One per instance | virtual `.img` or physical `/dev/diskN` | Yes (QMP) |
| Settings | App-wide + per-instance | plain `key=value` text | n/a |

Nothing Pioneer-owned is stored or shipped - the firmware install pipeline
runs locally and writes only into the user's app-data directory.

---

## App data layout (macOS)

```
~/Library/Application Support/com.cdj3k.emu/
  settings.txt                                   # app-wide
  instance-0/
    settings.txt                                 # per-instance
    emmc.qcow2                                   # the eMMC image
  instance-1/
    ...
  instance-2/
    ...
  instance-3/
    ...
```

The directory name is the macOS bundle identifier (reverse-DNS), not the
display name - see `crates/cdj3k-emu-storage/src/lib.rs:25` and
`cdj3k_emu_platform::app_meta::BUNDLE_ID`. On non-macOS hosts the base is
`$XDG_DATA_HOME` / `~/.local/share` (`lib.rs:14-26`).

USB images created by the in-app "Create blank USB" path are written to a
user-chosen location, not inside the app data directory.

---

## eMMC qcow2

A 29.1 GB sparse qcow2 image per instance (`EMMC_SIZE = 29 * GB + 100 * MB`,
`emmc.rs:37`). Layout mirrors the real CDJ-3000 `mmcblk1` so the Pioneer
init scripts find the partition numbers they expect.

| # | Size | Name | FS | Purpose |
|---|---|---|---|---|
| 1 | 4 MiB | bootloader | raw | U-Boot binary slot. U-Boot env at offset `0x003f8000` (see below) |
| 2 | 4 MiB | trustfirmware | raw | ARM TF-A (BL3X) |
| 3 | 4 MiB | resource | raw | Rockchip resource blob (RSCE) |
| 4 | 128 MiB | recovery | raw | Recovery firmware slot (FAT32 on real hw, zeroed here) |
| 5 | 256 MiB | firmware-a | raw | App firmware slot A |
| 6 | 256 MiB | firmware-b | raw | App firmware slot B |
| 7 | 64 MiB | settings | (ext4) | `/home/root/settings` - formatted by guest on first boot |
| 8 | ~28.4 GiB | userdata | (ext4) | `/mnt` - rekordbox cache, formatted by guest on first boot |

All partitions are 1 MiB-aligned (start LBA aligned to 2048); the backup
GPT reserves `1 + (128 * 128 / 512) = 33` sectors at end of disk
(`gpt.rs:22`). Partition type GUID is the Linux filesystem data type for
every entry.

### U-Boot environment block

A valid U-Boot env image is written at byte offset `0x003f8000`, size
`0x8000`, matching `/etc/fw_env.config` inside the firmware
(`emmc.rs:40-41`). Format:

```
[ CRC32-LE (4 B) ] [ key=value\0 ... \0\0 ] [ zero padding to 0x8000 ]
```

CRC32 (poly `CRC_32_ISO_HDLC`) covers bytes `[4..0x8000]`. Variables include
`bootcmd_emmc`, `bootargs_emmc`, `kernel_addr_r`, `fdt_addr_r`, the per-
instance `serial_number` (`DJMP{instance_id:06}EH`), and firmware metadata
read from the .UPD ISO (`miniloader` MD5, `release`, `rev_apl`,
`rev_kernel`) - see `emmc.rs:95-160`.

### First-boot formatting

Partitions p7 and p8 are left unformatted at provisioning time. On first
boot the guest's `settings-mount.sh` detects the blank superblock signature
and runs `mkfs.ext4` against each. This is why first boot is noticeably
slower than subsequent boots; once the filesystems exist they survive every
launch.

### Slot index

Pioneer's patched `virtio_blk.c` probes devices in **reverse** mmio slot
order, so the listing order in QEMU's argv matters. The runtime emits
`usb0` first, then `emmc0`, so the eMMC ends up at probe index 0 →
`/dev/mmcblk1` (the path the Pioneer init scripts expect) and USB lands at
index 1 → `/dev/sdb`. See `runtime/src/config.rs:339-360` for the comment
and the argv lines.

### Sparseness

qcow2 is created from a sparse raw scratch file by `qemu-img convert -O
qcow2 -o preallocation=off` (`emmc.rs:219-238`). Host disk usage stays
near-zero until the guest writes data; only the GPT, the U-Boot env block,
and (after first boot) the ext4 superblocks consume sectors.

### `default_path`

```
~/Library/Application Support/<BUNDLE_ID>/instance-{N}/emmc.qcow2
```

- `emmc.rs:64-68`. `provision_emmc` is idempotent: it returns early if the
file already exists (`emmc.rs:72-89`).

---

## USB drive slot

Always present in QEMU's argv, regardless of whether the user has attached
anything:

```
-drive   file=<placeholder>,if=none,id=usb0,format=raw,
         cache=writeback,file.locking=off
-device  virtio-blk-device,drive=usb0,id=usb0
```

(`runtime/src/config.rs:352-360`). The placeholder is a 1-sector raw file
so the virtio-blk device shows up to the guest even when empty.

### Hot-swap protocol

`UsbManager` (`runtime/src/usb.rs:282-468`) drives the swap via QMP:

1. `blockdev-change-medium id=usb0 filename=<new> format=raw` swaps the
   backing file with QEMU running.
2. QEMU fires `virtio_notify_config`. Pioneer's `virtblk_config_changed` →
   `revalidate_disk` makes the new size + partition table visible in the
   guest.
3. A 600 ms sleep gives the driver time to settle, then `cfgd` (over the
   `cdj3k.cfg` virtio-serial port) runs `usb-external-attach.sh` which
   mounts the FS and pokes `/proc/udev_usb1` - the interface EP122
   actually watches for USB state (`usb.rs:461-467`).

Detach is the reverse: `cfgd` runs `usb detach` (writes `umount /dev/sdb`
to `/proc/udev_usb1`, lazy-unmounts), 400 ms wait, then the placeholder is
swapped back in. If the previous medium was a physical disk, the host
remounts it on macOS (`usb.rs:444-456`).

### Virtual mode

User picks a `.img` file (or runs "Create blank USB" which makes an
exFAT-formatted raw image via macOS `hdiutil attach -nomount` +
`diskutil eraseDisk ExFAT REKORDBOX MBR`, `usb.rs:211-257`). The path is
persisted as `usb_virtual_path` in the per-instance `settings.txt`
(`settings.rs:120`).

### Physical mode

User picks a macOS BSD disk (`disk2`). The host-side flow
(`usb.rs:342-401`):

1. `MacOsDiskProvider::unmount_host` - `diskutil unmountDisk` so QEMU can
   open the device O_EXCL.
2. Probe `O_RDWR` from the runtime process. `/dev/diskN` is mode `0640`;
   on `EACCES` the runtime prompts via `osascript` for an admin-privilege
   `chmod 660 <dev>`. The dev path is regex-validated
   (`is_valid_bsd_disk_path`, `usb.rs:106-121`) before being interpolated
   into the AppleScript shell command.
3. `blockdev-change-medium` against `/dev/diskN`. On failure the host
   volume is remounted to leave the system clean.
4. The BSD name is persisted as `usb_physical_bsd` so the choice survives
   restarts (the disk is re-attached on next launch if still present).

`MacOsDiskProvider` lives in `runtime/src/macos_disk.rs` and exposes
`list_removable`, `unmount_disk`, `mount_disk` (lines 288/336/344).

---

## `file.locking=off`

Both `-drive` lines pass `file.locking=off`. The runtime owns exclusive
access at the `.app` level (one instance owns its directory) and does not
rely on QEMU's fcntl byte-range locks at offset 100.

The locks fire on `open()` of qcow2/raw files. If the previous QEMU
subprocess hasn't fully released its FDs by the time the new one starts
(common on rapid restart, since process teardown is async on macOS), the
new process aborts with `Failed to lock byte 100`. Disabling them removes
that race. Comment at `runtime/src/config.rs:348-351`.

---

## Settings persistence

Plain text, one `key=value` per line. No serde dependency - the value
space is tiny and the files are human-editable for debugging.
Implementation: `crates/cdj3k-emu-storage/src/settings.rs`.

### Forward-compat

`save()` re-reads the file into a `BTreeMap`, overwrites only the keys it
knows about, then writes the whole map back (`settings.rs:80-86`,
`193-229`). Unknown keys are preserved - downgrading or running a
side-build does not drop newer fields.

### Per-instance keys

`~/Library/Application Support/com.cdj3k.emu/instance-N/settings.txt`

| Key | Type | Default | Notes |
|---|---|---|---|
| `mac` | `xx:xx:xx:xx:xx:xx` | generated LAA | First byte forced to `02` (LAA unicast). Generated on first load if absent or invalid. |
| `audio_enabled` | `0` / `1` | `0` | Gates `virtio-snd` in argv |
| `audio_device_uid` | CoreAudio UID or empty | empty | Output device for `-audiodev coreaudio,out.device-uid=...` |
| `alc_enabled` | `0` / `1` | `1` | Pushes `audio_sync_enabled` to guest sysfs at boot |
| `haptic_enabled` | `0` / `1` | `1` | Gates Force Touch detent clicks |
| `net_iface` | `en0` etc. or empty | empty | Selected Pro DJ Link interface |
| `usb_virtual_path` | path or empty | empty | Last attached virtual USB image |
| `usb_physical_bsd` | `disk2` etc. or empty | empty | Last attached physical USB disk |

### App-wide keys

`~/Library/Application Support/com.cdj3k.emu/settings.txt`

| Key | Type | Default | Notes |
|---|---|---|---|
| `jog_adjust` | `f32` ∈ [0, 1] | `0.5` | Brake stop-time slider |
| `vinyl_speed` | `u8` | `0` | Vinyl-mode start/stop speed index |

(See `AppSettings`, `settings.rs:52-87`.) Firmware metadata captured from
the .UPD ISO is not stored here - it goes into the U-Boot env block
inside the eMMC image at provisioning time (`emmc.rs:119-127`).

---

## Firmware install path (short)

The in-app "Install Firmware" wizard
(`crates/cdj3k-emu-ui/src/app/firmware_wizard.rs`) takes:

- a `.UPD` file (Pioneer's LUKS1-encrypted firmware update), and
- a LUKS keyfile (user-supplied - never shipped).

Pipeline:

1. `cdj3k-emu-firmware::luks::decrypt_upd` parses the LUKS1 header,
   runs PBKDF2 against each active key slot, AF-merges the recovered
   key material, verifies the master-key digest, and streams the
   plaintext ISO to a scratch file (`firmware/src/luks.rs:245-262`).
   Supports `aes-cbc-essiv:sha256` and `aes-xts-plain64`.
2. `extract::read_firmware_info` reads version metadata
   (`IMAGES/RELEASE.TXT`, `APP.REV`, `SYSTEM.REV`, MD5 of
   `MINILOADER.IMG`) into a `FirmwareInfo` struct.
3. `extract::extract_kernel` extracts the rk3399 kernel image. Builds
   without an rk3399 marker are rejected with
   `ExtractError::UnsupportedG2M` - Renesas R-Car (G2M) firmware is
   out of scope (see `docs/g2m-renesas.md`).
4. `initramfs::extract_initramfs` / `patch_initramfs` apply the
   numbered patch scripts under `initramfs-patch/patch-rootfs.d/`
   (Dropbear enablement, cfgd installation, etc.).
5. `provision_emmc(EmmcConfig)` creates `instance-N/emmc.qcow2` with
   the partition table + U-Boot env containing the captured
   `FirmwareInfo`.

Pioneer-owned material never leaves the user's app-data directory.
Nothing is committed to the repo or bundled in the `.dmg`.

---

## Files

| Path | Role |
|---|---|
| `crates/cdj3k-emu-storage/src/lib.rs` | `app_data_dir`, public re-exports |
| `crates/cdj3k-emu-storage/src/emmc.rs` | Provisioning, partition layout, U-Boot env |
| `crates/cdj3k-emu-storage/src/gpt.rs` | Pure-Rust GPT writer (protective MBR + primary + backup) |
| `crates/cdj3k-emu-storage/src/settings.rs` | `AppSettings`, `InstanceSettings` |
| `crates/cdj3k-emu-runtime/src/usb.rs` | `UsbManager` hot-swap, virtual + physical attach |
| `crates/cdj3k-emu-runtime/src/macos_disk.rs` | `list_removable`, `unmount_disk`, `mount_disk` |
| `crates/cdj3k-emu-runtime/src/config.rs` | `-drive` argv lines (USB then eMMC) |
| `crates/cdj3k-emu-firmware/src/luks.rs` | LUKS1 decrypt of the .UPD payload |
| `crates/cdj3k-emu-firmware/src/extract.rs` | Kernel + `FirmwareInfo` extraction, G2M reject |
| `crates/cdj3k-emu-ui/src/app/firmware_wizard.rs` | "Install Firmware" wizard |
