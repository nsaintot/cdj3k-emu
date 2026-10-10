#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 29: pc-link-bridge, which forwards HID and MIDI between the gadget and
# the host.
#
# The bridge forwards the HID and MIDI endpoints of the USB gadget on
# dummy_hcd to the `cdj3k.usb-link` virtio-serial port. cdj3k-emu-runtime
# ends the link on the host; on macOS, HID frames become an IOHIDUserDevice
# and MIDI frames go to the CoreMIDI driver plugin. See docs/pc-link.md.
#
# The unit is installed but not enabled. cfgd starts or stops it when the host
# toggles `pc_link`. Until then the daemon stays silent, so the gadget
# endpoints sit idle and the host sees nothing, as with an unplugged cable.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"
: "${PATCH_ASSETS_DIR:?PATCH_ASSETS_DIR must be set by dispatcher}"
: "${APP_NAME:?APP_NAME must be set by dispatcher}"

# Find the binary: packaging/macos/stage.sh stages it into PATCH_ASSETS_DIR,
# and build.sh leaves it in guest/out/, as patch 21 does for cfgd.
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
Description=PC Link bridge between the gadget's HID and MIDI endpoints and cdj3k.usb-link
After=start-usb-gadget.service
Wants=start-usb-gadget.service
StartLimitIntervalSec=0

[Service]
Type=simple
ExecStart=/usr/sbin/cdj3k-pc-link-bridge
# APP_NAME names the app process; pcmode.c forces PC mode only on the apps
# that need it.
Environment=APP_NAME=$APP_NAME
Restart=always
RestartSec=2s

# There is no [Install] section: cfgd starts and stops the unit, so it must
# not start at boot.
SVCEOF
chmod 644 "$SERVICE_DIR/cdj3k-pc-link-bridge.service"
echo "  -> cdj3k-pc-link-bridge.service installed (cfgd starts it)"
