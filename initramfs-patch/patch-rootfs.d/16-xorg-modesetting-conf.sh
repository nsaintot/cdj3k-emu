#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 16: replace xorg.conf.d/20-modesetting.conf for virtio-gpu (--gl mode only)
#
# Changes from the Pioneer original:
#   Device: AccelMethod exa kept     (modesetting internal EXA - uses DRM/GEM, not Rockchip-specific;
#                                    works with virtio-gpu.ko which exposes GEM
#           DRI "2" removed          (modesetting ignores it with a warning; sets up DRI2 internally)
#           FlipFB "always" removed    (Rockchip/armsoc pageflip hint, not valid here)
#   Added:  Monitor "DSI-2" Ignore  (jog LCD connector reported connected by DRM;
#                                    EP122 owns CRTC 1 - tell X to leave it alone)
# Everything else (Monitor rotate/DPMS, Screen, ServerLayout) is unchanged.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

TARGET="$ROOTFS/etc/X11/xorg.conf.d/20-modesetting.conf"
[[ -f "$TARGET" ]] || { echo "  WARNING: $TARGET not found - skipping"; exit 0; }

cat > "$TARGET" << 'EOF'
Section "Device"
Identifier  "Rockchip Graphics"
    Driver      "modesetting"
    Option      "AccelMethod"    "exa"
EndSection

Section "Screen"
    Identifier  "Default Screen"
    Device      "Rockchip Graphics"
    Monitor     "Default Monitor"
EndSection

### Valid values for rotation are "normal", "left", "right"
Section "Monitor"
    Identifier  "Default Monitor"
    Option      "Rotate" "normal"
    Option      "DPMS"   "false"
EndSection

### Disable DPMS
Section "ServerLayout"
    Identifier  "ServerLayout0"
    Option      "BlankTime"    "0"
    Option      "StandbyTime"  "0"
    Option      "SuspendTime"  "0"
    Option      "OffTime"      "0"
EndSection

# DSI-2 = jog LCD (1280x240). Kernel reports it connected; tell X to ignore it.
# EP122 owns CRTC 1 exclusively via atomic DRM commits.
Section "Monitor"
    Identifier  "DSI-2"
    Option      "Ignore"  "true"
EndSection
EOF

echo "  -> 20-modesetting.conf replaced (AccelMethod=none, DRI/FlipFB removed, DSI-2 ignored)"
