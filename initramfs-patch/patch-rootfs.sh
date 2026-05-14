#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# initramfs-patch/patch-rootfs.sh
#
# Dispatcher: runs every script in patch-rootfs.d/ (in numeric order).
#
# Usage:
#   ./patch-rootfs.sh <path/to/initramfs-root>
#
# Each script in patch-rootfs.d/ receives two environment variables:
#   ROOTFS           - path to the extracted initramfs rootfs (from $1)
#   PATCH_ASSETS_DIR - path to this directory (contains cfgd_aarch64 binary from tools/)

set -euo pipefail

ROOTFS="${1:?Usage: $0 <initramfs-root>}"
export ROOTFS
export PATCH_ASSETS_DIR="$(cd "$(dirname "$0")" && pwd)"

PATCH_D="$PATCH_ASSETS_DIR/patch-rootfs.d"

echo "=== Patching initramfs rootfs at: $ROOTFS ==="
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
