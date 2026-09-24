#!/bin/sh
# SPDX-License-Identifier: MIT OR Apache-2.0
# 27-udev-usb1.sh
#
# Install insmod-udev-usb1.service - loads udev_usb1.ko which creates
# /proc/udev_usb1, the Pioneer USB event channel that the app reads.
# Must run before the app unit so the app finds the proc entry at startup.
set -eu
: "${ROOTFS:?ROOTFS must be set by dispatcher}"
: "${APP_UNIT:?APP_UNIT must be set by dispatcher}"

SERVICE_DIR="$ROOTFS/etc/systemd/system"
mkdir -p "$SERVICE_DIR"

cat > "$SERVICE_DIR/insmod-udev-usb1.service" << EOF
[Unit]
Description=Load udev_usb1.ko (/proc/udev_usb1 for ${APP_NAME} USB events)
DefaultDependencies=no
Before=${APP_UNIT}
Before=multi-user.target

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/sbin/insmod /lib/modules/udev_usb1.ko

[Install]
WantedBy=multi-user.target
EOF

ln -sf /etc/systemd/system/insmod-udev-usb1.service \
    "$SERVICE_DIR/multi-user.target.wants/insmod-udev-usb1.service"

echo "  [27] insmod-udev-usb1.service installed"
