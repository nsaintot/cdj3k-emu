#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 10: usb-drive-mount.service mounts the loop-backed USB image and
# notifies the app.
#
# build.sh copies build/usb.img into the initramfs as /opt/usb.img when that
# file exists. A mostly empty FAT32 image compresses to under 1 MB. At boot
# this service attaches the image to /dev/loop0 with the built-in loop driver
# (CONFIG_BLK_DEV_LOOP=y), mounts it and notifies the app through
# /proc/udev_usb1, so the app sees a connected USB drive. Without the image the
# service does nothing; a drive that the host attaches at run time goes through
# 19-usb-external.sh instead.
#
# Sequence, the same as a real deck booting with a USB drive inserted:
#   1. losetup /dev/loop0 /opt/usb.img; the kernel scans the partitions and
#      creates /dev/loop0p1.
#   2. Mount at /media/usb/sdb1, or at /media/usb/sdb for an image without a
#      partition table.
#   3. Write "mount ..." to /proc/udev_usb1, as device-mount.sh does.
#
# The service does not write "connect,". That event comes from GPIO/USB
# hardware detection through 90-usb-caution.rules (KERNELS=="5-1",
# ENV{CONNECT}=="1"). Writing it without a real USB controller bind sequence
# makes EP122 show "USB Error. Remove the device."; the mount event alone is
# enough.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"
: "${APP_UNIT:?APP_UNIT must be set by dispatcher}"

SERVICE_DIR="$ROOTFS/etc/systemd/system"
mkdir -p "$SERVICE_DIR"

# ---- /usr/sbin/usb-drive-mount.sh ----
cat > "$ROOTFS/usr/sbin/usb-drive-mount.sh" << 'SCRIPTEOF'
#!/bin/sh
USB_IMG="/opt/usb.img"
# Exit cleanly when the build embedded no USB image.
[ -f "$USB_IMG" ] || exit 0

/sbin/losetup /dev/loop0 "$USB_IMG"
sleep 1   # Wait for the kernel's partition scan (blkdev_reread_part).

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
    echo "usb-drive-mount: unsupported file system '$FS_TYPE'"
    rmdir "$MNT" 2>/dev/null; /sbin/losetup -d /dev/loop0 2>/dev/null
    exit 0 ;;
esac

printf 'mount %s %s protect:0' "$MNT" "$FS_TYPE" > /proc/udev_usb1
echo "usb-drive-mount: mounted $PART on $MNT ($FS_TYPE)"
SCRIPTEOF

chmod 755 "$ROOTFS/usr/sbin/usb-drive-mount.sh"
echo "  -> /usr/sbin/usb-drive-mount.sh installed"

# ---- usb-drive-mount.service ----
cat > "$SERVICE_DIR/usb-drive-mount.service" << SVCEOF
[Unit]
Description=Mount the loop-backed USB image /opt/usb.img and notify ${APP_NAME} through /proc/udev_usb1
DefaultDependencies=no
# The loop driver is built in (CONFIG_BLK_DEV_LOOP=y), so no module needs loading.
# Order after insmod-virtio-rng.service, an early oneshot service.
After=insmod-virtio-rng.service
# The mount must complete before ${APP_NAME} reads /proc/udev_usb1.
Before=${APP_UNIT}
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
