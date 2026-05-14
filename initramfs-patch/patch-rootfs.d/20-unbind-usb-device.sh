#!/usr/bin/env bash
# Patch 24: replace unbind-usb-device.sh with a version that works under QEMU virtio-mmio.
#
# Changes vs. the original script:
#   1. DEVICE sed: relaxed regex - no /host requirement, matches any parent path.
#   2. [ -z $DEVICE ]: quoted (prevents "too many arguments" under set -u).
#   3. udev_usb1 notify: mount-path grep instead of USB1_BUS_NAME="5-1" hardcode.
#   4. usb/unbind: guarded with N-N pattern - skips virtio and other non-USB buses.
#   5. virtio detection via sysfs: the immediate parent of /dev/sdb1 is the
#      block device "sdb", not "virtio0", so the original DEV_BUS_ID==virtio*
#      pattern never matched.  We now check /sys/class/block/<dev> -> ...
#      for "/virtio[0-9]+/" and emit `usb_state 0` on cdj3k.cfg so the host
#      CfgClient knows the guest ejected the drive.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

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

# Fix 1: relaxed sed - match path up to closing quote, no /host requirement.
DEVICE=$(udevadm info --query=path --name=${DEV_NAME} --attribute-walk | \
    egrep "looking at parent device" | head -n 1 | \
    sed -e "s/.*looking at parent device '\([^']*\)'.*/\1/g")

# Fix 2: quoted $DEVICE.
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
        # Fix 3: mount-path check instead of USB1_BUS_NAME="5-1" hardcode.
        if echo "${MOUNT_NAME}" | grep -q "^/media/usb/"; then
            /bin/echo -n umount ${MOUNT_NAME} > /proc/udev_usb1
        fi
    done
fi

[[ $VERBOSE == 1 ]] && echo "Unbinding device $DEV_BUS_ID"

# Fix 5: detect virtio-backed block devices via sysfs.  DEV_BUS_ID is the
# immediate parent basename (e.g. "sdb"), not "virtio0", so the old
# string-match `[[ $DEV_BUS_ID == virtio* ]]` never fired for virtio_blk.
# /sys/class/block/<dev_or_partition> symlinks include the full ancestor
# chain - look for "/virtio<N>/" anywhere in it.
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
    # virtio-blk has no USB bus to unbind from.
    # Signal cfgd to emit `usb_state 0` on the cdj3k.cfg port - we can't
    # write the port directly because cfgd holds it O_RDWR (virtio-serial
    # is single-opener on Linux, second open returns EBUSY).
    CFGD_PID=$(/bin/pidof cdj3k-cfgd 2>/dev/null)
    if [ -n "$CFGD_PID" ]; then
        /bin/kill -USR1 $CFGD_PID 2>/dev/null || true
    fi
else
    # Fix 4: guard usb/unbind - skip for non-USB bus IDs.
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

chmod 755 "$ROOTFS/home/root/scripts/unbind-usb-device.sh"
echo "  -> unbind-usb-device.sh patched (virtio2 support added)"
