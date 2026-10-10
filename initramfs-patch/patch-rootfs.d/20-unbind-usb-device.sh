#!/usr/bin/env bash
# Patch 20: replace unbind-usb-device.sh with a version that works on QEMU
# virtio-mmio.
#
# Changes from the original script:
#   1. The DEVICE sed uses a looser regex: it does not require /host and
#      matches any parent path.
#   2. $DEVICE is quoted in the -z test, which prevents "too many arguments"
#      under set -u.
#   3. The udev_usb1 notification greps the mount path instead of hardcoding
#      USB1_BUS_NAME="5-1".
#   4. The usb/unbind write is guarded by an N-N pattern, so virtio and other
#      non-USB buses are skipped.
#   5. virtio is detected through sysfs: the immediate parent of /dev/sdb1 is
#      the block device "sdb", not "virtio0", so a DEV_BUS_ID == virtio*
#      match would never fire. The script checks the /sys/class/block/<dev>
#      link for "/virtio[0-9]+/" and signals cfgd, which sends `usb_state 0`
#      on cdj3k.cfg so the host's CfgClient knows the guest ejected the
#      drive.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"
: "${APP_NAME:?APP_NAME must be set by dispatcher}"

cat > "$ROOTFS/home/root/scripts/unbind-usb-device.sh" << 'SCRIPTEOF'
#!/bin/sh
#
#  suspend-usb-device: an easy-to-use script to properly put an USB
#  device into suspend mode that can then be unplugged safely
#
#  Copyright (C) 2009-2016, Yan Li <elliot.li.tech@gmail.com>
#  GPLv3 - see source header for full licence text.
#
#  ACKNOWLEDGEMENTS:
#      Christian Schmitt <chris@ilovelinux.de> for firewire supporting
#      David <d.tonhofer@m-plify.com> for improving parent device search

usage()
{
    cat<<EOF
suspend-usb-device  Copyright (C) 2009-2016, Yan Li <elliot.li.tech@gmail.com>

usage:
$0 [options] dev

options:
  -l     show the device and USB bus ID only
  -h     print this usage
  -v     verbose
EOF
}

set -e -u

SHOW_DEVICE_ONLY=0
VERBOSE=0
while getopts "vlh" opt; do
    case "$opt" in
        h) usage; exit 2 ;;
        l) SHOW_DEVICE_ONLY=1 ;;
        v) VERBOSE=1 ;;
        ?) echo; usage; exit 2 ;;
    esac
done
MOUNT_PATH=${!OPTIND:-}
DEV_NAME=$(mount | grep "${MOUNT_PATH}" | awk '{print $1}')
if [[ ${DEV_NAME} =~ sd[a-h][1-4] ]]; then
    DEV_NAME=$(echo -n ${DEV_NAME} | sed -e "s/.$//g")
fi

if [ -z ${DEV_NAME} ]; then
    exit 2
fi

# Fix 1: match the path up to the closing quote, without requiring /host.
DEVICE=$(udevadm info --query=path --name=${DEV_NAME} --attribute-walk | \
    egrep "looking at parent device" | head -n 1 | \
    sed -e "s/.*looking at parent device '\([^']*\)'.*/\1/g")

# Fix 2: quote $DEVICE.
if [ -z "$DEVICE" ]; then
    1>&2 echo "cannot find appropriate parent USB/Firewire device, "
    1>&2 echo "perhaps ${DEV_NAME} is not an USB/Firewire device?"
    exit 1
fi

DEV_BUS_ID=${DEVICE##*/}

[[ $VERBOSE == 1 ]] && echo "Found device $DEVICE associated to $DEV_NAME; USB bus id is $DEV_BUS_ID"

if [ ${SHOW_DEVICE_ONLY} -eq 1 ]; then
    echo Device: ${DEVICE}
    echo Bus ID: ${DEV_BUS_ID}
    exit 0
fi

sync

if [ `id -u` -ne 0 ]; then
    1>&2 echo error, must be run as root, exiting...
    exit 1
fi

if [ ${SHOW_DEVICE_ONLY} -eq 0 ]; then
    MOUNT_LIST=$(mount | grep "^${DEV_NAME}[[:digit:]]* " | sed -e "s/.* on \(\/media\/usb\/.*\) type.*/\1/g")

    for MOUNT_NAME in ${MOUNT_LIST}
    do
        for RETRY in `/bin/seq 1 20`
        do
            FD=$(/bin/lsof | grep "${MOUNT_NAME}" | grep "PIONEER" | wc -l)
            /bin/echo "${RETRY}:${MOUNT_NAME}:fd [$FD]"
            if [ $FD -eq 0 ]; then
                break
            fi
            /bin/sleep 1
        done
    done

    for MOUNT_NAME in ${MOUNT_LIST}
    do
        /bin/umount -l ${MOUNT_NAME}
        # Fix 3: check the mount path instead of hardcoding USB1_BUS_NAME="5-1".
        if echo "${MOUNT_NAME}" | grep -q "^/media/usb/"; then
            /bin/echo -n umount ${MOUNT_NAME} > /proc/udev_usb1
        fi
    done
fi

[[ $VERBOSE == 1 ]] && echo "Unbinding device $DEV_BUS_ID"

# Fix 5: detect virtio-backed block devices through sysfs. DEV_BUS_ID is the
# basename of the immediate parent, such as "sdb", not "virtio0", so a match
# on `[[ $DEV_BUS_ID == virtio* ]]` never fires for virtio_blk. The
# /sys/class/block/<device or partition> link holds the full chain of
# ancestors; look for "/virtio<N>/" anywhere in it.
DEV_BASE=$(echo "${DEV_NAME}" | /bin/sed -e 's|^/dev/||')
IS_VIRTIO=0
if /bin/readlink "/sys/class/block/${DEV_BASE}" 2>/dev/null | grep -q "/virtio[0-9]"; then
    IS_VIRTIO=1
fi

if [[ "${DEV_BUS_ID}" == fw* ]]
then
    echo -n "${DEV_BUS_ID}" > /sys/bus/firewire/drivers/sbp2/unbind
elif [ $IS_VIRTIO -eq 1 ]
then
    # virtio-blk has no USB bus to unbind from. Ask cfgd to send
    # `usb_state 0` on the cdj3k.cfg port. The script cannot write the port
    # itself: cfgd holds it open O_RDWR, and Linux allows one opener per
    # virtio-serial port, so a second open returns EBUSY.
    CFGD_PID=$(/bin/pidof cdj3k-cfgd 2>/dev/null)
    if [ -n "$CFGD_PID" ]; then
        /bin/kill -USR1 $CFGD_PID 2>/dev/null || true
    fi
else
    # Fix 4: write usb/unbind only for USB bus IDs.
    if echo "${DEV_BUS_ID}" | grep -qE "^[0-9]+-[0-9]+$"; then
        echo -n "${DEV_BUS_ID}" > /sys/bus/usb/drivers/usb/unbind
    fi

    [[ $VERBOSE == 1 ]] && echo "Checking whether $DEVICE can be suspended"
    POWER_LEVEL_FILE=/sys${DEVICE}/power/level
    POWER_CONTROL_FILE=/sys${DEVICE}/power/control
    if [ ! -f "$POWER_CONTROL_FILE" -a ! -f "$POWER_LEVEL_FILE" ]; then
        1>&2 cat<<EOF
It's safe to remove the USB device now but better can be done. The
power level control file $POWER_LEVEL_FILE
doesn't exist on the system so I have no way to put the USB device
into suspend mode, perhaps you don't have CONFIG_USB_SUSPEND enabled
in your running kernel.
EOF
        exit 3
    elif [ ! -f "$POWER_CONTROL_FILE" ]; then
        [[ $VERBOSE == 1 ]] && echo "Suspending $DEVICE by writing to $POWER_LEVEL_FILE"
        echo 'suspend' > "$POWER_LEVEL_FILE"
    fi
fi
SCRIPTEOF

# The open-file wait greps lsof for the player: on the CDJ-3000 the lines
# carry "PIONEER", and on the CDJ-3000X they carry the binary path.
sed -i "s|grep \"PIONEER\"|grep -E \"PIONEER\\|/pdj/${APP_NAME}\"|" \
    "$ROOTFS/home/root/scripts/unbind-usb-device.sh"
chmod 755 "$ROOTFS/home/root/scripts/unbind-usb-device.sh"
echo "  -> unbind-usb-device.sh patched: virtio support added, and the lsof check matches ${APP_NAME}"
