#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# initramfs-patch/patch-rootfs.sh
#
# Dispatcher: runs every script in patch-rootfs.d/ (in numeric order).
#
# Usage:
#   ./patch-rootfs.sh <path/to/initramfs-root>
#
# Each script in patch-rootfs.d/ receives these environment variables:
#   ROOTFS           - path to the extracted initramfs rootfs (from $1)
#   PATCH_ASSETS_DIR - path to this directory (contains cfgd_aarch64 binary from tools/)
#   APP_UNIT         - the player application's systemd unit, detected from the
#                      rootfs: EP122.service (CDJ-3000) or EP145.service (CDJ-3000X)
#   APP_NAME         - APP_UNIT without the .service suffix (the process name)
#   APP_SLUG         - the slug the emulator names this player by: cdj3k or cdj3kx

set -euo pipefail

ROOTFS="${1:?Usage: $0 <initramfs-root>}"
export ROOTFS
export PATCH_ASSETS_DIR="$(cd "$(dirname "$0")" && pwd)"

# Steps that order a service before the app, or install a drop-in for it, go
# through APP_UNIT so one patch set serves both models.
APP_UNIT="$(cd "$ROOTFS/etc/systemd/system" 2>/dev/null && ls EP1[0-9][0-9].service 2>/dev/null | head -n1 || true)"
export APP_UNIT="${APP_UNIT:-EP122.service}"
export APP_NAME="${APP_UNIT%.service}"
case "$APP_UNIT" in
    EP145.service) export APP_SLUG="cdj3kx" ;;
    *)             export APP_SLUG="cdj3k" ;;
esac

PATCH_D="$PATCH_ASSETS_DIR/patch-rootfs.d"

echo "=== Patching initramfs rootfs at: $ROOTFS (app unit: $APP_UNIT) ==="
echo ""

shopt -s nullglob
scripts=("$PATCH_D"/[0-9]*.sh)

if [[ ${#scripts[@]} -eq 0 ]]; then
    echo "ERROR: no patch scripts found in $PATCH_D" >&2
    exit 1
fi

for script in "${scripts[@]}"; do
    echo "--- $(basename "$script") ---"
    bash "$script"
    echo ""
done

echo "=== All patches applied ==="
