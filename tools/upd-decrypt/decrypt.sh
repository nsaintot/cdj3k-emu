#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Runs INSIDE the cdj3k-upd-decrypt container.
# Expects:
#   /in/upd   - input .UPD (read-only bind)
#   /in/key   - keyfile (read-only bind)
#   /out      - output dir (rw bind); inner ISO is copied to /out/out.img

set -euo pipefail

IN=/in/upd
KEY=/in/key
OUT=/out/${OUT_NAME:-out.img}

[[ -f $IN  ]] || { echo "missing /in/upd" >&2; exit 1; }
[[ -f $KEY ]] || { echo "missing /in/key" >&2; exit 1; }

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
echo "[+] attached $IN -> $LOOP"

cryptsetup open --readonly --type luks --key-file "$KEY" "$LOOP" "$MAP"
echo "[+] LUKS opened as /dev/mapper/$MAP"

mount -o ro "/dev/mapper/$MAP" "$MNT"
echo "[+] mounted at $MNT"

echo "[+] container contents:"
ls -la "$MNT" "$MNT/images" 2>/dev/null || true

ISO=$(ls "$MNT"/images/*.iso 2>/dev/null | head -n1 || true)
if [[ -z $ISO ]]; then
  echo "no ISO found under $MNT/images/" >&2
  exit 2
fi

echo "[+] copying $(basename "$ISO") -> $OUT"
cp "$ISO" "$OUT"
sync
echo "[+] done. size=$(stat -c%s "$OUT") bytes"
