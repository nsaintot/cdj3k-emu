#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 28: usb_gadget.sh - PC-link USB gadget setup for QEMU.
#
# Keeps the firmware's own /home/root/scripts/usb_gadget.sh for everything
# that identifies the deck (idVendor, idProduct, bcdDevice, strings, the HID
# report length and descriptor), so each model presents the gadget its
# firmware defines.  fw_printenv reads the U-Boot env the eMMC image carries.
# Only the plumbing around it changes:
#   - drops uac1/uac2.usb0: CONFIG_USB_CONFIGFS_F_UAC2 is off, and the sysfs
#     knobs Pioneer's uac-monitor.sh drives (usb_f_uac2 'enable'/'uac2_srate')
#     are patches on their 4.4 BSP that mainline 6.6 does not have.  Host
#     audio goes over virtio-snd.  Patch 22 masks usb-f-uac.service to match.
#     docs/pc-link.md has the full picture.
#   - mounts configfs if needed and tears down a prior g1, so a re-run
#     rebuilds the gadget instead of failing on existing directories
#   - waits for dummy_hcd's UDC and binds to it explicitly
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

SCRIPT="$ROOTFS/home/root/scripts/usb_gadget.sh"
if [[ ! -f "$SCRIPT" ]]; then
    echo "  WARNING: $SCRIPT not found; skipping patch 28, PC Link will not work" >&2
    exit 0
fi
if grep -qF 'Rewritten by patch 28' "$SCRIPT"; then
    echo "  -> $SCRIPT already rewritten"
    exit 0
fi

# The firmware script's own lines, minus the ones this patch replaces: the
# shebang, the cd/mkdir into g1, the UAC functions and the UDC bind.  PC Link
# is optional, so a script without every anchor is left untouched and
# provisioning continues without it.
missing=()
for anchor in 'mkdir -p g1 && cd g1' 'functions/hid.usb0/report_desc' 'ls /sys/class/udc/ > UDC'; do
    grep -qF "$anchor" "$SCRIPT" || missing+=("$anchor")
done
if (( ${#missing[@]} )); then
    for anchor in "${missing[@]}"; do
        echo "  WARNING: $SCRIPT has no '$anchor'" >&2
    done
    echo "  WARNING: skipping patch 28, $SCRIPT left as shipped; PC Link will not work" >&2
    exit 0
fi
BODY=$(grep -vE '^#!|^cd /sys/kernel/config/usb_gadget/?$|^mkdir -p g1 && cd g1$|uac[12]\.usb0|^ls /sys/class/udc/ > UDC$' "$SCRIPT")

cat > "$SCRIPT" << SCRIPTEOF
#!/bin/bash
# PC-link gadget: the firmware's HID + MIDI functions, bound to dummy_udc.
# Rewritten by patch 28; the identity lines below are the firmware's own.

CONFIGFS=/sys/kernel/config
GADGET=\$CONFIGFS/usb_gadget/g1

if ! mountpoint -q "\$CONFIGFS"; then
    /bin/mount -t configfs none "\$CONFIGFS"
fi

# dummy_hcd is built in, so its UDC ("dummy_udc.0") should be present at
# boot; wait up to 3 s in case the udc class registers slightly late.
i=0
while [ \$i -lt 30 ] && [ -z "\$(ls -1 /sys/class/udc/ 2>/dev/null | head -1)" ]; do
    sleep 0.1
    i=\$((i + 1))
done
UDC_NAME=\$(ls -1 /sys/class/udc/ 2>/dev/null | head -1)
if [ -z "\$UDC_NAME" ]; then
    echo "usb_gadget: no UDC in /sys/class/udc - aborting" >&2
    exit 1
fi

# Tear down a prior gadget so a re-run starts clean.  configfs attributes
# report a non-zero size whatever they hold, so the bound check reads the
# value; writing "" to an unbound gadget returns -ENODEV.
if [ -d "\$GADGET" ]; then
    if [ -n "\$(cat "\$GADGET/UDC" 2>/dev/null)" ]; then
        echo "" > "\$GADGET/UDC"
    fi
    rm -f "\$GADGET/configs/c.1/"*.usb0 2>/dev/null || true
    rmdir "\$GADGET/configs/c.1/strings/0x409" 2>/dev/null || true
    rmdir "\$GADGET/configs/c.1" 2>/dev/null || true
    rmdir "\$GADGET/functions/"* 2>/dev/null || true
    rmdir "\$GADGET/strings/0x409" 2>/dev/null || true
    rmdir "\$GADGET" 2>/dev/null || true
fi

mkdir -p "\$GADGET"
cd "\$GADGET"

$BODY

echo "\$UDC_NAME" > UDC
echo "usb_gadget: bound g1 to \$UDC_NAME (\$(cat strings/0x409/product), no uac)"
SCRIPTEOF

chmod 755 "$SCRIPT"
echo "  -> $SCRIPT: firmware identity kept, UAC dropped, bound to dummy_udc"
