#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 29: pc-link-bridge - host↔gadget HID + MIDI pump.
#
# Pumps the dummy_hcd-attached USB gadget's HID + MIDI endpoints out to the
# `cdj3k.usb-link` virtio-serial port.  cdj3k-emu-runtime terminates it:
# HID frames become an IOHIDUserDevice, MIDI frames go to the CoreMIDI
# driver plugin.  See docs/pc-link.md.
#
# IMPORTANT: the unit is installed but NOT enabled.  cfgd starts/stops it
# on demand when the host toggles `pc_link` on/off.  Until then the daemon
# stays silent so the gadget endpoints sit idle and the host sees nothing,
# matching the user's "unplugged cable" mental model.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"
: "${PATCH_ASSETS_DIR:?PATCH_ASSETS_DIR must be set by dispatcher}"
: "${APP_NAME:?APP_NAME must be set by dispatcher}"

# Locate the binary: bundle.sh stages it into PATCH_ASSETS_DIR, build.sh
# leaves it in guest/out/ (mirrors patch 21 for cfgd).
if [[ -f "$PATCH_ASSETS_DIR/pc_link_bridge_aarch64" ]]; then
    BIN="$PATCH_ASSETS_DIR/pc_link_bridge_aarch64"
elif [[ -f "$PATCH_ASSETS_DIR/../guest/out/pc_link_bridge_aarch64" ]]; then
    BIN="$PATCH_ASSETS_DIR/../guest/out/pc_link_bridge_aarch64"
else
    echo "ERROR: pc_link_bridge_aarch64 not found in $PATCH_ASSETS_DIR or guest/out/" >&2
    echo "       Run: cd guest && make docker" >&2
    exit 1
fi
install -m 0755 "$BIN" "$ROOTFS/usr/sbin/cdj3k-pc-link-bridge"
echo "  -> /usr/sbin/cdj3k-pc-link-bridge installed"

SERVICE_DIR="$ROOTFS/etc/systemd/system"
mkdir -p "$SERVICE_DIR"
cat > "$SERVICE_DIR/cdj3k-pc-link-bridge.service" << SVCEOF
[Unit]
Description=PC-link bridge (HID + MIDI ↔ cdj3k.usb-link)
After=start-usb-gadget.service
Wants=start-usb-gadget.service
StartLimitIntervalSec=0

[Service]
Type=simple
ExecStart=/usr/sbin/cdj3k-pc-link-bridge
# The app process: pcmode.c forces PC mode only on apps that need it.
Environment=APP_NAME=$APP_NAME
Restart=always
RestartSec=2s

# Deliberately no [Install] section: cfgd controls the unit lifecycle, so we
# don't want a multi-user.target.wants symlink auto-starting it at boot.
SVCEOF
chmod 644 "$SERVICE_DIR/cdj3k-pc-link-bridge.service"
echo "  -> cdj3k-pc-link-bridge.service installed (manual start via cfgd)"
