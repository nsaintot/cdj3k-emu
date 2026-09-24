#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 14: cabinet-install.service - put cabinet.img on the settings partition.
#
# The deck keeps its Widevine keys and the Device Library Plus key file
# (settings/cabinet/encryption/lsdk.dat) inside /home/root/settings/cabinet.img,
# a LUKS container.  The vendor updater copies that file onto p7, and
# apl_start.sh opens it on every boot via `genkey_pr | initoptenv`, which
# derives the passphrase from the model and the /proc/cpuinfo Serial that guest
# patch 12 publishes.  Nothing re-keys the container, so it opens only for the
# unit it was keyed for.
#
# The emulator's installer stages the image raw at the start of the recovery
# partition behind a 512-byte ASCII header.  This service copies it onto the
# settings partition after emmc-fs.sh has formatted and mounted it and before
# the app starts.  The copy goes to a temporary file that is size- and
# magic-checked before it is renamed into place, so a full partition leaves no
# cabinet.img and the next boot retries.  An installed cabinet with the staged
# image's size and the LUKS magic is left alone.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"
: "${APP_UNIT:?APP_UNIT must be set by dispatcher}"

SERVICE_DIR="$ROOTFS/etc/systemd/system"
mkdir -p "$SERVICE_DIR" "$ROOTFS/usr/sbin"

cat > "$ROOTFS/usr/sbin/cabinet-install.sh" << 'EOFS'
#!/bin/sh
# Copy the staged cabinet.img onto the settings partition.  A cabinet already
# there is kept when its size matches the staged image and it starts with the
# LUKS magic; otherwise it is replaced.
set -eu
# Byte-wise tr and string comparison for the binary magic.
export LC_ALL=C

DEST=/home/root/settings/cabinet.img
TMP=$DEST.tmp
STAGE=/dev/mmcblk1p4

file_size() {
    ls -ln "$1" 2>/dev/null | awk '{ print $5 }'
}

has_luks_magic() {
    [ "$(dd if="$1" bs=6 count=1 2>/dev/null | tr -d '\000')" = "$(printf 'LUKS\272\276')" ]
}

[ -b "$STAGE" ] || exit 0
# Only write to the real settings partition, never to the rootfs ramfs.
grep -q ' /home/root/settings ' /proc/mounts || exit 0

hdr=$(dd if="$STAGE" bs=512 count=1 2>/dev/null | tr -d '\000')
case "$hdr" in
    CDJ3KCAB1*) ;;
    *) exit 0 ;;
esac

size=$(printf '%s\n' "$hdr" | sed -n 's/^size=//p')
[ -n "$size" ] || exit 0

# dd copies whole sectors, so the installed file is the image padded to 512.
blocks=$(( (size + 511) / 512 ))
expected=$(( blocks * 512 ))

rm -f "$TMP"
if [ -e "$DEST" ]; then
    if [ "$(file_size "$DEST")" = "$expected" ] && has_luks_magic "$DEST"; then
        exit 0
    fi
    echo "cabinet-install: ${DEST} is truncated or invalid, reinstalling" >&2
    rm -f "$DEST"
fi

if ! dd if="$STAGE" bs=512 skip=1 count="$blocks" of="$TMP" 2>/dev/null; then
    rm -f "$TMP"
    echo "cabinet-install: copy to ${TMP} failed, retrying next boot" >&2
    exit 1
fi
got=$(file_size "$TMP")
if [ "$got" != "$expected" ] || ! has_luks_magic "$TMP"; then
    rm -f "$TMP"
    echo "cabinet-install: ${TMP} is ${got:-?} of ${expected} bytes or lacks the LUKS magic, retrying next boot" >&2
    exit 1
fi
sync
mv "$TMP" "$DEST"
sync
echo "cabinet-install: ${size} bytes -> ${DEST}"
EOFS
chmod 755 "$ROOTFS/usr/sbin/cabinet-install.sh"

cat > "$SERVICE_DIR/cabinet-install.service" << SVCEOF
[Unit]
Description=Install cabinet.img onto the settings partition
After=emmc-fs.service
Before=${APP_UNIT}

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/usr/sbin/cabinet-install.sh

[Install]
WantedBy=multi-user.target
SVCEOF

chmod 644 "$SERVICE_DIR/cabinet-install.service"

MULTI_USER_WANTS="$ROOTFS/etc/systemd/system/multi-user.target.wants"
mkdir -p "$MULTI_USER_WANTS"
ln -sf /etc/systemd/system/cabinet-install.service \
    "$MULTI_USER_WANTS/cabinet-install.service"

echo "  -> cabinet-install.service installed and enabled"
