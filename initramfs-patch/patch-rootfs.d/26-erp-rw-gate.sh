#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 26: a missing ERP_RW GPIO reads LOW (CDJ-1500X).
#
# Updater.service runs /home/root/Startup on every boot. Unless U-Boot's
# boot_type is "update", it exports GPIO 23 (ERP_RW, from the ErP sub-CPU) and
# exits when the line reads 0; any other value starts the USB updater, which
# waits for an update stick for good, and pre-setting.service, ordered after
# Updater.service, never starts the app. QEMU has no such GPIO, so the read
# comes back empty; the patch turns that into 0, the reading of a deck that is
# not being updated.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"
: "${APP_SLUG:?APP_SLUG must be set by dispatcher}"

if [[ "$APP_SLUG" != cdj1500x ]]; then
    echo "  -> no ERP_RW gate on this player - nothing to do"
    exit 0
fi

STARTUP="$ROOTFS/home/root/Startup"
READ='ERP_RW=$(cat /sys/class/gpio/gpio23/value)'
if ! grep -qF "$READ" "$STARTUP"; then
    echo "ERROR: $STARTUP no longer reads ERP_RW as expected" >&2
    exit 1
fi
sed -i 's|ERP_RW=$(cat /sys/class/gpio/gpio23/value)$|ERP_RW=$(cat /sys/class/gpio/gpio23/value 2>/dev/null \|\| echo 0)|' "$STARTUP"
echo "  -> /home/root/Startup: a missing ERP_RW GPIO reads LOW"
