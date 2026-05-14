#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 13: pre-setting.service - order after insmod-virtio-rng.service
#
# pre-setting.sh runs subucom_read, which opens /dev/subucom_spi1.0 to read
# the testmode byte and write /tmp/testmode.  subucom_virt.ko (loaded by
# insmod-virtio-rng.service) creates that device.
#
# Without this ordering, pre-setting.service races with insmod-virtio-rng.service
# and subucom_read fails with "error: open ErP" - /tmp/testmode is never written,
# apl_start.sh's `if [ -f /tmp/testmode ]` is false, and EP122 never launches.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

DROPIN_DIR="$ROOTFS/etc/systemd/system/pre-setting.service.d"
mkdir -p "$DROPIN_DIR"

cat > "$DROPIN_DIR/10-qemu.conf" << 'EOF'
[Unit]
# QEMU: subucom_read opens /dev/subucom_spi1.0 (created by subucom_virt.ko).
# Wait for both the subucom_virt insmod and the (no-op) virtio-rng service.
After=insmod-subucom-virt.service
After=insmod-virtio-rng.service
EOF

echo "  -> pre-setting.service.d/10-qemu.conf: After=insmod-subucom-virt.service insmod-virtio-rng.service"
