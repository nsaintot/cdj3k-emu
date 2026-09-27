#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 17: insmod-virtio-snd.service - load virtio_snd.ko before the app unit
#
# virtio_snd.ko is the minimal virtio-sound PCM playback driver that pairs with
# QEMU's -device virtio-sound-device,audiodev=<backend>.  It registers as ALSA
# card 0 and routes EP122's audio directly to the host audio system.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"
: "${APP_UNIT:?APP_UNIT must be set by dispatcher}"

SERVICE_DIR="$ROOTFS/etc/systemd/system"
mkdir -p "$SERVICE_DIR"

# The ALSA period comes from the kernel command line (cdj3k.snd_period=N), so
# the host picks it per boot without reprovisioning.
cat > "$ROOTFS/usr/sbin/cdj3k-insmod-snd" << 'SHEOF'
#!/bin/sh
period=$(sed -n 's/.*cdj3k\.snd_period=\([0-9]*\).*/\1/p' /proc/cmdline)
exec /sbin/insmod /lib/modules/virtio_snd.ko ${period:+period_frames=$period}
SHEOF
chmod 755 "$ROOTFS/usr/sbin/cdj3k-insmod-snd"

cat > "$SERVICE_DIR/insmod-virtio-snd.service" << SVCEOF
[Unit]
Description=Load virtio_snd.ko - virtio-sound PCM playback (host audio via QEMU)
DefaultDependencies=no
Before=${APP_UNIT}
Before=multi-user.target
After=insmod-virtio-rng.service

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/usr/sbin/cdj3k-insmod-snd

[Install]
WantedBy=multi-user.target
SVCEOF

chmod 644 "$SERVICE_DIR/insmod-virtio-snd.service"

MULTI_USER_WANTS="$ROOTFS/etc/systemd/system/multi-user.target.wants"
mkdir -p "$MULTI_USER_WANTS"
ln -sf /etc/systemd/system/insmod-virtio-snd.service \
    "$MULTI_USER_WANTS/insmod-virtio-snd.service"

echo "  -> insmod-virtio-snd.service installed and enabled"
