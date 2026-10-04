#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# update-index.sh VERSION TAG DIR [NOTES_MD] - write DIR/cdj3k-emu-update.json,
# the index the app's updater reads from the latest release: one entry per
# package in DIR, with its platform, architecture, format, URL, size and
# SHA-256, and the release notes as Markdown.
#
# Architectures use Rust's std::env::consts::ARCH names (x86_64, aarch64), or
# `universal` for a macOS bundle holding both. A file in DIR that is not a
# package fails the run.
set -euo pipefail
version="$1"
tag="$2"
dir="$3"
notes_md="${4:-/dev/null}"
: "${GITHUB_REPOSITORY:?}"
base="https://github.com/$GITHUB_REPOSITORY/releases/download/$tag"
out="cdj3k-emu-update.json"

entries=()
for path in "$dir"/*; do
    name=$(basename "$path")
    case "$name" in
        "$out" | "$out.sig" | SHA256SUMS.txt | appcast.xml) continue ;;
    esac
    re="^CDJ3K-Emulator-${version//./\\.}-(macos|linux|windows)-([A-Za-z0-9_]+)\\.(dmg|deb|rpm|AppImage|exe)$"
    if [[ ! "$name" =~ $re ]]; then
        echo "::error::$name is not a $version package" >&2
        exit 1
    fi
    os="${BASH_REMATCH[1]}"
    arch="${BASH_REMATCH[2]}"
    ext="${BASH_REMATCH[3]}"
    case "$arch" in
        x64 | x86_64) arch=x86_64 ;;
        arm64 | aarch64) arch=aarch64 ;;
        universal) ;;
        *) echo "::error::$name: unknown architecture $arch" >&2; exit 1 ;;
    esac
    case "$ext" in
        exe) kind=inno ;;
        AppImage) kind=appimage ;;
        *) kind="$ext" ;;
    esac
    entries+=("$(jq -n \
        --arg os "$os" --arg arch "$arch" --arg kind "$kind" \
        --arg url "$base/$name" \
        --argjson size "$(wc -c < "$path" | tr -d " ")" \
        --arg sha256 "$(sha256sum "$path" | cut -d' ' -f1)" \
        '{os: $os, arch: $arch, kind: $kind, url: $url, size: $size, sha256: $sha256}')")
done
[[ ${#entries[@]} -gt 0 ]] || { echo "::error::no packages in $dir" >&2; exit 1; }

printf '%s\n' "${entries[@]}" | jq -s \
    --arg version "$version" \
    --arg notes "https://github.com/$GITHUB_REPOSITORY/releases/tag/$tag" \
    --rawfile release_notes "$notes_md" \
    '{schema: 1, version: $version, notes: $notes, release_notes: $release_notes, packages: .}' > "$dir/$out"
cat "$dir/$out"
