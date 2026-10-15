#!/bin/sh
# Download a deck's firmware from AlphaTheta, check it against firmware.sha256,
# and print the path of its .UPD.
#
# Usage: fetch-firmware.sh <model> [release] [--pin]
#   model     cdj3k, cdj3kx or cdj1500x
#   release   such as 3.20; the newest release in AlphaTheta's list by default
#   --pin     add the archive's SHA-256 to firmware.sha256 when it has none
#
# The archives are kept in $CDJ3K_FIRMWARE_CACHE, by default
# ~/.cache/cdj3k-emu/firmware. An archive without a pinned hash is refused.
set -eu

FEED=https://dl.rekordbox.com/application/fwupdm/firm_version_info.txt
here=$(cd "$(dirname "$0")" && pwd)
pins="$here/firmware.sha256"
cache=${CDJ3K_FIRMWARE_CACHE:-$HOME/.cache/cdj3k-emu/firmware}

model=${1:-}
release=${2:-}
pin=no
case "$release" in --pin) release=; pin=yes ;; esac
[ "${3:-}" = --pin ] && pin=yes

case "$model" in
cdj3k) product=CDJ-3000 ;;
cdj3kx) product=CDJ-3000X ;;
cdj1500x) product=CDJ-1500X ;;
*)
    echo "usage: fetch-firmware.sh cdj3k|cdj3kx|cdj1500x [release] [--pin]" >&2
    exit 2
    ;;
esac

# The list has one line per product and host: PRODUCT,VERSION,Mac|Win,URL,
# where VERSION is the release without its dot, such as 322 for 3.22.
line=$(curl -sSfL "$FEED" | tr -d '\r' | grep "^$product,[0-9]*,Mac," | head -1)
if [ -z "$line" ]; then
    echo "fetch-firmware.sh: AlphaTheta's list has no $product" >&2
    exit 1
fi
latest=$(echo "$line" | cut -d, -f2)
url=$(echo "$line" | cut -d, -f4)
if [ -n "$release" ]; then
    case "$release" in
    *[!0-9.]* | .* | *. | *..*)
        echo "fetch-firmware.sh: $release is not a release such as 3.20" >&2
        exit 2
        ;;
    esac
    # Older releases sit next to the newest one, under the same name pattern.
    wanted=$(echo "$release" | tr -d .)
    newest_url=$url
    url=$(echo "$url" | sed "s/$latest\(\.[A-Za-z]*\)\$/$wanted\1/")
    if [ "$wanted" != "$latest" ] && [ "$url" = "$newest_url" ]; then
        echo "fetch-firmware.sh: no archive name for $product $release in $newest_url" >&2
        exit 1
    fi
fi
zip=$(basename "$url")

mkdir -p "$cache"
if [ ! -f "$cache/$zip" ]; then
    curl -sSfL -o "$cache/$zip.part" "$url"
    mv "$cache/$zip.part" "$cache/$zip"
fi

if command -v sha256sum >/dev/null; then
    sum=$(sha256sum "$cache/$zip" | cut -d' ' -f1)
else
    sum=$(shasum -a 256 "$cache/$zip" | cut -d' ' -f1)
fi
pinned=$(grep "  $zip\$" "$pins" 2>/dev/null | cut -d' ' -f1 || true)
if [ -z "$pinned" ]; then
    if [ "$pin" = no ]; then
        echo "fetch-firmware.sh: $zip has no hash in firmware.sha256; check it and pass --pin" >&2
        exit 1
    fi
    echo "$sum  $zip" >> "$pins"
elif [ "$pinned" != "$sum" ]; then
    echo "fetch-firmware.sh: $zip does not match its pinned hash" >&2
    exit 1
fi

out="$cache/${zip%.*}"
if [ ! -d "$out" ]; then
    rm -rf "$out.part"
    mkdir -p "$out.part"
    unzip -q "$cache/$zip" -d "$out.part"
    mv "$out.part" "$out"
fi
upd=$(find "$out" -iname '*.upd' | head -1)
if [ -z "$upd" ]; then
    echo "fetch-firmware.sh: $zip holds no .UPD" >&2
    exit 1
fi
echo "$upd"
