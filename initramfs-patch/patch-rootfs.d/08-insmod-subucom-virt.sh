#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 08: insmod-subucom-virt.service - load subucom_virt.ko before pre-setting
#
# pre-setting.sh runs subucom_read which opens /dev/subucom_spi1.0 to read the
# testmode byte and write /tmp/testmode.  Without /tmp/testmode, apl_start.sh
# prints "can't read /tmp/testmode" and exits without launching EP122.
#
# subucom_virt.ko registers the virtual /dev/subucom_spi1.0 device.
# This service must complete before pre-setting.service runs.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

SERVICE_DIR="$ROOTFS/etc/systemd/system"
mkdir -p "$SERVICE_DIR"

cat > "$SERVICE_DIR/insmod-subucom-virt.service" << 'SVCEOF'
[Unit]
Description=Load subucom_virt.ko - virtual /dev/subucom_spi1.0 for QEMU
DefaultDependencies=no
Before=pre-setting.service
Before=EP122.service
Before=multi-user.target

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/bin/sh -c 'TM=0; grep -qw subucom_testmode /proc/cmdline && TM=1; /sbin/insmod /lib/modules/subucom_virt.ko inject_testmode=$$TM'

[Install]
WantedBy=multi-user.target
SVCEOF

chmod 644 "$SERVICE_DIR/insmod-subucom-virt.service"

MULTI_USER_WANTS="$ROOTFS/etc/systemd/system/multi-user.target.wants"
mkdir -p "$MULTI_USER_WANTS"
ln -sf /etc/systemd/system/insmod-subucom-virt.service \
    "$MULTI_USER_WANTS/insmod-subucom-virt.service"

echo "  -> insmod-subucom-virt.service installed and enabled"
