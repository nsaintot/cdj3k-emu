#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 09: order pre-setting.service after the sub-CPU device exists.
#
# pre-setting.sh runs subucom_read, which opens the sub-CPU device to read
# the testmode byte and write /tmp/testmode. subucom_virt.ko, which
# insmod-subucom-virt.service (patch 08) loads, creates that device.
#
# Without this ordering, pre-setting.service races with
# insmod-subucom-virt.service and subucom_read fails with "error: open ErP":
# /tmp/testmode is never written, apl_start.sh's `if [ -f /tmp/testmode ]` is
# false, and the app never starts.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

DROPIN_DIR="$ROOTFS/etc/systemd/system/pre-setting.service.d"
mkdir -p "$DROPIN_DIR"

cat > "$DROPIN_DIR/10-qemu.conf" << 'EOF'
[Unit]
# subucom_read opens the sub-CPU device, which subucom_virt.ko creates.
# insmod-virtio-rng.service does nothing on vanilla 6.6 and only keeps the order.
After=insmod-subucom-virt.service
After=insmod-virtio-rng.service
EOF

echo "  -> pre-setting.service.d/10-qemu.conf: After=insmod-subucom-virt.service insmod-virtio-rng.service"
