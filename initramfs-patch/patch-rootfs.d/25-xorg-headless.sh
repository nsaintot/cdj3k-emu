#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 25: the headless Xorg config, which uses dummy_drv.so when the guest
# has no /dev/dri/card0.
#
# x11-only.sh launches "X :0" with no -configdir, so Xorg loads
# /etc/X11/xorg.conf.d/20-modesetting.conf (Driver "modesetting"), which opens
# /dev/dri/card0. virtio_gpu provides card0 only when QEMU exposes a
# virtio-gpu device; without a DRM device, Xorg exits with "no screens found".
#
# The patch:
#   1. Writes xorg-headless.conf (Driver "dummy") to /etc/x11-headless-conf/.
#      dummy_drv.so creates a software screen with no kernel device. It is the
#      only file in that directory, so modesetting is never loaded.
#   2. Rewrites x11-only.sh to check /dev/dri/card0 at boot:
#        - When card0 exists, it runs exec X :0 (modesetting, through
#          20-modesetting.conf).
#        - Without card0, it runs exec X :0 -configdir /etc/x11-headless-conf.
#      Both modes then work without rebuilding the initramfs.
#   3. Removes the stock CPU pins: the taskset in x11-only.sh (core 3 on the
#      CDJ-3000, core 5 on the CDJ-3000X) and, on the CDJ-3000X, the
#      `taskset -pc 5` that apl_start.sh applies to Xorg and its InputThread.
#      apl_start.sh also sets
#      sched_rt_runtime_us=-1, so under emulation the player's real-time
#      threads on the pinned core starve Xorg.
#
# Patch 22 installs the dummy_drv.so binary (Xorg ABI 24.0, Ubuntu 20.04
# arm64) in /usr/lib/xorg/modules/drivers/.
#
# Constraints:
#   - xorg-headless.conf must contain only the Monitor, Device and Screen
#     sections.
#   - ServerLayout, InputDevice (Driver "void") and ServerFlags each crash
#     Pioneer's Xorg 1.20.4, because those drivers are absent from the
#     Pioneer rootfs.
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

APL_START="$ROOTFS/home/root/scripts/apl_start.sh"
if [[ -f "$APL_START" ]] && grep -q 'taskset -pc 5 \${PID}' "$APL_START"; then
    sed -i 's|/usr/bin/taskset -pc 5 \${PID}|:  # QEMU: Xorg stays unpinned|' "$APL_START"
    echo "  -> $APL_START: Xorg and InputThread left unpinned"
fi

X11_SH="$ROOTFS/etc/systemd/system/x11-only.sh"
if [[ ! -f "$X11_SH" ]]; then
    echo "  WARNING: $X11_SH not found; x11-only.sh is not patched"
    exit 0
fi

# Rewrite x11-only.sh with runtime card0 detection and no CPU pin.
cat > "$X11_SH" << 'SHEOF'
#!/bin/sh
# card0 exists when QEMU exposes virtio-gpu; without it, X runs headless.
if [ -e /dev/dri/card0 ]; then
    exec X :0
else
    exec X :0 -configdir /etc/x11-headless-conf
fi
SHEOF
chmod 755 "$X11_SH"
echo "  -> $X11_SH rewritten: card0 detection (modesetting vs dummy), no CPU pin"
