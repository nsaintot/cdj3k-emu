#!/bin/sh
# Build the E2E USB image: a 64 MiB MBR disk with one FAT32 partition labelled
# E2EUSB. It holds generated WAV tracks whose tempo is in their name:
#
#   Tracks/A440 120bpm.wav
#   Tracks/C523 128bpm.wav
#   Tracks/Slow/E330 90bpm.wav
#
# Usage: make-usb.sh [out.img]   (default: e2e-usb.img next to this script)
#
# macOS uses hdiutil. Linux needs sfdisk, mkfs.vfat (dosfstools) and mtools.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
out=${1:-$here/e2e-usb.img}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

python3 -I "$here/make-tracks.py" "$work/Tracks"
rm -f "$out"

case "$(uname -s)" in
Darwin)
    hdiutil create -quiet -size 64m -layout MBRSPUD -fs "MS-DOS FAT32" \
        -volname E2EUSB -type UDIF "$work/usb.dmg"
    volume=$(hdiutil attach -nobrowse "$work/usb.dmg" | awk -F'\t' '/E2EUSB/ {print $NF}')
    cp -R "$work/Tracks" "$volume/"
    hdiutil detach -quiet "$volume"
    hdiutil convert -quiet "$work/usb.dmg" -format UDTO -o "$work/usb"
    mv "$work/usb.cdr" "$out"
    ;;
Linux)
    # sfdisk and mkfs.vfat live in sbin, which a user's PATH may lack.
    PATH=$PATH:/usr/sbin:/sbin
    truncate -s 64M "$out"
    echo 'start=2048, type=c' | sfdisk -q "$out"
    mkfs.vfat -F 32 -s 1 -n E2EUSB --offset 2048 "$out" >/dev/null
    MTOOLS_SKIP_CHECK=1 mcopy -s -i "$out@@1M" "$work/Tracks" ::/
    ;;
*)
    echo "make-usb.sh: $(uname -s) is not supported" >&2
    exit 1
    ;;
esac
echo "$out"
