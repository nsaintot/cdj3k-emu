#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 14: usb-drive-mount.service - mount loop-backed USB image and notify EP122
#
# virtio_blk.ko crashes on the Pioneer kernel (struct ABI mismatch in add_disk()).
# QEMU's USB host controllers are PCI-only; -machine virt has no PCI bus.
#
# Solution: build-initramfs.sh embeds build/usb.img into the initramfs as
# /opt/usb.img (compresses to <1MB - mostly-empty FAT32 = zeros).  At boot this
# service uses the Pioneer kernel's built-in loop driver (CONFIG_BLK_DEV_LOOP=y)
# to expose the image as /dev/loop0, then mounts it and notifies EP122 via the
# Pioneer kernel proc interface so it sees a connected USB drive.
#
# Sequence (mirrors real CDJ boot with USB pre-inserted):
#   1. losetup /dev/loop0 /opt/usb.img  → kernel scans partitions → /dev/loop0p1
#   2. Mount at /media/usb/sda[1]       (same path device-mount.sh uses for bus 5-1)
#   3. Write "mount ..." to /proc/udev_usb1  (mirrors device-mount.sh)
#
# NOTE: "connect," is intentionally NOT written. It comes from GPIO/USB hardware
# detection via 90-usb-caution.rules (KERNELS=="5-1", ENV{CONNECT}=="1"). Writing
# it without a real USB controller bind sequence causes EP122 to show
# "USB Error. Remove the device." - the mount event alone is sufficient.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

SERVICE_DIR="$ROOTFS/etc/systemd/system"
mkdir -p "$SERVICE_DIR"

# ---- /usr/sbin/usb-drive-mount.sh ----
cat > "$ROOTFS/usr/sbin/usb-drive-mount.sh" << 'SCRIPTEOF'
#!/bin/sh
USB_IMG="/opt/usb.img"
# Exit cleanly if no image (build without embedded USB)
[ -f "$USB_IMG" ] || exit 0

/sbin/losetup /dev/loop0 "$USB_IMG"
sleep 1   # kernel partition scan (blkdev_reread_part)

if [ -b /dev/loop0p1 ]; then PART=/dev/loop0p1; DEVBASE=sdb1
else                          PART=/dev/loop0;   DEVBASE=sdb;  fi

eval $(/sbin/blkid -o udev -p "$PART" 2>/dev/null || true)
FS_TYPE="${ID_FS_TYPE:-vfat}"
[ "${ID_FS_LABEL:-}" = "EFI" ] && exit 0
[ "${ID_PART_ENTRY_NAME:-}" = "Microsoftx20reservedx20partition" ] && exit 0

MNT="/media/usb/${DEVBASE}"
mkdir -p "$MNT"

case "$FS_TYPE" in
  vfat)  mount -o "rw,noatime,shortname=mixed,dmask=000,fmask=000,codepage=437,iocharset=iso8859-1,usefree,utf8,flush" "$PART" "$MNT" ;;
  exfat) mount -t exfat -o rw,noatime,flush "$PART" "$MNT" ;;
  hfsplus) mount -t hfsplus -o rw,noatime,force "$PART" "$MNT" ;;
  *)
    echo "usb-drive-mount: unsupported fs '$FS_TYPE'"
    rmdir "$MNT" 2>/dev/null; /sbin/losetup -d /dev/loop0 2>/dev/null
    exit 0 ;;
esac

printf 'mount %s %s protect:0' "$MNT" "$FS_TYPE" > /proc/udev_usb1
echo "usb-drive-mount: $PART → $MNT ($FS_TYPE)"
SCRIPTEOF

chmod 755 "$ROOTFS/usr/sbin/usb-drive-mount.sh"
echo "  -> /usr/sbin/usb-drive-mount.sh installed"

# ---- usb-drive-mount.service ----
cat > "$SERVICE_DIR/usb-drive-mount.service" << 'SVCEOF'
[Unit]
Description=Mount loop-backed USB image (/opt/usb.img) and notify EP122 via /proc/udev_usb1
DefaultDependencies=no
# Loop device is built-in (CONFIG_BLK_DEV_LOOP=y); no module load needed.
# Use insmod-virtio-rng.service as ordering anchor (it's an early oneshot service).
After=insmod-virtio-rng.service
# Must complete before EP122 reads /proc/udev_usb1
Before=EP122.service
Before=multi-user.target

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/usr/sbin/usb-drive-mount.sh

[Install]
WantedBy=multi-user.target
SVCEOF

chmod 644 "$SERVICE_DIR/usb-drive-mount.service"

MULTI_USER_WANTS="$ROOTFS/etc/systemd/system/multi-user.target.wants"
mkdir -p "$MULTI_USER_WANTS"
ln -sf /etc/systemd/system/usb-drive-mount.service \
    "$MULTI_USER_WANTS/usb-drive-mount.service"
echo "  -> usb-drive-mount.service installed and enabled"
