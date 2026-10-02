#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# build.sh - turn the staged tree into the three Linux artefacts.
#
#   build.sh [--version V] [--stage DIR] [--out DIR] [--only deb|rpm|appimage]
#
# One stage, three packagings, one architecture per run: the package
# description differs per arch (the x86_64 build runs under TCG).
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$REPO_ROOT/Cargo.toml" | head -n1)"
STAGE="$REPO_ROOT/dist/linux"
OUT="$REPO_ROOT/dist"
ONLY=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --version) VERSION="$2"; shift 2 ;;
        --stage)   STAGE="$2"; shift 2 ;;
        --out)     OUT="$2"; shift 2 ;;
        --only)    ONLY="$2"; shift 2 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done
[[ -d "$STAGE/opt/cdj3k-emu" ]] || {
    echo "ERROR: $STAGE is not a staged tree - run packaging/linux/stage.sh" >&2
    exit 1
}
mkdir -p "$OUT"

# The emulator is aarch64 whatever the host is; on x86_64 that means TCG,
# which is several times slower than a machine that can use KVM.
case "$(uname -m)" in
    x86_64|amd64)
        ARCH=amd64; APPIMAGE_ARCH=x86_64
        BLURB="This x86_64 build runs the guest under TCG emulation, which is substantially slower than the aarch64 build's KVM." ;;
    aarch64|arm64)
        ARCH=arm64; APPIMAGE_ARCH=aarch64
        BLURB="This aarch64 build uses KVM where the kernel offers it." ;;
    *) echo "ERROR: unsupported host architecture $(uname -m)" >&2; exit 1 ;;
esac
export VERSION ARCH STAGE BLURB

want() { [[ -z "$ONLY" || "$ONLY" == "$1" ]]; }

# ── deb and rpm ───────────────────────────────────────────────────────────────
if want deb || want rpm; then
    command -v nfpm >/dev/null || { echo "ERROR: nfpm not installed" >&2; exit 1; }
    command -v envsubst >/dev/null || { echo "ERROR: envsubst not installed" >&2; exit 1; }
    # nfpm does not expand environment variables, so the recipe is a template.
    RECIPE="$(mktemp -t nfpm.XXXXXX.yaml)"
    trap 'rm -f "$RECIPE"' EXIT
    envsubst '${VERSION} ${ARCH} ${BLURB}' \
        < "$REPO_ROOT/packaging/linux/nfpm.yaml.in" > "$RECIPE"
    for format in deb rpm; do
        want "$format" || continue
        echo "==> $format"
        # From inside the tree: the recipe names its sources relatively, so it
        # carries no path from the machine that built it.
        (cd "$STAGE" && nfpm package --config "$RECIPE" \
            --packager "$format" \
            --target "$OUT/CDJ3K-Emulator-${VERSION}-linux-${APPIMAGE_ARCH}.$format")
    done
fi

# ── AppImage ──────────────────────────────────────────────────────────────────
# Built from the same tree, with AppRun naming the payload because the mount
# point differs every launch.
if want appimage; then
    command -v appimagetool >/dev/null || {
        echo "ERROR: appimagetool not installed" >&2; exit 1; }
    echo "==> AppImage"
    APPDIR="$OUT/CDJ3K-Emulator.AppDir"
    rm -rf "$APPDIR"
    mkdir -p "$APPDIR"
    cp -a "$STAGE/opt" "$APPDIR/opt"
    install -m 0755 "$REPO_ROOT/packaging/linux/AppRun" "$APPDIR/AppRun"
    install -m 0644 "$REPO_ROOT/packaging/linux/cdj3k-emu.desktop" \
        "$APPDIR/cdj3k-emu.desktop"
    # appimagetool wants the icon at the root as well as in the theme.
    cp -a "$STAGE/usr/share" "$APPDIR/usr-share-tmp"
    mkdir -p "$APPDIR/usr/share"
    mv "$APPDIR/usr-share-tmp"/* "$APPDIR/usr/share/"
    rmdir "$APPDIR/usr-share-tmp"
    biggest=$(ls -1 "$APPDIR/usr/share/icons/hicolor" | sort -t x -k1 -n | tail -n1)
    cp "$APPDIR/usr/share/icons/hicolor/$biggest/apps/cdj3k-emu.png" \
        "$APPDIR/cdj3k-emu.png"

    ARCH="$APPIMAGE_ARCH" appimagetool --no-appstream "$APPDIR" \
        "$OUT/CDJ3K-Emulator-${VERSION}-linux-${APPIMAGE_ARCH}.AppImage"
    rm -rf "$APPDIR"
fi

echo "==> Artefacts in $OUT"
ls -la "$OUT" | grep -Ei 'deb$|rpm$|AppImage$' || true
