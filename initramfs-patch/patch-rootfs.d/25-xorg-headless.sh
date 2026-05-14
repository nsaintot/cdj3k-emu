#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 25: xorg headless config - dummy_drv.so with runtime card0 detection
#
# Problem: x11-only.sh launches "X :0" with no -configdir, so Xorg loads
# /etc/X11/xorg.conf.d/20-modesetting.conf (Driver "modesetting").  On Pioneer
# kernel this opens /dev/dri/card0 from virtio_gpu.ko; when there is no DRM
# device Xorg exits "no screens found".
#
# Fix:
#   1. Write xorg-headless.conf (Driver "dummy") to /etc/x11-headless-conf/.
#      dummy_drv.so creates a pure-software virtual screen with no kernel device.
#      This is the only file in that directory so modesetting is never loaded.
#   2. Rewrite x11-only.sh to check /dev/dri/card0 at boot:
#        - card0 present  → exec X :0  (modesetting via 20-modesetting.conf)
#        - no card0       → exec X :0 -configdir /etc/x11-headless-conf
#      This makes both --gl (card0 from virtio_gpu.ko) and headless modes work
#      without rebuilding the initramfs.
#
# Note: the dummy_drv.so binary (Xorg ABI 24.0, Ubuntu 20.04 arm64) is deployed
# to /usr/lib/xorg/modules/drivers/ by build-initramfs.sh.
#
# Constraints (confirmed from Pioneer binary tracing):
#   - xorg-headless.conf must contain ONLY Monitor + Device + Screen sections.
#   - ServerLayout, InputDevice (Driver "void"), and ServerFlags all crash
#     Pioneer Xorg 1.20.4 - those drivers are absent from the Pioneer rootfs.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

CONFDIR="$ROOTFS/etc/x11-headless-conf"
mkdir -p "$CONFDIR"

cat > "$CONFDIR/xorg-headless.conf" << 'EOF'
Section "Monitor"
    Identifier "Headless Monitor"
EndSection

Section "Device"
    Identifier "Headless Device"
    Driver     "dummy"
EndSection

Section "Screen"
    Identifier "Headless Screen"
    Device     "Headless Device"
    Monitor    "Headless Monitor"
EndSection
EOF
chmod 644 "$CONFDIR/xorg-headless.conf"
echo "  -> /etc/x11-headless-conf/xorg-headless.conf written (Driver dummy)"

X11_SH="$ROOTFS/etc/systemd/system/x11-only.sh"
if [[ ! -f "$X11_SH" ]]; then
    echo "  WARNING: $X11_SH not found - skipping x11-only.sh patch"
    exit 0
fi

# Rewrite x11-only.sh with runtime card0 detection.
# Preserves the taskset CPU pin from the original script; falls back to CPU 3.
TASKSET_CPU=3
if grep -q 'taskset' "$X11_SH"; then
    TASKSET_CPU=$(grep -oE 'taskset -c [0-9]+' "$X11_SH" | grep -oE '[0-9]+$' || echo 3)
fi

cat > "$X11_SH" << SHEOF
#!/bin/sh
# card0 present = virtio_gpu loaded (--gl mode); absent = headless
if [ -e /dev/dri/card0 ]; then
    exec taskset -c ${TASKSET_CPU} X :0
else
    exec taskset -c ${TASKSET_CPU} X :0 -configdir /etc/x11-headless-conf
fi
SHEOF
chmod 755 "$X11_SH"
echo "  -> $X11_SH rewritten: card0 detection (modesetting vs dummy)"
