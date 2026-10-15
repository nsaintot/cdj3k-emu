#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 19: usb-external, the attach script for a USB drive that the host
# plugs in at run time.
#
# The USB slot is a virtio-blk device (id usb0) that kernel patch
# 01-virtio-blk-naming names /dev/sdb. cdj3k-emu-runtime swaps its medium with
# QMP blockdev-change-medium, then sends `usb attach` over the cdj3k.cfg
# virtio-serial port, and cdj3k-cfgd (patch 21) runs this script.
#
# Protocol:
#   Mount:  printf 'mount %s %s protect:0' "$MNT" "$FS_TYPE" > /proc/udev_usb1
#   Eject:  printf 'umount %s' "$MNT" > /proc/udev_usb1
#   Do not write "connect,": it makes the app show "USB Error. Remove the device."
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

# ---- /usr/sbin/usb-external-attach.sh ----
cat > "$ROOTFS/usr/sbin/usb-external-attach.sh" << 'SCRIPTEOF'
#!/bin/sh
# Mount the USB drive that the host attached (/dev/sdb) and notify the app
# through /proc/udev_usb1. cdj3k-cfgd runs this script when cdj3k-emu-runtime
# sends `usb attach`.

DEV=/dev/sdb

# /dev/sdb always exists: without a drive, the slot holds a one-sector
# placeholder. The attach can arrive before the guest sees the new
# medium's size, and a partition table read at the old size drops sdb1. Wait
# up to 3 s for the size, then rescan the partition table until sdb1 appears,
# for up to 3 s more.
i=0
while [ $i -lt 30 ] && [ "$(cat /sys/block/sdb/size 2>/dev/null || echo 0)" -le 1 ]; do
    sleep 0.1; i=$((i+1))
done
i=0
while [ $i -lt 30 ] && [ ! -b "${DEV}1" ]; do
    /sbin/blockdev --rereadpt "$DEV" 2>/dev/null || true
    sleep 0.1; i=$((i+1))
done

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
  *) echo "usb-external-attach: unsupported file system '$FS_TYPE'"; rmdir "$MNT" 2>/dev/null; exit 1 ;;
esac

if ! mount | grep -q " $MNT "; then
    echo "usb-external-attach: mount failed for $PART ($FS_TYPE)"
    rmdir "$MNT" 2>/dev/null
    exit 1
fi

printf 'mount %s %s protect:0' "$MNT" "$FS_TYPE" > /proc/udev_usb1

# Ask cfgd to send `usb_state 1` on the cdj3k.cfg port. The script cannot
# write the port itself: cfgd holds it open O_RDWR, and Linux allows one
# opener per virtio-serial port, so a second open returns EBUSY. The host
# started the attach and does not need the notice, but it keeps the host and
# guest views in step.
CFGD_PID=$(/bin/pidof cdj3k-cfgd 2>/dev/null)
if [ -n "$CFGD_PID" ]; then
    /bin/kill -USR2 $CFGD_PID 2>/dev/null || true
fi

echo "usb-external-attach: mounted $PART on $MNT ($FS_TYPE)"
SCRIPTEOF

chmod 755 "$ROOTFS/usr/sbin/usb-external-attach.sh"
echo "  -> /usr/sbin/usb-external-attach.sh installed"

# /sbin/mount.exfat is a symlink to mount.exfat-fuse, which takes over
# 'mount -t exfat' before the kernel driver can. Remove it so the kernel's
# built-in exfat (CONFIG_EXFAT_FS=y) handles the mount.
rm -f "$ROOTFS/sbin/mount.exfat"
echo "  -> removed /sbin/mount.exfat symlink (kernel exfat takes over)"

# Detach has no script of its own: cfgd's `usb detach` handler notifies EP122
# through /proc/udev_usb1 and lazily unmounts /media/usb/sd* itself.
