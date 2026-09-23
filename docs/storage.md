# cdj3k-emu - Storage

> Reference for how the emulator persists state on the host: the per-instance
> eMMC qcow2 image, the hot-swappable USB drive slot, and the plain-text
> settings files.

---

## Overview

Three classes of persistent storage:

| Class | Scope | Backing | Hot-swap |
|---|---|---|---|
| eMMC qcow2 | One per instance (1..4) | qcow2 file under app data | No (boot-time) |
| USB drive slot | One per instance | virtual `.img` or physical `/dev/diskN` | Yes (QMP) |
| Settings | Per-instance | plain `key=value` text | n/a |

Nothing Pioneer-owned is stored or shipped - the firmware install pipeline
runs locally and writes only into the user's app-data directory.

---

## App data layout (macOS)

```
~/Library/Application Support/com.cdj3k.emu/
  settings.txt                                   # app-wide
  instance-1/
    settings.txt                                 # per-instance
    Image                                        # vanilla kernel
    initramfs-patched.cpio.gz                    # patched rootfs
    emmc.qcow2                                   # the eMMC image
    .open                                        # the owning process's lock
    .staging/                                    # an install not yet applied
  instance-2/
    ...
  instance-3/
    ...
  instance-4/
    ...
```

A slot holds **one installation**, for the model recorded as its `model`
setting, with the three files at the slot root. The paths come from
`cdj3k_emu_storage::FirmwarePaths::new(slot)`; installing another model into
the slot replaces them, eMMC included. Settings, the MAC and the USB media
selection are per slot.

An install never touches the live files, so it runs beside a running
emulation, in this window's slot or another's. It writes into
`instance-N/.staging/` (`StagedFirmware`) while holding an exclusive `flock`
on `.staging/.lock`, and once every file is in place writes
`.staging/complete`: the model, the firmware release and the SoC serial the
new cabinet was keyed for. That record is the signal. The process that owns
the slot (`SlotClaim`, an `flock` on `instance-N/.open`) swaps the files in
and writes the three values into the slot's settings (`apply_staged`)
whenever the slot's emulation is not running - at once if nothing runs,
otherwise at its next start. A restart of a running emulation counts: with a
finished install waiting, the runtime worker retires instead of restarting
QEMU in place (`relaunch_requested`) and the shell launches again. Another
process asks for that restart with `request_restart`, which leaves
`.staging/restart` beside the record for the owner to find. The swap takes
the eMMC's `flock`, which the app's `QemuInstance` holds for as long as QEMU
runs.

The claim file holds its owner's pid (`slot_holder`): a second process
started for a slot another one owns brings that one's window forward and
exits.

An install that fails or is cancelled deletes what it wrote. One killed
part way leaves files with no record and a lock nobody holds - the OS drops
an `flock` with its process - and whoever finds them deletes them. A swap
cut short between its renames keeps its record, and no new install may start
until an apply has finished it. Deleting a slot also drops a finished install
that was never applied.

A slot with firmware but no `model` setting holds a CDJ-3000;
`adopt_unrecorded_slot` records it at startup.

The directory name is the macOS bundle identifier (reverse-DNS), not the
display name - see `cdj3k_emu_storage::app_data_dir` and
`cdj3k_emu_platform::app_meta::BUNDLE_ID`. On non-macOS hosts the base is
`$XDG_DATA_HOME` / `~/.local/share`.

USB images created by the in-app "Create blank USB" path are written to a
user-chosen location, not inside the app data directory.

---

## eMMC qcow2

A 29.1 GB sparse qcow2 image per instance (`emmc::EMMC_SIZE`). Layout mirrors
the real CDJ-3000 `mmcblk1` so the Pioneer init scripts find the partition
numbers they expect.

| # | Size | Name | FS | Purpose |
|---|---|---|---|---|
| 1 | 4 MiB | bootloader | raw | U-Boot binary slot, holding the U-Boot env (see below) |
| 2 | 4 MiB | trustfirmware | raw | ARM TF-A (BL3X) |
| 3 | 4 MiB | resource | raw | Rockchip resource blob (RSCE) |
| 4 | 128 MiB | recovery | raw | Recovery firmware slot (FAT32 on real hw). Holds the staged `cabinet.img` here - see below |
| 5 | 256 MiB | firmware-a | raw | App firmware slot A |
| 6 | 256 MiB | firmware-b | raw | App firmware slot B |
| 7 | 64 MiB | settings | (ext4) | `/home/root/settings` - formatted by guest on first boot |
| 8 | ~28.4 GiB | userdata | (ext4) | `/mnt` - rekordbox cache, formatted by guest on first boot |

All partitions are 1 MiB-aligned; the backup GPT sits at the end of the disk
(`gpt.rs`). Partition type GUID is the Linux filesystem data type for every
entry.

### U-Boot environment block

A valid U-Boot env image is written where `/etc/fw_env.config` inside the
firmware says it lives (`emmc::UBOOT_ENV_OFFSET` / `UBOOT_ENV_SIZE`): a
CRC32-LE of the rest, then NUL-separated `key=value` pairs. Variables include
`bootcmd_emmc`, `bootargs_emmc`, `kernel_addr_r`, `fdt_addr_r`, the per-
instance `serial_number` (`DJMP{instance_id:06}EH`), and firmware metadata
read from the .UPD ISO (`miniloader` MD5, `release`, `rev_apl`,
and the system revision under the model's `system_rev_env`: `rev_kernel` on
the CDJ-3000, `rev_system` on the CDJ-3000X, as each deck's updater writes
it) - see `emmc::write_uboot_env`.

### Cabinet image

`images/cabinet.img` is a LUKS container holding the Widevine keys and the
Device Library Plus key file (`cabinet/encryption/lsdk.dat`). On the deck the
vendor updater mounts p7 and copies it to `/home/root/settings/cabinet.img`;
`apl_start.sh` opens it on every boot with `genkey_pr | initoptenv`, deriving
the passphrase from the U-Boot `model` and the `/proc/cpuinfo` Serial (see the
SoC serial section below).

The emulator cannot write ext4 from the host, so the install stages the image
raw in the recovery partition behind a small ASCII header naming its size
(`emmc::stage_cabinet`). Guest patch 14 copies it onto the settings partition
once `emmc-fs.sh` has formatted and mounted it and before the app starts. An
existing `cabinet.img` of the staged size that opens as LUKS is kept, so the
copy survives later boots; a truncated or foreign one is replaced.

The deck derives the passphrase as `sha512hex((model + serial) x (N+1))`,
where `model` is the U-Boot variable, `serial` is the `/proc/cpuinfo` value and
`N` is `strtol` of the serial's last two hex digits. `genkey_pr` prints it and
`initoptenv` feeds it to `cryptsetup open`. Nothing re-keys the container at
boot, so the container has to carry a keyslot for the slot's own serial before
it is staged.

#### Keying the container for the slot

Every CDJ-3000X cabinet descends from the one container the firmware ships -
same UUID, same master key, a different keyslot. The shipped keyslot opens with
the factory passphrase `genkey_pb` computes,
`sha512hex(model_env + sha512hex(images/images.tar.gz))` (`vendor_passphrase`),
so the install needs the `images.tar.gz` from the same package. It opens that
slot to recover the master key, mints the slot's SoC serial, adds a keyslot for
`sha512hex((model_env + serial) x (N+1))` (`cabinet_passphrase`), and stages
the result (`luks/rekey.rs`, `add_keyslot`). Without `images.tar.gz`, or when
no factory slot opens, the container is staged unkeyed.

The master key is verified against the header's mk-digest before anything is
written, so a container no factory slot opens (another deck family, another
firmware's `images.tar.gz`) is staged unchanged rather than corrupted. Only
the free slot's header entry and its already-reserved material region are
touched; the payload is not.

Two environment variables are for working with a real deck's container:

| Variable | Effect |
|---|---|
| `CDJ3K_CABINET_IMG=<file>` | Stage that LUKS container instead of the firmware's own. It still gets a keyslot for this slot. |
| `CDJ3K_SOC_SERIAL=<16 hex>` | Pin the slot's SoC serial instead of minting one, so the guest reproduces a specific deck's passphrase. |

The install logs `[serial] SoC serial = <hex>`, `[cabinet] keyed for <hex> in
slot <n>` and `[cabinet] staging <n> MiB`; the guest then logs
`cabinet-install: <n> bytes -> /home/root/settings/cabinet.img` and
`cabinet-tmp-cabinet.mount: Succeeded`. Zero `CipherElements` errors in the
journal means the cabinet opened; eight means it stayed shut.

### First-boot formatting

Partitions p7 and p8 are left unformatted at provisioning time. On first
boot the guest's `emmc-fs.sh` detects the blank superblock signature
and runs `mkfs.ext4` against each. This is why first boot is noticeably
slower than subsequent boots; once the filesystems exist they survive every
launch.

### Slot index

Pioneer's patched `virtio_blk.c` probes devices in **reverse** mmio slot
order, so the listing order in QEMU's argv matters. The runtime emits
`usb0` first, then `emmc0`, so the eMMC ends up at probe index 0 →
`/dev/mmcblk1` (the path the Pioneer init scripts expect) and USB lands at
index 1 → `/dev/sdb`. See the comment over the `-drive` lines in
`runtime/src/config.rs`.

### Sparseness

qcow2 is created from a sparse raw scratch file by `qemu-img convert -O
qcow2 -o preallocation=off` (`emmc::convert_to_qcow2`). Host disk usage stays
near-zero until the guest writes data; only the GPT, the U-Boot env block,
and (after first boot) the ext4 superblocks consume sectors.

### `default_path`

```
~/Library/Application Support/<BUNDLE_ID>/instance-{N}/emmc.qcow2
```

`provision_emmc` is idempotent: it returns early if the file already
exists.

---

## USB drive slot

Always present in QEMU's argv, regardless of whether the user has attached
anything:

```
-drive   file=<placeholder>,if=none,id=usb0,format=raw,
         cache=writeback,file.locking=off
-device  virtio-blk-device,drive=usb0,id=usb0
```

(`runtime/src/config.rs`). The placeholder is a 1-sector raw file
so the virtio-blk device shows up to the guest even when empty.

### Hot-swap protocol

`UsbManager` (`runtime/src/usb.rs`) drives the swap via QMP:

1. `blockdev-change-medium id=usb0 filename=<new> format=raw` swaps the
   backing file with QEMU running.
2. QEMU fires `virtio_notify_config`. Pioneer's `virtblk_config_changed` →
   `revalidate_disk` makes the new size + partition table visible in the
   guest.
3. A 600 ms sleep gives the driver time to settle, then `cfgd` (over the
   `cdj3k.cfg` virtio-serial port) runs `usb-external-attach.sh` which
   mounts the FS and pokes `/proc/udev_usb1` - the interface EP122
   actually watches for USB state.

Detach is the reverse: `cfgd` runs `usb detach` (writes `umount /dev/sdb`
to `/proc/udev_usb1`, lazy-unmounts), 400 ms wait, then the placeholder is
swapped back in. If the previous medium was a physical disk, the host
remounts it on macOS.

### Virtual mode

User picks a `.img` file (or runs "Create blank USB" which makes an
exFAT-formatted raw image via macOS `hdiutil attach -nomount` +
`diskutil eraseDisk ExFAT REKORDBOX MBR`). The path is persisted as
`usb_virtual_path` in the per-instance `settings.txt`.

### Physical mode

User picks a macOS BSD disk (`disk2`). The host-side flow (`usb.rs`):

1. `MacOsDiskProvider::unmount_host` - `diskutil unmountDisk` so QEMU can
   open the device O_EXCL.
2. Probe `O_RDWR` from the runtime process. `/dev/diskN` is mode `0640`;
   on `EACCES` the runtime prompts via `osascript` for an admin-privilege
   `chmod 660 <dev>`. The dev path is regex-validated
   (`is_valid_bsd_disk_path`) before being interpolated
   into the AppleScript shell command.
3. `blockdev-change-medium` against `/dev/diskN`. On failure the host
   volume is remounted to leave the system clean.
4. The BSD name is persisted as `usb_physical_bsd` so the choice survives
   restarts (the disk is re-attached on next launch if still present).

`MacOsDiskProvider` lives in `runtime/src/macos_disk.rs` and exposes
`list_removable`, `unmount_disk` and `mount_disk`.

---

## `file.locking=off`

Both `-drive` lines pass `file.locking=off`. The runtime owns exclusive
access at the `.app` level (one instance owns its directory) and does not
rely on QEMU's fcntl byte-range locks at offset 100.

The locks fire on `open()` of qcow2/raw files. If the previous QEMU
subprocess hasn't fully released its FDs by the time the new one starts
(common on rapid restart, since process teardown is async on macOS), the
new process aborts with `Failed to lock byte 100`. Disabling them removes
that race (see the comment in `runtime/src/config.rs`).

---

## Settings persistence

Plain text, one `key=value` per line. No serde dependency - the value
space is tiny and the files are human-editable for debugging.
Implementation: `crates/cdj3k-emu-storage/src/settings/`.

### Key registries

`save()` re-reads the file into a `BTreeMap`, overwrites only the keys it
knows about, then writes the whole map back.

Each file has a registry of the keys it may hold - `SLOT_KEYS` for a slot's,
`APP_KEYS` for the app-wide one - and `write_kv` drops everything else, so a
key a release retires leaves the file at the next save. A registry belongs to
a file, not to a struct: `InstanceSettings` and `PanelSettings` share the slot
file, and either one pruning to its own keys would delete the other's - losing
`soc_serial` that way would lock the slot's `cabinet.img` out for good. The
cost of the registry is that a new field must be listed in it or it will not
survive its own save; the `the_slot_registry_covers_every_key_written` test
fails if one is missed.

### Concurrent writers

The UI, the menu thread and the runtime worker all persist per-instance
settings. Every write is atomic (temp file + `rename`, so a reader never
sees an empty file - an empty read would mint a new MAC) and every
load-modify-save - `InstanceSettings::update`, `PanelSettings::save` - holds
one process-wide lock across the read and the write so two threads cannot
lose each other's fields.

### Per-instance keys

`~/Library/Application Support/com.cdj3k.emu/instance-N/settings.txt`

| Key | Type | Default | Notes |
|---|---|---|---|
| `mac` | `xx:xx:xx:xx:xx:xx` | generated LAA | First byte forced to `02` (LAA unicast). Generated on first load if absent or invalid. |
| `soc_serial` | 16 hex digits | generated | The guest's `/proc/cpuinfo` `Serial` line. Minted on first load, replaced by an applied install, fixed otherwise - see below. |
| `audio_enabled` | `0` / `1` | `0` | Gates `virtio-snd` in argv |
| `audio_device_uid` | CoreAudio UID or empty | empty | Output device for `-audiodev coreaudio,out.device-uid=...` |
| `alc_enabled` | `0` / `1` | `1` | Pushes `audio_sync_enabled` to guest sysfs at boot |
| `haptic_enabled` | `0` / `1` | `1` | Gates Force Touch detent clicks |
| `model` | `cdj3k` / `cdj3kx` or empty | empty | The model the slot's installation is; the app opens the slot on it |
| `firmware_release` | e.g. `3.20` or empty | empty | The release the installation came from, as `IMAGES/RELEASE.TXT` gives it |
| `pc_link_enabled` | `0` / `1` | `0` | PC Link (USB-B) cable plugged in |
| `net_iface` | `en0` etc. or empty | empty | Selected Pro DJ Link interface |
| `usb_virtual_path` | path or empty | empty | Last attached virtual USB image |
| `usb_physical_bsd` | `disk2` etc. or empty | empty | Last attached physical USB disk |
| `screen_extended` | `0` / `1` | `0` | Draw the LCD over the decoration around it |
| `jog_adjust` | `f32` ∈ [0, 1] | `0.5` | JOG ADJUST rotary; picks the brake stop time |
| `vinyl_speed` | `u8` | `0` | VINYL SPEED ADJUST rotary; rides every MISO frame |

### SoC serial

`soc_serial` is 16 lowercase hex digits with a non-zero last byte, the form an
RK3399 prints for `Serial`. aarch64 prints no such line and QEMU has no SoC
serial to expose, so the emulator supplies one: it rides the kernel cmdline as
`cdj3k.serial=<hex>`, and guest patch 12
(`initramfs-patch/patch-rootfs.d/12-cpuinfo-serial.sh`) appends the `Serial`
line to a copy of `/proc/cpuinfo` under `/run` and bind-mounts it over the
original, so every process reads one value. Both models get it.

The deck's `genkey_pr` derives the `cabinet.img` LUKS passphrase from the model
and that serial, repeating the pair `strtol(last two digits, 16) + 1` times -
hence the non-zero last byte. Nothing re-keys the container at boot, so it opens
only for the serial the install keyed it for: a serial changed after an
install locks `cabinet.img` out. It is minted once per install
- on first load when absent or malformed, and by
`InstanceSettings::mint_soc_serial` when the wizard creates a new eMMC, which
records it with the install - or pinned to a real deck's with
`CDJ3K_SOC_SERIAL`.

`screen_extended`, `jog_adjust` and `vinyl_speed` are a deck's own panel
state, so they belong to the slot the deck runs in: two slots side by side
keep separate knob positions. They are
written by `PanelSettings`, which owns only those keys and leaves the rest of
the file to `InstanceSettings`.

A slot carrying none of these three takes the defaults and writes them on its
next save.

The app-wide `~/Library/Application Support/com.cdj3k.emu/settings.txt` holds
no keys: `APP_KEYS` is empty and nothing reads it. `prune_app_file()` empties
it at startup of any key it still carries.

Firmware metadata captured from the .UPD ISO is stored in neither file - it
goes into the U-Boot env block inside the eMMC image at provisioning time.

---

## Firmware install path (short)

The in-app "Install Firmware" wizard
(`crates/cdj3k-emu-ui/src/app/firmware_wizard.rs`) provisions one model in
one slot (the model comes from the picker card or the running emulation) and
takes:

- a `.UPD` file (Pioneer's LUKS1-encrypted firmware update) plus a LUKS
  keyfile (user-supplied - never shipped), **or**
- the already-decrypted ISO 9660 payload of that `.UPD` (detected by its
  `CD001` descriptor), which skips step 1 below.

Pipeline:

1. `cdj3k-emu-firmware::luks::decrypt_upd` parses the LUKS1 header,
   runs PBKDF2 against each active key slot, AF-merges the recovered
   key material, verifies the master-key digest, and streams the
   plaintext ISO to a scratch file.
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
5. `provision_emmc(EmmcConfig)` creates the eMMC in `instance-N/.staging/`,
   which `apply_staged` moves to `instance-N/emmc.qcow2`, with the
   partition table + U-Boot env containing the captured `FirmwareInfo`.
6. `read_cabinet_image` pulls `images/cabinet.img` out of the ISO (inner
   `CDJ3K-RK3399.ISO` first, then the outer one) and `provision_emmc`
   stages it in the recovery partition for guest patch 14.
7. `InstanceSettings::mint_soc_serial` mints the slot's SoC serial, or
   `parse_soc_serial` takes it from `CDJ3K_SOC_SERIAL`. The staged cabinet
   opens only for the serial it was keyed for, so the two travel together:
   the serial goes into the staging record with the model and release, and
   into the slot's settings only when `apply_staged` swaps the new files in.

Pioneer-owned material never leaves the user's app-data directory.
Nothing is committed to the repo or bundled in the `.dmg`.

---

## Files

| Path | Role |
|---|---|
| `crates/cdj3k-emu-storage/src/lib.rs` | `app_data_dir`, public re-exports |
| `crates/cdj3k-emu-storage/src/staging.rs` | `StagedFirmware`, `pending_install`, `apply_staged`, `request_restart` |
| `crates/cdj3k-emu-storage/src/emmc.rs` | Provisioning, partition layout, U-Boot env |
| `crates/cdj3k-emu-storage/src/gpt.rs` | Pure-Rust GPT writer (protective MBR + primary + backup) |
| `crates/cdj3k-emu-storage/src/settings/kv.rs` | the `key=value` file layer, `SLOT_KEYS` / `APP_KEYS`, `prune_app_file` |
| `crates/cdj3k-emu-storage/src/settings/instance.rs` | `InstanceSettings` |
| `crates/cdj3k-emu-storage/src/settings/panel.rs` | `PanelSettings` |
| `crates/cdj3k-emu-storage/src/settings/identity.rs` | MAC and SoC-serial minting and validation |
| `crates/cdj3k-emu-runtime/src/usb.rs` | `UsbManager` hot-swap, virtual + physical attach |
| `crates/cdj3k-emu-runtime/src/macos_disk.rs` | `list_removable`, `unmount_disk`, `mount_disk` |
| `crates/cdj3k-emu-runtime/src/config.rs` | `-drive` argv lines (USB then eMMC) |
| `crates/cdj3k-emu-firmware/src/luks.rs` | LUKS1 decrypt of the .UPD payload |
| `crates/cdj3k-emu-firmware/src/extract.rs` | Kernel + `FirmwareInfo` extraction, G2M reject |
| `crates/cdj3k-emu-ui/src/app/firmware_wizard.rs` | "Install Firmware" wizard |
