#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 17: EP122.service.d/10-qemu.conf - load ep122_shim.so via LD_PRELOAD
#
# Root cause: the original apl_start.sh does not contain the QEMU_PRELOAD line
# that patch 01 tried to replace (subucom_stub.so / djlink_shim.so never existed
# in this rootfs).  Without ep122_shim.so being preloaded into EP122, the USB bind
# intercept is inactive.
#
# Symptom: when EP122 processes the "mount /media/usb/sda ..." event from
# /proc/udev_usb1, it writes the USB interface name to:
#   /sys/bus/usb/drivers/usb-storage/bind
#   /sys/bus/usb/drivers/usb-storage/unbind
#   /sys/bus/usb/drivers/usb/unbind
# In QEMU -machine virt there is no xHCI/EHCI controller so these sysfs writes
# return ENODEV → EP122 shows "USB Error. Remove the device."
#
# Fix: install a systemd service drop-in that sets
#   Environment=LD_PRELOAD=/home/root/ep122_shim.so
# for EP122.service (which exec's apl_start.sh → EP122).  EP122 and all children
# inherit LD_PRELOAD; ep122_shim.so intercepts the bind writes → /dev/null.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

DROP_IN_DIR="$ROOTFS/etc/systemd/system/EP122.service.d"
mkdir -p "$DROP_IN_DIR"

cat > "$DROP_IN_DIR/10-qemu.conf" << 'SVCEOF'
[Service]
# QEMU: preload ep122_shim.so into EP122 and all children launched from
# apl_start.sh.  Without it EP122 writes to /sys/bus/usb/drivers/usb-storage/bind
# (no xHCI controller in QEMU virt → ENODEV) and shows "USB Error. Remove the device."
# ep122_shim.so intercepts those writes → /dev/null so they succeed silently.
# Environment=EP122_TIME_SHIFT_DEBUG=1
# Environment=EP122_LINK_DEBUG=1
Environment=LD_PRELOAD=/home/root/ep122_shim.so
SVCEOF

chmod 644 "$DROP_IN_DIR/10-qemu.conf"
echo "  -> EP122.service.d/10-qemu.conf installed (LD_PRELOAD=ep122_shim.)"
