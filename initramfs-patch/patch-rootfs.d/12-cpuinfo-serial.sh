#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 12: cpuinfo-serial.service - publish the SoC serial in /proc/cpuinfo.
#
# aarch64 prints no Serial line (it was an arm32 feature) and QEMU has no SoC
# serial to expose, so the guest's /proc/cpuinfo has none.  genkey_pr reads the
# value after ':' on that line and hashes it with the model to derive the
# cabinet.img LUKS passphrase, so without it the container carrying the Widevine
# keys and the Device Library Plus key file cannot be opened.
#
# The emulator passes cdj3k.serial=<16 hex digits> on the kernel cmdline from
# the slot's settings file.  This service appends the Serial line to a copy of
# /proc/cpuinfo under /run and bind-mounts it over the original, so every
# process - genkey_pr included - reads one value.  It runs before the app and
# before pre-setting.service, and is a no-op when the cmdline carries no serial.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"
: "${APP_UNIT:?APP_UNIT must be set by dispatcher}"

SERVICE_DIR="$ROOTFS/etc/systemd/system"
mkdir -p "$SERVICE_DIR" "$ROOTFS/usr/sbin"

cat > "$ROOTFS/usr/sbin/cpuinfo-serial.sh" << 'EOFS'
#!/bin/sh
# Publish cdj3k.serial=<hex> from the kernel cmdline as /proc/cpuinfo's Serial.
set -eu

# A kernel that prints its own Serial line owns the value.
if grep -q '^Serial' /proc/cpuinfo; then
    exit 0
fi

serial=
for word in $(cat /proc/cmdline); do
    case "$word" in
        cdj3k.serial=*) serial="${word#cdj3k.serial=}" ;;
    esac
done
[ -n "$serial" ] || exit 0

{ cat /proc/cpuinfo; printf 'Serial\t\t: %s\n' "$serial"; } > /run/cpuinfo
mount -o bind /run/cpuinfo /proc/cpuinfo
EOFS
chmod 755 "$ROOTFS/usr/sbin/cpuinfo-serial.sh"

cat > "$SERVICE_DIR/cpuinfo-serial.service" << SVCEOF
[Unit]
Description=Publish the SoC serial as the Serial line of /proc/cpuinfo
DefaultDependencies=no
RequiresMountsFor=/run
Before=pre-setting.service
Before=${APP_UNIT}
Before=multi-user.target

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/usr/sbin/cpuinfo-serial.sh

[Install]
WantedBy=multi-user.target
SVCEOF

chmod 644 "$SERVICE_DIR/cpuinfo-serial.service"

MULTI_USER_WANTS="$ROOTFS/etc/systemd/system/multi-user.target.wants"
mkdir -p "$MULTI_USER_WANTS"
ln -sf /etc/systemd/system/cpuinfo-serial.service \
    "$MULTI_USER_WANTS/cpuinfo-serial.service"

echo "  -> cpuinfo-serial.service installed and enabled"
