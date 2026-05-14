#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 06: fix dropbear ECDSA host key permissions
#
# dropbear refuses to start if the host key file is world-readable (mode 644).
# The Pioneer rootfs ships it at 644; tighten to 600.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

if [[ "${ENABLE_SSH:-0}" != "1" ]]; then
    echo "  -> SSH disabled (ENABLE_SSH != 1) - skipping dropbear key fix"
    exit 0
fi

KEY="$ROOTFS/etc/dropbear/dropbear_ecdsa_host_key"
if [[ -f "$KEY" ]]; then
    chmod 600 "$KEY"
    echo "  -> $KEY: 644 -> 600"
else
    echo "  WARNING: $KEY not found"
fi
