#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Runs inside the cdj3k-upd-decrypt container and expects these binds:
#   /in/upd   - the input .UPD (read-only)
#   /in/key   - the keyfile (read-only)
#   /out      - the output directory (read-write); the inner ISO is copied to
#               /out/$OUT_NAME, by default /out/out.img

set -euo pipefail

IN=/in/upd
KEY=/in/key
OUT=/out/${OUT_NAME:-out.img}

[[ -f $IN  ]] || { echo "/in/upd is missing." >&2; exit 1; }
[[ -f $KEY ]] || { echo "/in/key is missing." >&2; exit 1; }

MAP=upd_decrypt
MNT=/mnt/upd
LOOP=""

cleanup() {
  set +e
  mountpoint -q "$MNT" && umount "$MNT"
  [[ -e /dev/mapper/$MAP ]] && cryptsetup close "$MAP"
  [[ -n $LOOP ]] && losetup -d "$LOOP"
}
trap cleanup EXIT

mkdir -p "$MNT"

LOOP=$(losetup --find --show --read-only "$IN")
echo "[+] attached $IN to $LOOP"

cryptsetup open --readonly --type luks --key-file "$KEY" "$LOOP" "$MAP"
echo "[+] LUKS opened as /dev/mapper/$MAP"

mount -o ro "/dev/mapper/$MAP" "$MNT"
echo "[+] mounted at $MNT"

echo "[+] container contents:"
ls -la "$MNT" "$MNT/images" 2>/dev/null || true

ISO=$(ls "$MNT"/images/*.iso 2>/dev/null | head -n1 || true)
if [[ -z $ISO ]]; then
  echo "No ISO was found under $MNT/images/." >&2
  exit 2
fi

echo "[+] copying $(basename "$ISO") to $OUT"
cp "$ISO" "$OUT"
sync
echo "[+] done; wrote $(stat -c%s "$OUT") bytes."
