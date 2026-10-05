#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# stage.sh - assemble the macOS .app that packaging/macos/build.sh signs.
#
#   stage.sh [--arch arm64|x86_64|universal] [--out DIR] [--debug] [--no-build]
#            [--version V] [--build N]
#
#   --arch           the release .app's architectures: arm64 (Apple silicon
#                    only), x86_64, or universal (both; an Intel Mac runs the
#                    guest under TCG).  Each slice links the QEMU of
#                    `qemu/build.sh --macos <arch>` (qemu/install-macos-<arch>/)
#                    and builds for the macOS 15.0 deployment target.
#                    Without it: a dev .app of this Mac's arch, linking
#                    qemu/install/ (qemu/build.sh), for this Mac's macOS.
#   --out DIR        where CDJ3K Emulator.app goes (default: dist/macos)
#   --debug          build the debug profile (default: release)
#   --no-build       skip cargo build; reuse the last build output
#   --version V      CFBundleShortVersionString (default: Cargo.toml's version)
#   --build N        CFBundleVersion (default: 1)
#
# Prerequisites, all from the repo's own builds:
#   qemu/install/ or qemu/install-macos-<arch>/       - qemu/build.sh
#     lib/libcdj3k-emu-qemu.dylib, bin/qemu-img
#   build/Image, build/docker-out/                    - build.sh
#   guest/out/                                        - build.sh
#
# The script:
#   1. Builds the app with cargo, once per arch
#   2. Stages each arch's cdj3k-emu, libcdj3k-emu-qemu.dylib and qemu-img,
#      with its Homebrew dylib graph beside them (@loader_path) so the .app
#      runs without Homebrew installed
#   3. Fills Contents/MacOS with the one slice, or both merged by lipo
#   4. Populates Contents/Resources: patch/ (with vanilla-modules/*.ko), tools/,
#      the kernel and the icon
#   5. Writes Info.plist
#   6. Builds the CoreMIDI driver plugin for the same arches
#
# The result is ad-hoc signed piecewise and not sealed; build.sh signs it.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

# ── Temp-file cleanup ────────────────────────────────────────────────────────
# `set -e` will bail us out on any failure (codesign, hdiutil, cargo, curl);
# without an EXIT trap, the entitlement plist and staging directories that
# we create with mktemp would leak into /tmp until the next reboot.  Each
# create step appends its path to TMP_CLEANUP and the trap nukes them all.
TMP_CLEANUP=()
cleanup_tmp() {
    # Bash gotcha: an EXIT trap's final command status overrides the script's
    # explicit `exit N`. If every entry has already been removed manually,
    # the `[[ -e ]] && rm` compound short-circuits to 1 and the whole script
    # would exit 1 despite `exit 0` at the bottom.  `return 0` pins it down.
    for p in "${TMP_CLEANUP[@]}"; do
        [[ -n "$p" && -e "$p" ]] && rm -rf "$p"
    done
    return 0
}
trap cleanup_tmp EXIT


# ── Options ──────────────────────────────────────────────────────────────────
PROFILE="release"
CARGO_PROFILE_FLAG="--release"
DO_BUILD=1
OUT_DIR="$REPO_ROOT/dist/macos"
ARCH=""
APP_VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$REPO_ROOT/Cargo.toml" | head -n1)"
APP_BUILD="1"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --arch)       ARCH="$2"; shift ;;
        --out)        OUT_DIR="$2"; shift ;;
        --debug)      PROFILE="debug"; CARGO_PROFILE_FLAG="" ;;
        --no-build)   DO_BUILD=0 ;;
        --version)    APP_VERSION="$2"; shift ;;
        --build)      APP_BUILD="$2"; shift ;;
        -h|--help)    sed -n '3,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *)            echo "ERROR: unknown argument: $1" >&2; exit 1 ;;
    esac
    shift
done
case "$ARCH" in
    arm64|x86_64|universal) ;;
    "") ;;
    *) echo "ERROR: --arch takes arm64, x86_64 or universal" >&2; exit 1 ;;
esac

# ── Paths ─────────────────────────────────────────────────────────────────────
APP_DIR="$OUT_DIR/CDJ3K Emulator.app"
MACOS_DIR="$APP_DIR/Contents/MacOS"
RESOURCES_DIR="$APP_DIR/Contents/Resources"

# A slice is one arch's cdj3k-emu, libcdj3k-emu-qemu.dylib and qemu-img;
# the bundle's Contents/MacOS is its single slice, or both merged by lipo.
# Without --arch it is the native build: qemu/install/ and cargo's default
# target.
NATIVE=0
case "$ARCH" in
    "")        NATIVE=1; ARCHS=("$(uname -m)") ;;
    universal) ARCHS=(arm64 x86_64) ;;
    *)         ARCHS=("$ARCH") ;;
esac
if [[ "$NATIVE" -eq 0 ]]; then
    export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-15.0}"
fi

rust_triple() {
    case "$1" in
        arm64)  echo aarch64-apple-darwin ;;
        x86_64) echo x86_64-apple-darwin ;;
    esac
}

qemu_dir() {
    if [[ "$NATIVE" -eq 0 ]]; then
        echo "$REPO_ROOT/qemu/install-macos-$1"
    else
        echo "$REPO_ROOT/qemu/install"
    fi
}

binary_path() {
    if [[ "$NATIVE" -eq 0 ]]; then
        echo "$REPO_ROOT/target/$(rust_triple "$1")/$PROFILE/cdj3k-emu"
    else
        echo "$REPO_ROOT/target/$PROFILE/cdj3k-emu"
    fi
}

# ── Build ─────────────────────────────────────────────────────────────────────
if [[ "$DO_BUILD" -eq 1 ]]; then
    "$REPO_ROOT/winit/fetch.sh"
    for arch in "${ARCHS[@]}"; do
        if [[ "$NATIVE" -eq 0 ]]; then
            triple=$(rust_triple "$arch")
            echo "==> cargo build $CARGO_PROFILE_FLAG -p cdj3k-emu --target $triple"
            # With MACOSX_DEPLOYMENT_TARGET set, a release proc-macro stripped
            # of its debuginfo fails to load ("can't find crate"); proc-macros
            # and build scripts stay unstripped.
            (cd "$REPO_ROOT" && CDJ3K_EMU_QEMU_LIB_DIR="$(qemu_dir "$arch")/lib" \
                CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP=false \
                cargo build $CARGO_PROFILE_FLAG -p cdj3k-emu --target "$triple")
        else
            echo "==> cargo build $CARGO_PROFILE_FLAG -p cdj3k-emu"
            (cd "$REPO_ROOT" && cargo build $CARGO_PROFILE_FLAG -p cdj3k-emu)
        fi
    done
fi

# ── Assemble bundle ───────────────────────────────────────────────────────────
echo "==> Assembling $APP_DIR"
rm -rf "$APP_DIR"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

# Each slice is staged and self-contained on its own: its Homebrew dylib graph
# (if any) is copied beside it and every load command rewritten to
# @loader_path, so the .app runs on a machine without Homebrew. The helper
# ad-hoc-signs what it rewrites; build.sh re-seals everything
# with the final identity.
SLICES=()
for arch in "${ARCHS[@]}"; do
    binary=$(binary_path "$arch")
    qemu=$(qemu_dir "$arch")
    for f in "$binary" "$qemu/lib/libcdj3k-emu-qemu.dylib" "$qemu/bin/qemu-img"; do
        if [[ ! -f "$f" ]]; then
            echo "ERROR: $arch: not found: $f" >&2
            if [[ "$f" == "$binary" ]]; then
                echo "       Run without --no-build" >&2
            elif [[ "$NATIVE" -eq 0 ]]; then
                echo "       Run: bash qemu/build.sh --macos $arch" >&2
            else
                echo "       Run: ./qemu/build.sh  (configured with --enable-tools)" >&2
            fi
            exit 1
        fi
    done
    slice=$(mktemp -d -t "cdj3k-slice-$arch")
    TMP_CLEANUP+=("$slice")
    cp "$binary" "$slice/cdj3k-emu"
    cp "$qemu/lib/libcdj3k-emu-qemu.dylib" "$slice/"
    # The firmware wizard provisions each slot's eMMC with qemu-img convert;
    # a Finder-launched .app has no useful $PATH.
    cp "$qemu/bin/qemu-img" "$slice/"
    echo "==> Bundling the $arch slice's dylibs (self-contained)"
    "$REPO_ROOT/scripts/bundle-dylibs.sh" "$slice" \
        libcdj3k-emu-qemu.dylib qemu-img cdj3k-emu
    SLICES+=("$slice")
done

if [[ ${#SLICES[@]} -eq 1 ]]; then
    cp "${SLICES[0]}"/* "$MACOS_DIR/"
else
    # A file in both slices becomes one universal file; a dylib only one
    # slice links stays thin, since only that arch loads it.
    for name in $(for s in "${SLICES[@]}"; do ls "$s"; done | sort -u); do
        parts=()
        for s in "${SLICES[@]}"; do
            [[ -f "$s/$name" ]] && parts+=("$s/$name")
        done
        if [[ ${#parts[@]} -gt 1 ]]; then
            lipo -create "${parts[@]}" -output "$MACOS_DIR/$name"
        else
            cp "${parts[0]}" "$MACOS_DIR/$name"
        fi
    done
    echo "     merged ${ARCHS[*]}: $(lipo -archs "$MACOS_DIR/cdj3k-emu")"
fi

# Nothing in Contents/MacOS may still name a path outside the bundle or the
# OS: a leftover /opt/homebrew reference is a crash on a clean machine.
# (`grep -v` exits 1 when nothing is stray, which `set -e -o pipefail` would
# otherwise turn into a silent exit of the whole script.)
STRAY=$(for f in "$MACOS_DIR"/*; do
            for arch in $(lipo -archs "$f"); do
                otool -arch "$arch" -L "$f" 2>/dev/null | tail -n +2 | awk '{print $1}' \
                  | { grep -v '^/usr/lib/\|^/System/\|^@loader_path/\|^@rpath/\|^@executable_path/' || true; } \
                  | sed "s|^|$(basename "$f") ($arch): |"
            done
        done)
if [[ -n "$STRAY" ]]; then
    echo "ERROR: unbundled dylib references remain:" >&2
    echo "$STRAY" >&2
    exit 1
fi

# ── Resources: patch scripts, guest modules and tools, PPM assets ────────────
#
# Prerequisites: build.sh must have been run first so that:
#   build/docker-out/modules/*.ko                  - guest kernel modules
#   tools/*_aarch64, guest/out/deck_shim.so        - pre-built guest tools
echo "==> Assembling Contents/Resources"

RES_DIR="$RESOURCES_DIR"

# patch/   - single merged dispatcher + per-step assets
#
# The repo keeps initramfs-patch/patch-rootfs.d/ as ~28 numbered scripts for
# debuggability; the bundle ships ONE concatenated patch-rootfs.sh so the
# .app contains a single file instead of the directory tree.
RES_PATCH="$RES_DIR/patch"
"$REPO_ROOT/scripts/make-patch-dispatcher.sh" \
    "$REPO_ROOT/initramfs-patch/patch-rootfs.d" "$RES_PATCH/patch-rootfs.sh"

# cfgd_aarch64 is required: 21-cfgd.sh aborts the whole rootfs patch without
# it.  Checked here because that abort happens at *provision* time, long after
# a bundle that skipped the file silently looked like it built cleanly.
if [[ -f "$REPO_ROOT/guest/out/cfgd_aarch64" ]]; then
    cp "$REPO_ROOT/guest/out/cfgd_aarch64" "$RES_PATCH/"
else
    echo "ERROR: guest/out/cfgd_aarch64 not found" >&2
    echo "       Run: ./build.sh" >&2
    exit 1
fi

# 29-pc-link-bridge.sh installs pc_link_bridge_aarch64 into the rootfs.  It
# reads PATCH_ASSETS_DIR, falling back to guest/out/, which exists only in a
# dev checkout.
if [[ -f "$REPO_ROOT/guest/out/pc_link_bridge_aarch64" ]]; then
    cp "$REPO_ROOT/guest/out/pc_link_bridge_aarch64" "$RES_PATCH/"
else
    echo "ERROR: guest/out/pc_link_bridge_aarch64 not found" >&2
    echo "       Run: ./build.sh" >&2
    exit 1
fi

# 02-dropbear-install.sh installs dropbear_aarch64 into the rootfs of
# firmware that ships no SSH server.
if [[ -f "$REPO_ROOT/guest/out/dropbear_aarch64" ]]; then
    cp "$REPO_ROOT/guest/out/dropbear_aarch64" "$REPO_ROOT/guest/out/dropbear_aarch64.LICENSE" \
        "$RES_PATCH/"
else
    echo "ERROR: guest/out/dropbear_aarch64 not found" >&2
    echo "       Run: ./build.sh" >&2
    exit 1
fi

# patch/vanilla-modules/  - 6.6 out-of-tree modules for 22-vanilla-kernel-fixups.sh
MODS_SRC="$REPO_ROOT/build/docker-out/modules"
if [[ -d "$MODS_SRC" ]] && compgen -G "$MODS_SRC/*.ko" > /dev/null; then
    mkdir -p "$RES_PATCH/vanilla-modules"
    cp "$MODS_SRC"/*.ko "$RES_PATCH/vanilla-modules/"
    echo "     bundled modules: $(ls "$RES_PATCH/vanilla-modules"/*.ko | xargs -n1 basename | tr '\n' ' ')"
else
    echo "WARNING: build/docker-out/modules not found - run ./build.sh first"
fi

# patch/dummy_drv.so  - Xorg dummy video driver (ABI 24.0) for headless mode
DUMMY_DRV_SRC="$REPO_ROOT/build/docker-out/dummy_drv.so"
if [[ -f "$DUMMY_DRV_SRC" ]]; then
    cp "$DUMMY_DRV_SRC" "$RES_PATCH/dummy_drv.so"
    echo "     bundled dummy_drv.so"
else
    echo "WARNING: build/docker-out/dummy_drv.so not found - run ./build.sh first"
fi

# tools/   - aarch64 guest ELFs + deck_shim.so.  The firmware provisioner
# installs everything here into the rootfs's /usr/bin (deck_shim.so goes to
# /home/root).
RES_TOOLS="$RES_DIR/tools"
mkdir -p "$RES_TOOLS"
for tool in subucom_live subucom_forwarder; do
    src="$REPO_ROOT/guest/out/${tool}_aarch64"
    if [[ -f "$src" ]]; then
        cp "$src" "$RES_TOOLS/$tool"
        chmod +x "$RES_TOOLS/$tool"
        echo "     bundled $tool"
    else
        echo "WARNING: guest tool not found: $src  (run: ./build.sh --modules-only)"
    fi
done
if [[ -f "$REPO_ROOT/guest/out/deck_shim.so" ]]; then
    cp "$REPO_ROOT/guest/out/deck_shim.so" "$RES_TOOLS/deck_shim.so"
    echo "     bundled deck_shim.so"
else
    echo "WARNING: guest/out/deck_shim.so not found - run: make -C guest"
fi


# App icon - .icns expected at app/cdj3k-emu/assets/cdj3k-emu.icns
ICNS_SRC="$REPO_ROOT/app/cdj3k-emu/assets/cdj3k-emu.icns"
if [[ -f "$ICNS_SRC" ]]; then
    cp "$ICNS_SRC" "$RES_DIR/cdj3k-emu.icns"
    echo "     bundled cdj3k-emu.icns"
else
    echo "WARNING: cdj3k-emu.icns not found at $ICNS_SRC - bundle will use the generic app icon"
fi

# Image - aarch64 Linux 6.6 LTS kernel (required by firmware wizard)
KERNEL="$REPO_ROOT/build/Image"
if [[ -f "$KERNEL" ]]; then
    cp "$KERNEL" "$RES_DIR/Image"
    echo "     bundled Image"
else
    echo "ERROR: Image not found at $KERNEL"
    echo "       Build it first: ./build.sh"
    exit 1
fi

# The updater replaces a release bundle from the release's .dmg. A dev build
# carries no marker and is not updated.
if [[ $NATIVE -eq 0 ]]; then
    echo dmg > "$RES_DIR/package-kind"
fi

# ── Sparkle ──────────────────────────────────────────────────────────────────
# The macOS updater. The app loads it at runtime from Contents/Frameworks, and
# only in a release bundle (package-kind). Its XPC services, for sandboxed
# apps, are removed.
SPARKLE_VERSION="2.10.0"
SPARKLE_SHA256="c2bf58aa8387266ac179357b1415d6f2635f044da8be41042af32425dae6da0c"
SPARKLE_DIR="$REPO_ROOT/build/sparkle-$SPARKLE_VERSION"
if [[ ! -d "$SPARKLE_DIR/Sparkle.framework" ]]; then
    echo "==> Fetching Sparkle $SPARKLE_VERSION"
    mkdir -p "$SPARKLE_DIR"
    archive="$SPARKLE_DIR/Sparkle.tar.xz"
    curl -fsSL -o "$archive" \
        "https://github.com/sparkle-project/Sparkle/releases/download/$SPARKLE_VERSION/Sparkle-$SPARKLE_VERSION.tar.xz"
    got=$(shasum -a 256 "$archive" | cut -d' ' -f1)
    if [[ "$got" != "$SPARKLE_SHA256" ]]; then
        echo "ERROR: Sparkle-$SPARKLE_VERSION.tar.xz sha256 $got" >&2
        rm -rf "$SPARKLE_DIR"
        exit 1
    fi
    tar -xf "$archive" -C "$SPARKLE_DIR" Sparkle.framework
    rm "$archive"
fi
mkdir -p "$APP_DIR/Contents/Frameworks"
rm -rf "$APP_DIR/Contents/Frameworks/Sparkle.framework"
ditto "$SPARKLE_DIR/Sparkle.framework" "$APP_DIR/Contents/Frameworks/Sparkle.framework"
rm -rf "$APP_DIR/Contents/Frameworks/Sparkle.framework/Versions/B/XPCServices" \
    "$APP_DIR/Contents/Frameworks/Sparkle.framework/XPCServices"
echo "     bundled Sparkle.framework $SPARKLE_VERSION"
SPARKLE_PUBLIC_KEY=$(tr -d '[:space:]' < "$REPO_ROOT/packaging/update-ed25519.pub")

# ── Info.plist ────────────────────────────────────────────────────────────────
echo "==> Writing Info.plist (version=$APP_VERSION build=$APP_BUILD)"
cat > "$APP_DIR/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
    "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>com.cdj3k.emu</string>

    <key>CFBundleName</key>
    <string>CDJ3K Emulator</string>

    <key>CFBundleDisplayName</key>
    <string>CDJ3K Emulator</string>

    <key>CFBundleExecutable</key>
    <string>cdj3k-emu</string>

    <key>CFBundleIconFile</key>
    <string>cdj3k-emu</string>

    <key>CFBundlePackageType</key>
    <string>APPL</string>

    <key>CFBundleVersion</key>
    <string>${APP_BUILD}</string>

    <key>CFBundleShortVersionString</key>
    <string>${APP_VERSION}</string>

    <key>LSMinimumSystemVersion</key>
    <string>15.0</string>

    <key>NSHighResolutionCapable</key>
    <true/>

    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>

    <!-- Sparkle. The feed URL comes from the app (cdj3k-emu-update); a
         release's CFBundleVersion is its CD run number, which is what the
         appcast's sparkle:version compares against. -->
    <key>SUPublicEDKey</key>
    <string>${SPARKLE_PUBLIC_KEY}</string>
    <!-- Sparkle's name for the app: the process renames itself per slot. -->
    <key>SUBundleName</key>
    <string>CDJ3K Emulator</string>
    <key>SUEnableAutomaticChecks</key>
    <true/>
    <key>SUScheduledCheckInterval</key>
    <integer>21600</integer>
    <key>SUAllowsAutomaticUpdates</key>
    <true/>
    <key>SUVerifyUpdateBeforeExtraction</key>
    <true/>

    <!-- Required for HVF (Hypervisor.framework) entitlement. -->
    <!-- Sign with a Developer ID cert for distribution; ad-hoc for local use. -->
    <key>com.apple.security.hypervisor</key>
    <true/>

    <!-- Entitlements that matter live in the code signature, not here; this
         key is informational.  The restricted ones (vmnet, virtual HID) come
         from Contents/embedded.provisionprofile - see build.sh.
         Pro DJ Link networking is QEMU's own -netdev vmnet-bridged /
         vmnet-host, which vmnet authorises through that entitlement. -->
</dict>
</plist>
PLIST

# ── CoreMIDI driver plugin ───────────────────────────────────────────────────
# Host bundle (not a cargo/qemu artifact), so build it here regardless of
# --no-build.  Shipped in Resources; pc_link::midi_driver::ensure_driver_installed
# copies it into ~/Library/Audio/MIDI Drivers on the first PC Link toggle.
# --deep does not descend into Resources, so build.sh signs it explicitly
# before sealing the app.
echo "==> Building CoreMIDI driver plugin"
make -C "$REPO_ROOT/tools/midi-driver" clean >/dev/null 2>&1 || true
midi_archflags=""
for arch in "${ARCHS[@]}"; do midi_archflags+=" -arch $arch"; done
make -C "$REPO_ROOT/tools/midi-driver" ARCHFLAGS="$midi_archflags"
cp -R "$REPO_ROOT/tools/midi-driver/CDJ3KEmuMIDI.plugin" "$RESOURCES_DIR/"
echo "     bundled CDJ3KEmuMIDI.plugin"


echo ""
echo "Staged: $APP_DIR ($(lipo -archs "$MACOS_DIR/cdj3k-emu"))"
echo "        sign and package it with packaging/macos/build.sh"
exit 0
