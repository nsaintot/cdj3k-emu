#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 15: virtio-serial stream services - subucom-forwarder.
#
# Main LCD is exposed natively via X → virtio-console (no capture process needed).
# Jog LCD now uses ivshmem zero-copy: ep122_shim writes pixels directly into the
# ivshmem BAR; the host polls the seqlock counter from its mmap. No userspace
# forwarder process and no wake channel needed for the jog stream.
#
# Binaries (/usr/bin/subucom_forwarder) are built in the Docker image and
# extracted by build-initramfs.sh before this patch runs.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

SERVICE_DIR="$ROOTFS/etc/systemd/system"

MULTI_USER_WANTS="$SERVICE_DIR/multi-user.target.wants"
mkdir -p "$MULTI_USER_WANTS"

# ---- subucom-forwarder.service ----
# Bidirectional bridge: /dev/subucom_ctrl ↔ cdj3k.ctrl virtio-serial port.
#   LED  (guest→host): subucom_ctrl → vport → ctrl.sock
#   CTRL (host→guest): ctrl.sock → vport → subucom_ctrl
# Must start after insmod-virtio-rng (subucom_virt.ko); virtio_console is
# built into the 6.6 kernel, so /dev/virtio-ports/cdj3k.ctrl is always present.
cat > "$SERVICE_DIR/subucom-forwarder.service" << 'SVCEOF'
[Unit]
Description=subucom bidirectional bridge (subucom_ctrl <-> cdj3k.ctrl virtio-serial)
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
