#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 13: <app unit>.d/10-qemu.conf loads deck_shim.so through LD_PRELOAD.
#
# Without deck_shim.so preloaded into the app, the USB bind intercept is
# inactive. When the app processes the "mount /media/usb/sdb1 …" event from
# /proc/udev_usb1, it writes the USB interface name to:
#   /sys/bus/usb/drivers/usb-storage/bind
#   /sys/bus/usb/drivers/usb-storage/unbind
#   /sys/bus/usb/drivers/usb/unbind
# QEMU -machine virt has no xHCI or EHCI controller, so these sysfs writes
# return ENODEV and the app shows "USB Error. Remove the device."
#
# The drop-in reads LD_PRELOAD from two environment files; the second one,
# when present, wins:
#   /etc/cdj3k/preload.env           LD_PRELOAD=deck_shim.so (part of the image)
#   /run/cdj3k-mods/preload.env      deck_shim.so first, then the libraries of
#                                    this boot's mods (patch 30); absent when
#                                    the boot has no mods
# systemd reads both at every start of the unit, so a mod's libraries need no
# daemon-reload. The app and all its children inherit LD_PRELOAD, and
# deck_shim.so redirects the bind writes to /dev/null. The shim hooks only libc
# entry points, so it serves every player binary.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"
: "${APP_UNIT:?APP_UNIT must be set by dispatcher}"

mkdir -p "$ROOTFS/etc/cdj3k"
cat > "$ROOTFS/etc/cdj3k/preload.env" << 'ENVEOF'
LD_PRELOAD=/home/root/deck_shim.so
ENVEOF
chmod 644 "$ROOTFS/etc/cdj3k/preload.env"

DROP_IN_DIR="$ROOTFS/etc/systemd/system/${APP_UNIT}.d"
mkdir -p "$DROP_IN_DIR"

cat > "$DROP_IN_DIR/10-qemu.conf" << 'SVCEOF'
[Unit]
Wants=cdj3k-mods.service
After=cdj3k-mods.service

[Service]
# The shim's diagnostics are off by default. Read them with `journalctl -u <unit>`,
# the unit this drop-in belongs to (EP122 on the CDJ-3000, EP145 on the
# CDJ-3000X, EP166 on the CDJ-1500X).
# Environment=DECK_TIME_SHIFT_DEBUG=1
# Environment=DECK_LINK_DEBUG=1

EnvironmentFile=/etc/cdj3k/preload.env
EnvironmentFile=-/run/cdj3k-mods/preload.env
SVCEOF

chmod 644 "$DROP_IN_DIR/10-qemu.conf"
echo "  -> ${APP_UNIT}.d/10-qemu.conf installed (LD_PRELOAD from /etc/cdj3k and /run/cdj3k-mods)"
