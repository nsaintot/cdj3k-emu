#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 01: apl_start.sh - SN65DSI84 I2C check stub + aplay skip
#
# The SN65DSI84 is a DSI-to-LVDS bridge controlled via I2C bus 3.
# In QEMU there is no I2C bus 3, so i2cget hangs. Stub the result to 0x00 (pass).
# aplay silence.wav uses plughw:0,1 (card 0, device 1) which does not exist in QEMU.
# The chime uses plughw:0,1 which doesn't exist (virtio_snd is device 0 only). Skip it.
# ep122_shim.so is loaded via LD_PRELOAD in EP122.service.d/10-qemu.conf (patch 13).
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

TARGET="$ROOTFS/home/root/scripts/apl_start.sh"
[[ -f "$TARGET" ]] || { echo "  WARNING: $TARGET not found -- skipping"; exit 0; }

sed -i '' \
    -e 's|SN65REG=\$(/sbin/i2cget -f -y 3 0x2c 0xe5)|SN65REG="0x00"  # QEMU: stubbed|g' \
    -e 's|SN65REG=\$(/sbin/i2cget -f -y 3 0x2d 0xe5)|SN65REG="0x00"  # QEMU: stubbed|g' \
    -e 's|/bin/aplay .*silence\.wav|true  # QEMU: skip aplay silence.wav (plughw:0,1 absent)|g' \
    "$TARGET"

echo "  -> SN65 I2C checks stubbed"
echo "  -> aplay silence.wav skipped (plughw:0,1 does not exist; boot chime is cosmetic)"
