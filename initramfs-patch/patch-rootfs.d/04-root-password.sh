#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 08: clear root password for passwordless SSH
#
# dropbear is started with -B (allow blank passwords) via its override.conf.
# We also clear root's password hash in /etc/shadow so a blank password is
# accepted: ssh -p 2222 root@localhost  (no password prompt in the QEMU dev VM).
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

if [[ "${ENABLE_SSH:-0}" != "1" ]]; then
    echo "  -> SSH disabled (ENABLE_SSH != 1) - leaving /etc/shadow untouched"
    exit 0
fi

SHADOW="$ROOTFS/etc/shadow"
if [[ -f "$SHADOW" ]]; then
    sed -i '' 's/^root:[^:]*:/root::/' "$SHADOW"
    echo "  -> root password hash cleared (passwordless SSH enabled)"
else
    echo "  WARNING: $SHADOW not found"
fi
