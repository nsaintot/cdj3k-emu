#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 13: <app unit>.d/10-qemu.conf - load deck_shim.so via LD_PRELOAD
#
# Root cause: the original apl_start.sh does not contain the QEMU_PRELOAD line
# that patch 01 tried to replace (subucom_stub.so / djlink_shim.so never existed
# in this rootfs).  Without deck_shim.so being preloaded into EP122, the USB bind
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
#   Environment=LD_PRELOAD=/home/root/deck_shim.so
# for the app unit (EP122.service on the CDJ-3000, EP145.service on the
# CDJ-3000X; it exec's apl_start.sh → the app).  The app and all children
# inherit LD_PRELOAD; deck_shim.so intercepts the bind writes → /dev/null.
# The shim hooks libc entry points only, so it serves both binaries.
#
# The cdj3k-mods are not in this object and the emulator installs none of
# them: they ship as their own LD_PRELOADed library from the cdj3k-mods
# releases.  LD_PRELOAD takes a colon-separated list, so adding them later is
# an edit to the Environment= line below.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"
: "${APP_UNIT:?APP_UNIT must be set by dispatcher}"

DROP_IN_DIR="$ROOTFS/etc/systemd/system/${APP_UNIT}.d"
mkdir -p "$DROP_IN_DIR"

cat > "$DROP_IN_DIR/10-qemu.conf" << 'SVCEOF'
[Service]
# Shim diagnostics, off by default.  Read them with `journalctl -u <unit>`,
# the unit this drop-in belongs to (EP122 on the CDJ-3000, EP145 on the CDJ-3000X).
# Environment=DECK_TIME_SHIFT_DEBUG=1
# Environment=DECK_LINK_DEBUG=1

Environment=LD_PRELOAD=/home/root/deck_shim.so
SVCEOF

chmod 644 "$DROP_IN_DIR/10-qemu.conf"
echo "  -> ${APP_UNIT}.d/10-qemu.conf installed (LD_PRELOAD=deck_shim.so)"
