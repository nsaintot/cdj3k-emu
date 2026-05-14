#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 21: cdj3k-cfgd - guest-side config daemon.
#
# Single bidirectional virtio-serial channel `cdj3k.cfg` (replaces the old
# cdj3k.usbcmd + cdj3k.usbstate pair). cdj3k-cfgd:
#
#   - dispatches `usb attach`/`usb detach` from the host (replaces the old
#     cdj3k-usbcmd-listener.sh)
#   - exposes whitelisted /sys/module/virtio_snd/parameters via `set`/`get`
#   - pushes audio_latency_ms (CSV: guest,host,total) to the host every 3s
#
# The USB attach/detach hook scripts (patches 19, 20) write `usb_state 0|1`
# lines directly to /dev/virtio-ports/cdj3k.cfg - Linux guarantees atomic
# writes ≤ PIPE_BUF on virtio-serial, so concurrent writes from cfgd and
# the scripts cannot interleave.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"
: "${PATCH_ASSETS_DIR:?PATCH_ASSETS_DIR must be set by dispatcher}"

# cfgd_aarch64 lives in PATCH_ASSETS_DIR when called from the .app bundle
# (bundle.sh copies it from guest/out/), or in guest/out/ when called
# directly from build-initramfs.sh.
if [[ -f "$PATCH_ASSETS_DIR/cfgd_aarch64" ]]; then
    CFGD_BIN="$PATCH_ASSETS_DIR/cfgd_aarch64"
elif [[ -f "$PATCH_ASSETS_DIR/../guest/out/cfgd_aarch64" ]]; then
    CFGD_BIN="$PATCH_ASSETS_DIR/../guest/out/cfgd_aarch64"
else
    echo "ERROR: cfgd_aarch64 not found in $PATCH_ASSETS_DIR or guest/out/" >&2
    echo "       Run: cd guest && make docker" >&2
    exit 1
fi
install -m 0755 "$CFGD_BIN" "$ROOTFS/usr/sbin/cdj3k-cfgd"
echo "  -> /usr/sbin/cdj3k-cfgd installed (from $(basename "$(dirname "$CFGD_BIN")")/)"

SERVICE_DIR="$ROOTFS/etc/systemd/system"
WANTS_DIR="$SERVICE_DIR/multi-user.target.wants"
mkdir -p "$WANTS_DIR"

cat > "$SERVICE_DIR/cdj3k-cfgd.service" << 'SVCEOF'
[Unit]
Description=CDJ-3000 cfg daemon (cdj3k.cfg virtio-serial bridge)
After=insmod-virtio-snd.service
StartLimitIntervalSec=0

[Service]
Type=simple
ExecStart=/usr/sbin/cdj3k-cfgd
Restart=always
RestartSec=2s

[Install]
WantedBy=multi-user.target
SVCEOF
chmod 644 "$SERVICE_DIR/cdj3k-cfgd.service"
ln -sf /etc/systemd/system/cdj3k-cfgd.service \
    "$WANTS_DIR/cdj3k-cfgd.service"

echo "  -> cdj3k-cfgd.service installed and enabled"
