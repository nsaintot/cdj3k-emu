#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 11: the virtio-serial stream service, subucom-forwarder.
#
# The main LCD reaches the host through X and needs no capture process.
# The jog LCD uses zero-copy ivshmem: deck_shim writes pixels directly into
# the ivshmem BAR, and the host polls the seqlock counter from its mmap. The
# jog stream needs no user-space forwarder process and no wake channel.
#
# build.sh, or the provisioning guest, installs /usr/bin/subucom_forwarder
# before this patch runs.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

SERVICE_DIR="$ROOTFS/etc/systemd/system"

MULTI_USER_WANTS="$SERVICE_DIR/multi-user.target.wants"
mkdir -p "$MULTI_USER_WANTS"

# ---- subucom-forwarder.service ----
# The forwarder copies frames in both directions between /dev/subucom_ctrl and
# the cdj3k.ctrl virtio-serial port: LED frames from the guest to the host,
# control frames from the host to the guest. It waits for /dev/subucom_ctrl
# itself, so it does not depend on when insmod-subucom-virt.service loads
# subucom_virt.ko. virtio_console is built into the 6.6 kernel, so
# /dev/virtio-ports/cdj3k.ctrl is always present.
cat > "$SERVICE_DIR/subucom-forwarder.service" << 'SVCEOF'
[Unit]
Description=Bridge between /dev/subucom_ctrl and the cdj3k.ctrl virtio-serial port
After=insmod-virtio-rng.service
After=insmod-virtio-console.service
StartLimitIntervalSec=0

[Service]
Type=simple
ExecStart=/usr/bin/subucom_forwarder
Restart=always
RestartSec=2s

[Install]
WantedBy=multi-user.target
SVCEOF

chmod 644 "$SERVICE_DIR/subucom-forwarder.service"
ln -sf /etc/systemd/system/subucom-forwarder.service \
    "$MULTI_USER_WANTS/subucom-forwarder.service"
echo "  -> subucom-forwarder.service installed and enabled"
