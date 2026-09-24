#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 23: state files for hardware the guest does not have.
#
# The CDJ-3000X firmware runs wlan-monitor.sh for its Wi-Fi module; without
# the SDIO device the script exits before writing the files it owns, so they
# are created at boot by systemd-tmpfiles with the values it would have
# written on that same failure path. /var/volatile and /tmp are tmpfs.
#
# /var/net_mlan0_state.dat: absent, the app logs
# "[checkNetworkConnectionChange] ... open err." once a second.
#
# /tmp/ccode: absent, the app does not start. It stats the file every 100 ms
# and waits, while wlan-monitor.sh spends `for i in seq 1 10; sleep 1` looking
# for an SDIO device that will never appear before writing it - so the app's
# initApplication takes ~10 s longer than the CDJ-3000's, whose firmware has no
# such wait. "XX" is what the monitor writes when it finds no device, and
# mlan0Addr is written on the same path.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

if [[ ! -f "$ROOTFS/home/root/scripts/wlan-monitor.sh" ]]; then
    echo "  -> no wlan-monitor.sh in this rootfs - nothing to do"
    exit 0
fi

mkdir -p "$ROOTFS/etc/tmpfiles.d"
cat > "$ROOTFS/etc/tmpfiles.d/cdj3k-absent-hardware.conf" << 'EOFC'
# Wi-Fi link state the app polls; the module is absent under QEMU.
f /var/net_mlan0_state.dat 0644 root root - down
# Country code the app blocks on at startup, and the address beside it.
f /tmp/ccode 0644 root root - XX
f /tmp/mlan0Addr 0644 root root - 00:00:00:00:00:00
EOFC
echo "  -> tmpfiles.d/cdj3k-absent-hardware.conf: net_mlan0_state.dat, ccode, mlan0Addr"
