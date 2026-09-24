#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 28: usb_gadget.sh - PC-link USB gadget setup for QEMU.
#
# Rewrites Pioneer's /home/root/scripts/usb_gadget.sh so it works against
# dummy_hcd's dummy_udc UDC instead of the (absent) RK3399 DWC3:
#   - drops uac2.usb0: CONFIG_USB_CONFIGFS_F_UAC2 is off, and the sysfs knobs
#     Pioneer's uac-monitor.sh drives (usb_f_uac2 'enable'/'uac2_srate') are
#     patches on their 4.4 BSP that mainline 6.6 does not have.  Host audio
#     goes over virtio-snd.  Patch 22 masks usb-f-uac.service to match.
#     docs/pc-link.md has the full picture.
#   - keeps midi.usb0 + hid.usb0 with original Pioneer parameters
#   - replaces fw_printenv (no u-boot env on QEMU) with static fallbacks
#   - binds explicitly to dummy_udc; bails cleanly if it isn't there yet
#   - idempotent: tears down any prior g1 before rebuilding
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

cat > "$ROOTFS/home/root/scripts/usb_gadget.sh" << 'SCRIPTEOF'
#!/bin/sh
# CDJ-3000 PC-link gadget: HID (vendor 0xFFA0) + MIDI.
# UAC2 removed for QEMU - host audio is routed via virtio-snd, not USB.

set -eu

CONFIGFS=/sys/kernel/config
GADGET=$CONFIGFS/usb_gadget/g1

# configfs may not be mounted yet on first boot.
if ! mountpoint -q "$CONFIGFS"; then
    /bin/mount -t configfs none "$CONFIGFS"
fi

# dummy_hcd is built in (=y), so a UDC should be present at boot.  Pick the
# first entry under /sys/class/udc/ rather than hard-coding a name; the
# dummy_hcd UDC is named "dummy_udc.0" (instance-suffixed), and other
# platforms would call theirs something else entirely.  Wait up to 3 s in
# case the udc class registers slightly after rootfs init.
i=0
while [ $i -lt 30 ] && [ -z "$(ls -1 /sys/class/udc/ 2>/dev/null | head -1)" ]; do
    sleep 0.1
    i=$((i + 1))
done
UDC_NAME=$(ls -1 /sys/class/udc/ 2>/dev/null | head -1)
if [ -z "$UDC_NAME" ]; then
    echo "usb_gadget: no UDC in /sys/class/udc - aborting" >&2
    exit 1
fi

# Tear down any prior gadget so re-runs (or stale state from a previous boot
# carried in a snapshot) start clean.
if [ -d "$GADGET" ]; then
    # configfs attributes report a non-zero size whatever they hold, so the
    # bound check reads the value.  Writing "" to an unbound gadget returns
    # -ENODEV, which under `set -e` would abort before the rebuild below.
    if [ -n "$(cat "$GADGET/UDC" 2>/dev/null)" ]; then
        echo "" > "$GADGET/UDC"
    fi
    rm -f "$GADGET/configs/c.1/"hid.usb0 "$GADGET/configs/c.1/"midi.usb0 2>/dev/null || true
    rmdir "$GADGET/configs/c.1/strings/0x409" 2>/dev/null || true
    rmdir "$GADGET/configs/c.1" 2>/dev/null || true
    rmdir "$GADGET/functions/"* 2>/dev/null || true
    rmdir "$GADGET/strings/0x409" 2>/dev/null || true
    rmdir "$GADGET" 2>/dev/null || true
fi

mkdir -p "$GADGET"
cd "$GADGET"

# VID/PID/bcd: VID/PID match real CDJ-3000 so rekordbox identifies the device.
# bcdDevice on real HW comes from fw_printenv -n release; that env doesn't
# exist on QEMU, so we hard-code 0x0100 (firmware 1.00).
echo 0x2b73 > idVendor
echo 0x002f > idProduct
echo 0x0100 > bcdDevice
echo 0x0200 > bcdUSB
echo 0x00   > bDeviceClass
echo 0x00   > bDeviceSubClass
echo 0x00   > bDeviceProtocol

mkdir -p strings/0x409
# Serial: real hardware reads u-boot env serial_number; QEMU has none, use a
# stable QEMU-recognisable string so the host can pin its virtual device.
echo "CDJ3KQEMU000000" > strings/0x409/serialnumber
echo "Pioneer DJ"      > strings/0x409/manufacturer
echo "CDJ-3000"        > strings/0x409/product

mkdir -p functions/midi.usb0

mkdir -p functions/hid.usb0
echo 0  > functions/hid.usb0/protocol
echo 0  > functions/hid.usb0/subclass
echo 64 > functions/hid.usb0/report_length
echo -ne \\x06\\xa0\\xff\\x09\\x01\\xa1\\x01\\x09\\x02\\xa1\\x00\\x06\\xa1\\xff\\x09\\x03\\x09\\x04\\x15\\x80\\x25\\x7f\\x35\\x00\\x45\\xff\\x75\\x08\\x95\\x40\\x81\\x02\\x09\\x05\\x09\\x06\\x15\\x80\\x25\\x7f\\x35\\x00\\x45\\xff\\x75\\x08\\x95\\x40\\x91\\x02\\xc0\\xc0 > functions/hid.usb0/report_desc

mkdir -p configs/c.1
echo 2 > configs/c.1/MaxPower

ln -sf functions/midi.usb0 configs/c.1/
ln -sf functions/hid.usb0  configs/c.1/

echo "$UDC_NAME" > UDC
echo "usb_gadget: bound g1 to $UDC_NAME (midi + hid, no uac)"
SCRIPTEOF

chmod 755 "$ROOTFS/home/root/scripts/usb_gadget.sh"
echo "  -> /home/root/scripts/usb_gadget.sh rewritten (midi + hid, dummy_udc)"
