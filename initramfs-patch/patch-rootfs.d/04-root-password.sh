#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 04: clear the root password for passwordless SSH.
#
# dropbear's override.conf starts it with -B, which allows blank passwords.
# This patch also clears root's password hash in /etc/shadow, so a blank
# password is accepted: `ssh -p 2222 root@localhost` asks for no password.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

if [[ "${ENABLE_SSH:-0}" != "1" ]]; then
    echo "  -> SSH disabled (ENABLE_SSH != 1); /etc/shadow is left unchanged"
    exit 0
fi

SHADOW="$ROOTFS/etc/shadow"
if [[ -f "$SHADOW" ]]; then
    sed -i 's/^root:[^:]*:/root::/' "$SHADOW"
    echo "  -> root password hash cleared (passwordless SSH enabled)"
else
    echo "  WARNING: $SHADOW not found"
fi
