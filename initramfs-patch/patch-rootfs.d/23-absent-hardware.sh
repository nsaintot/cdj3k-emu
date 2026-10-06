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
# and waits. "XX" is what the monitor writes when it finds no device, and
# mlan0Addr is written on the same path.
#
# The unit that runs wlan-monitor.sh is masked: on its no-SDIO path the script
# writes the WLAN module caution (E-7026: WLAN module ERROR) to
# /proc/udev_usbctn1 - wlanbterr after a 10 s probe, or mlanbterr at once,
# depending on the firmware - and the files it would have written are
# provided here.  The unit is start-mlan0.service on the CDJ-3000X,
# start-wlan.service on the CDJ-1500X.
#
# mlan0 itself: service mode's version page reads the WLAN address from
# `ifconfig mlan0` and, while it finds none, re-runs it twice a second. A unit
# adds mlan0 with the guest kernel's built-in dummy driver (booted with
# dummy.numdummies=0, so there is no dummy0) and gives it eth0's address with
# the locally-administered bit set. It stays down, so the app's network code
# sees no link on it.
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

masked=0
for unit in "$ROOTFS"/etc/systemd/system/*.service; do
    [[ -f "$unit" && ! -L "$unit" ]] || continue
    grep -q '^ExecStart=.*/wlan-monitor\.sh' "$unit" || continue
    ln -sf /dev/null "$unit"
    echo "  -> masked $(basename "$unit") (no WLAN module under QEMU)"
    masked=1
done
# A rootfs patched before carries cdj3k-mlan0.service and the unit masked.
if [[ "$masked" == 0 && ! -f "$ROOTFS/etc/systemd/system/cdj3k-mlan0.service" ]]; then
    echo "ERROR: no unit runs wlan-monitor.sh in this rootfs" >&2
    exit 1
fi

cat > "$ROOTFS/etc/systemd/system/cdj3k-mlan0.service" << 'EOFU'
[Unit]
Description=mlan0 for the absent Wi-Fi module (a dummy interface, down)

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/bin/sh -c 'm=$$(cat /sys/class/net/eth0/address); ip link add mlan0 type dummy && ip link set mlan0 address 02:$${m#*:}'

[Install]
WantedBy=multi-user.target
EOFU
mkdir -p "$ROOTFS/etc/systemd/system/multi-user.target.wants"
ln -sf ../cdj3k-mlan0.service "$ROOTFS/etc/systemd/system/multi-user.target.wants/cdj3k-mlan0.service"
echo "  -> cdj3k-mlan0.service: mlan0 added as a dummy, down"
