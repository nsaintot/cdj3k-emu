#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 23: usb-external - on-demand attach script for QEMU USB mass storage hotplug.
#
# QEMU presents the USB drive via qemu-xhci + usb-storage.  The Pioneer kernel's
# USB storage driver enumerates it as /dev/sdb: /dev/sda is claimed by the SD card
# slot driver, so the USB storage device lands on the next available SCSI slot.
#
# Invoked on demand by cdj3k-cfgd (patch 21) when cdj3k-emu-runtime sends
# `usb attach` over the cdj3k.cfg virtio-serial port after QMP device_add.
#
# Protocol:
#   Mount:  printf 'mount %s %s protect:0' "$MNT" "$FS_TYPE" > /proc/udev_usb1
#   Eject:  printf 'umount %s' "$MNT" > /proc/udev_usb1
#   NEVER write "connect," - causes "USB Error. Remove the device."
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

# ---- /usr/sbin/usb-external-attach.sh ----
cat > "$ROOTFS/usr/sbin/usb-external-attach.sh" << 'SCRIPTEOF'
#!/bin/sh
# Attach hotplugged USB mass storage (appears as /dev/sdb via usb-storage + sd_mod;
# /dev/sda is the SD card slot) and notify EP122 via /proc/udev_usb1.
# Triggered by cdj3k-cfgd after cdj3k-emu-runtime sends `usb attach`.

DEV=/dev/sdb

# /dev/sdb is always registered (virtio_blk keeps it present with capacity 0
# when no USB is plugged in).  Trigger a partition table rescan so that sdb1
# appears now that real media is present, then wait up to 3s for it.
/sbin/blockdev --rereadpt "$DEV" 2>/dev/null || true
i=0
while [ $i -lt 30 ] && [ ! -b "${DEV}1" ]; do sleep 0.1; i=$((i+1)); done

# Use sdb1 if the partition appeared; fall back to raw device if not.
if [ -b "${DEV}1" ]; then
    PART="${DEV}1"; DEVBASE=sdb1
else
    PART="$DEV";    DEVBASE=sdb
fi

eval $(/sbin/blkid -o udev -p "$PART" 2>/dev/null || true)
FS_TYPE="${ID_FS_TYPE:-vfat}"

MNT="/media/usb/${DEVBASE}"
mkdir -p "$MNT"

case "$FS_TYPE" in
  vfat)            mount -o "rw,noatime,shortname=mixed,dmask=000,fmask=000,flush" "$PART" "$MNT" ;;
  exfat)           mount -t exfat -o rw,noatime "$PART" "$MNT" 2>/dev/null || \
                   mount -t exfat-fuse -o rw,noatime "$PART" "$MNT" ;;
  hfsplus)         mount -t hfsplus -o rw,noatime,force "$PART" "$MNT" ;;
  ext4|ext3|ext2) mount -t "$FS_TYPE" -o rw,noatime "$PART" "$MNT" ;;
  *) echo "usb-external-attach: unsupported fs '$FS_TYPE'"; rmdir "$MNT" 2>/dev/null; exit 1 ;;
esac

if ! mount | grep -q " $MNT "; then
    echo "usb-external-attach: mount failed for $PART ($FS_TYPE)"
    rmdir "$MNT" 2>/dev/null
    exit 1
fi

printf 'mount %s %s protect:0' "$MNT" "$FS_TYPE" > /proc/udev_usb1

# Signal cfgd to emit `usb_state 1` on the cdj3k.cfg port - we can't write
# the port directly because cfgd holds it O_RDWR (virtio-serial is single-
# opener on Linux, second open returns EBUSY).  The host doesn't strictly
# need this for the attach side (it initiated the attach via QMP) but the
# notification keeps the host/guest view symmetric.
CFGD_PID=$(/bin/pidof cdj3k-cfgd 2>/dev/null)
if [ -n "$CFGD_PID" ]; then
    /bin/kill -USR2 $CFGD_PID 2>/dev/null || true
fi

echo "usb-external-attach: $PART → $MNT ($FS_TYPE)"
SCRIPTEOF

chmod 755 "$ROOTFS/usr/sbin/usb-external-attach.sh"
echo "  -> /usr/sbin/usb-external-attach.sh installed"

# /sbin/mount.exfat is a symlink to mount.exfat-fuse; it intercepts 'mount -t exfat'
# before the kernel driver gets a chance.  Remove it so the kernel's built-in
# exfat (CONFIG_EXFAT_FS=y) handles the mount directly.
rm -f "$ROOTFS/sbin/mount.exfat"
echo "  -> removed /sbin/mount.exfat symlink (kernel exfat takes over)"

# Detach has no dedicated script: cfgd's `usb detach` handler signals EP122
# via /proc/udev_usb1 and lazy-unmounts /media/usb/sd* in-process.
