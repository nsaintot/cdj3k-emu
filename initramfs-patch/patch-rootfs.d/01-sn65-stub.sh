#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 01: stub the SN65DSI84 I2C check in apl_start.sh and skip aplay.
#
# The SN65DSI84 is a DSI-to-LVDS bridge on I2C bus 3. QEMU has no I2C bus 3,
# so i2cget hangs; the patch sets the result to 0x00, which passes the check.
# aplay silence.wav plays the boot chime on plughw:0,1 (card 0, device 1),
# which does not exist because virtio_snd provides device 0 only. Skip it.
# deck_shim.so is loaded through LD_PRELOAD in the app unit's drop-in
# 10-qemu.conf (patch 13).
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

TARGET="$ROOTFS/home/root/scripts/apl_start.sh"
[[ -f "$TARGET" ]] || { echo "  WARNING: $TARGET not found; skipping"; exit 0; }

sed -i \
    -e 's|SN65REG=\$(/sbin/i2cget -f -y 3 0x2c 0xe5)|SN65REG="0x00"  # Stubbed under QEMU.|g' \
    -e 's|SN65REG=\$(/sbin/i2cget -f -y 3 0x2d 0xe5)|SN65REG="0x00"  # Stubbed under QEMU.|g' \
    -e 's|/bin/aplay .*silence\.wav|true  # Skipped under QEMU: plughw:0,1 does not exist.|g' \
    "$TARGET"

echo "  -> SN65 I2C checks stubbed"
echo "  -> aplay silence.wav skipped: plughw:0,1 does not exist, and the boot chime is cosmetic"
