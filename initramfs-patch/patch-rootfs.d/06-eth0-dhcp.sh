#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 10: /etc/network/interfaces - add eth0 DHCP stanza
#
# The original interfaces file only configures 'lo'. virtio_net.ko creates eth0
# but ifup -a won't bring it up without this entry. QEMU SLIRP assigns 10.0.2.15
# via its built-in DHCP; host:2222 -> guest:22 forwarding reaches dropbear once
# eth0 has an IP.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

IFACES="$ROOTFS/etc/network/interfaces"
if ! grep -q "eth0" "$IFACES" 2>/dev/null; then
    printf '\n# virtio-net - QEMU SLIRP (10.0.2.15/24 from built-in DHCP)\nauto eth0\niface eth0 inet dhcp\n' \
        >> "$IFACES"
    echo "  -> eth0 inet dhcp appended to /etc/network/interfaces"
else
    echo "  -> eth0 already present, skipping"
fi
