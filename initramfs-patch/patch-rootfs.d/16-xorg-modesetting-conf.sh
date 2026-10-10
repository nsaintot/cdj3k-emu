#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 16: replace xorg.conf.d/20-modesetting.conf for virtio-gpu. X uses it
# only when /dev/dri/card0 exists (see patch 25).
#
# Changes from the Pioneer original:
#   Device:  AccelMethod stays "exa", modesetting's internal EXA over DRM/GEM,
#            which virtio-gpu exposes. It is "none" on the CDJ-1500X, whose
#            modesetting build allocates its own EXA offscreen pixmaps, which
#            virtio-gpu refuses.
#            DRI "2" is removed: modesetting ignores it with a warning and sets
#            up DRI2 itself.
#            FlipFB "always" is removed: it is a Rockchip/armsoc page-flip hint
#            that does not apply here.
#   Added:   Monitor "DSI-2" Ignore. DRM reports the jog LCD connector as
#            connected and EP122 owns CRTC 1, so X is told to leave it alone.
# Everything else (Monitor rotate and DPMS, Screen, ServerLayout) is unchanged.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"
: "${APP_SLUG:?APP_SLUG must be set by dispatcher}"

case "$APP_SLUG" in
    cdj1500x) ACCEL=none ;;
    *)        ACCEL=exa ;;
esac

TARGET="$ROOTFS/etc/X11/xorg.conf.d/20-modesetting.conf"
[[ -f "$TARGET" ]] || { echo "  WARNING: $TARGET not found; skipping"; exit 0; }

cat > "$TARGET" << EOF
Section "Device"
Identifier  "Rockchip Graphics"
    Driver      "modesetting"
    Option      "AccelMethod"    "$ACCEL"
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

# DSI-2 is the jog LCD (1280x240). The kernel reports it as connected, so X is
# told to ignore it. EP122 owns CRTC 1 through atomic DRM commits.
Section "Monitor"
    Identifier  "DSI-2"
    Option      "Ignore"  "true"
EndSection
EOF

echo "  -> 20-modesetting.conf replaced (AccelMethod=$ACCEL, DRI/FlipFB removed, DSI-2 ignored)"
