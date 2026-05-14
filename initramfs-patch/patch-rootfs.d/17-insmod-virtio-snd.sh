#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 17: insmod-virtio-snd.service - load virtio_snd.ko before EP122.service
#
# virtio_snd.ko is the minimal virtio-sound PCM playback driver that pairs with
# QEMU's -device virtio-sound-device,audiodev=<backend>.  It registers as ALSA
# card 0 and routes EP122's audio directly to the host audio system.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

SERVICE_DIR="$ROOTFS/etc/systemd/system"
mkdir -p "$SERVICE_DIR"

cat > "$SERVICE_DIR/insmod-virtio-snd.service" << 'SVCEOF'
[Unit]
Description=Load virtio_snd.ko - virtio-sound PCM playback (host audio via QEMU)
DefaultDependencies=no
Before=EP122.service
Before=multi-user.target
After=insmod-virtio-rng.service

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/sbin/insmod /lib/modules/virtio_snd.ko

[Install]
WantedBy=multi-user.target
SVCEOF

chmod 644 "$SERVICE_DIR/insmod-virtio-snd.service"

MULTI_USER_WANTS="$ROOTFS/etc/systemd/system/multi-user.target.wants"
mkdir -p "$MULTI_USER_WANTS"
ln -sf /etc/systemd/system/insmod-virtio-snd.service \
    "$MULTI_USER_WANTS/insmod-virtio-snd.service"

echo "  -> insmod-virtio-snd.service installed and enabled"
