#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# stage.sh - assemble the tree that every Linux package ships.
#
#   stage.sh [--out DIR] [--target TRIPLE] [--no-build] [--qemu DIR]
#
# One tree, three packagings: the deb and the rpm install it at
# /opt/cdj3k-emu, and the AppImage mounts it. Nothing below is format-specific.
#
#   <prefix>/bin/        cdj3k-emu, qemu-system-aarch64, qemu-img
#   <prefix>/lib/        the vendored dependency graph ($ORIGIN)
#   <prefix>/share/cdj3k-emu/
#                        Image, patch/, tools/
#
# The binary finds all of it relative to itself, so the prefix can move.
#
# Prerequisites, all from the repo's own builds:
#   qemu/install/bin/{qemu-system-aarch64,qemu-img}  - qemu/build.sh
#   build/Image, build/docker-out/                   - build.sh
#   guest/out/                                       - build.sh
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

OUT="$REPO_ROOT/dist/linux"
TARGET=""
DO_BUILD=1
# Where the patched QEMU for the *target* lives. Cross-building puts it
# somewhere other than the checkout's own `qemu/install`, which on a macOS
# work tree holds that host's build.
QEMU_DIR="${CDJ3K_QEMU_DIR:-$REPO_ROOT/qemu/install}"
BIN_DIR_OVERRIDE=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --out)     OUT="$2"; shift 2 ;;
        --target)  TARGET="$2"; shift 2 ;;
        --qemu)    QEMU_DIR="$2"; shift 2 ;;
        --binary)  BIN_DIR_OVERRIDE="$2"; shift 2 ;;
        --no-build) DO_BUILD=0; shift ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

PREFIX="$OUT/opt/cdj3k-emu"
BIN="$PREFIX/bin"
LIB="$PREFIX/lib"
SHARE="$PREFIX/share/cdj3k-emu"

rm -rf "$OUT"
mkdir -p "$BIN" "$LIB" "$SHARE"

# ── The emulator ──────────────────────────────────────────────────────────────
CARGO_TARGET_ARGS=()
BUILT_DIR="$REPO_ROOT/target/release"
if [[ -n "$TARGET" ]]; then
    CARGO_TARGET_ARGS=(--target "$TARGET")
    BUILT_DIR="$REPO_ROOT/target/$TARGET/release"
fi
if [[ -n "$BIN_DIR_OVERRIDE" ]]; then
    BUILT_DIR="$BIN_DIR_OVERRIDE"
    DO_BUILD=0
fi
if [[ $DO_BUILD -eq 1 ]]; then
    "$REPO_ROOT/winit/fetch.sh"
    echo "==> cargo build --release"
    (cd "$REPO_ROOT" && cargo build --release -p cdj3k-emu "${CARGO_TARGET_ARGS[@]}")
fi
if [[ ! -f "$BUILT_DIR/cdj3k-emu" ]]; then
    echo "ERROR: $BUILT_DIR/cdj3k-emu not found" >&2
    exit 1
fi
install -m 0755 "$BUILT_DIR/cdj3k-emu" "$BIN/cdj3k-emu"

# ── QEMU ──────────────────────────────────────────────────────────────────────
# Ours is patched, so a distro's qemu-system-aarch64 is not a substitute and
# the package never depends on one. /opt keeps it out of reach of $PATH, where
# it could be mistaken for the system's.
for tool in qemu-system-aarch64 qemu-img; do
    src="$QEMU_DIR/bin/$tool"
    if [[ ! -f "$src" ]]; then
        echo "ERROR: $src not found - run qemu/build.sh (see docker/Dockerfile.linux-host)," >&2
        echo "       or point --qemu / CDJ3K_QEMU_DIR at a build for this target" >&2
        exit 1
    fi
    install -m 0755 "$src" "$BIN/$tool"
done

# ── Payload ───────────────────────────────────────────────────────────────────
echo "==> Assembling share/cdj3k-emu"

"$REPO_ROOT/scripts/make-patch-dispatcher.sh" \
    "$REPO_ROOT/initramfs-patch/patch-rootfs.d" "$SHARE/patch/patch-rootfs.sh"

# Guest ELFs the patch steps install into the rootfs. 21-cfgd.sh aborts the
# patch without cfgd.
for tool in cfgd_aarch64 pc_link_bridge_aarch64 dropbear_aarch64 dropbear_aarch64.LICENSE; do
    src="$REPO_ROOT/guest/out/$tool"
    if [[ ! -f "$src" ]]; then
        echo "ERROR: guest/out/$tool not found - run ./build.sh" >&2
        exit 1
    fi
    mode=0755
    [[ "$tool" == *.LICENSE ]] && mode=0644
    install -m "$mode" "$src" "$SHARE/patch/$tool"
done

MODS="$REPO_ROOT/build/docker-out/modules"
if compgen -G "$MODS/*.ko" > /dev/null; then
    mkdir -p "$SHARE/patch/vanilla-modules"
    install -m 0644 "$MODS"/*.ko "$SHARE/patch/vanilla-modules/"
    echo "     modules: $(ls "$MODS"/*.ko | wc -l | tr -d ' ')"
else
    echo "ERROR: $MODS/*.ko not found - run ./build.sh" >&2
    exit 1
fi

DUMMY="$REPO_ROOT/build/docker-out/dummy_drv.so"
if [[ -f "$DUMMY" ]]; then
    install -m 0644 "$DUMMY" "$SHARE/patch/dummy_drv.so"
else
    echo "ERROR: $DUMMY not found - run ./build.sh" >&2
    exit 1
fi

mkdir -p "$SHARE/tools"
for tool in subucom_live subucom_forwarder; do
    src="$REPO_ROOT/guest/out/${tool}_aarch64"
    if [[ ! -f "$src" ]]; then
        echo "ERROR: $src not found - run ./build.sh --modules-only" >&2
        exit 1
    fi
    install -m 0755 "$src" "$SHARE/tools/$tool"
done
if [[ ! -f "$REPO_ROOT/guest/out/deck_shim.so" ]]; then
    echo "ERROR: guest/out/deck_shim.so not found - run: make -C guest" >&2
    exit 1
fi
install -m 0644 "$REPO_ROOT/guest/out/deck_shim.so" "$SHARE/tools/deck_shim.so"

if [[ ! -f "$REPO_ROOT/build/Image" ]]; then
    echo "ERROR: build/Image not found - run ./build.sh" >&2
    exit 1
fi
install -m 0644 "$REPO_ROOT/build/Image" "$SHARE/Image"

# ── Desktop integration ───────────────────────────────────────────────────────
# The only files outside the prefix: a launcher on $PATH, a menu entry and the
# icons a desktop looks for by name. The deb and the rpm declare the symlink
# themselves; it is written here too so the staged tree can be copied onto a
# machine as-is.
mkdir -p "$OUT/usr/bin" "$OUT/usr/share/applications"
ln -sf /opt/cdj3k-emu/bin/cdj3k-emu "$OUT/usr/bin/cdj3k-emu"
install -m 0644 "$REPO_ROOT/packaging/linux/cdj3k-emu.desktop" \
    "$OUT/usr/share/applications/cdj3k-emu.desktop"

ICON_SRC="$REPO_ROOT/app/cdj3k-emu/assets/icon_1024_padded.png"
if command -v magick >/dev/null || command -v convert >/dev/null; then
    CONVERT=$(command -v magick || command -v convert)
    for size in 32 48 64 128 256 512; do
        dir="$OUT/usr/share/icons/hicolor/${size}x${size}/apps"
        mkdir -p "$dir"
        "$CONVERT" "$ICON_SRC" -resize "${size}x${size}" "$dir/cdj3k-emu.png"
    done
    echo "     icons: 32..512"
else
    # Without ImageMagick, one icon; a desktop scales what it finds.
    dir="$OUT/usr/share/icons/hicolor/512x512/apps"
    mkdir -p "$dir"
    install -m 0644 "$ICON_SRC" "$dir/cdj3k-emu.png"
    echo "WARNING: ImageMagick not found - shipping the 1024px icon as 512x512"
fi

# ── Vendoring ─────────────────────────────────────────────────────────────────
"$REPO_ROOT/scripts/bundle-sos.sh" "$BIN" "$LIB" \
    "$BIN/cdj3k-emu" "$BIN/qemu-system-aarch64" "$BIN/qemu-img"

# ── Library resolution check ──────────────────────────────────────────────────
# Every staged binary must resolve all its libraries from the staged tree.
echo "==> Checking every binary resolves"
unresolved=0
for exe in "$BIN"/*; do
    [[ -f "$exe" && -x "$exe" ]] || continue
    while read -r line; do
        echo "     $(basename "$exe"): $line" >&2
        unresolved=$((unresolved + 1))
    done < <(ldd "$exe" 2>/dev/null | grep "not found" || true)
done
if [[ $unresolved -gt 0 ]]; then
    echo "ERROR: $unresolved unresolved library reference(s); the package would not run" >&2
    exit 1
fi
echo "     all binaries resolve"

echo "==> Staged $(du -sh "$OUT" | cut -f1) at $OUT"
