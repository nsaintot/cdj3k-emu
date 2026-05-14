#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# bundle.sh - build and package cdj3k-emu as a macOS .app bundle.
#
# Usage:
#   ./bundle.sh [--debug] [--no-build] [--out DIR] [--sign IDENTITY] [--dmg]
#               [--version VERSION] [--build BUILD]
#
#   --debug          build debug profile (default: release)
#   --no-build       skip cargo build; reuse last build output
#   --out DIR        output directory for cdj3k-emu.app (default: ./dist)
#   --sign IDENTITY  codesign identity string (overrides CODESIGN_IDENTITY env var)
#                    Use "Apple Development" to pick your only dev cert automatically.
#                    Falls back to ad-hoc (-) when omitted - TCC/FDA will not work.
#   --dmg            after bundling, package the .app into a compressed .dmg
#                    image alongside it (matches CFBundleShortVersionString).
#   --version VER    CFBundleShortVersionString to embed (default: 0.1.0).
#                    Also names the DMG: CDJ3K-Emulator-<VER>.dmg.
#   --build N        CFBundleVersion build number (default: 1).
#
# Prerequisites:
#   - qemu/install/lib/libcdj3k-emu-qemu.dylib  (from qemu/build.sh)
#   - qemu/install/bin/qemu-img             (from qemu/build.sh)
#   - build/initramfs-work/rootfs/lib/modules/*.ko  (from build.sh)
#   - tools/*_aarch64, guest/out/ep122_shim.so   (pre-built guest tools)
#
# The script:
#   1. Builds tools/cdj3k-emu with cargo
#   2. Creates cdj3k-emu.app/Contents/{MacOS,Resources}
#   3. Copies cdj3k-emu, libcdj3k-emu-qemu.dylib, qemu-img, socket_vmnet into Contents/MacOS
#   4. Populates Contents/Resources: modules/*.ko, patch/, tools/, assets/
#   5. Writes Info.plist
#   6. Codesigns the bundle (real identity when provided, ad-hoc otherwise)

set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")" && pwd)"

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
OUT_DIR="$REPO_ROOT/dist"
# Real identity (e.g. "Apple Development") enables TCC/FDA tracking.
# Falls back to ad-hoc ("-") when not set.
SIGN_IDENTITY="${CODESIGN_IDENTITY:-}"
MAKE_DMG=0
APP_VERSION="0.1.0"
APP_BUILD="1"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --debug)      PROFILE="debug"; CARGO_PROFILE_FLAG="" ;;
        --no-build)   DO_BUILD=0 ;;
        --out=*)      OUT_DIR="${1#--out=}" ;;
        --out)        OUT_DIR="$2"; shift ;;
        --sign=*)     SIGN_IDENTITY="${1#--sign=}" ;;
        --sign)       SIGN_IDENTITY="$2"; shift ;;
        --dmg)        MAKE_DMG=1 ;;
        --version=*)  APP_VERSION="${1#--version=}" ;;
        --version)    APP_VERSION="$2"; shift ;;
        --build=*)    APP_BUILD="${1#--build=}" ;;
        --build)      APP_BUILD="$2"; shift ;;
    esac
    shift
done

# ── Paths ─────────────────────────────────────────────────────────────────────
BINARY="$REPO_ROOT/target/$PROFILE/cdj3k-emu"
DYLIB="$REPO_ROOT/qemu/install/lib/libcdj3k-emu-qemu.dylib"
QEMU_IMG="$REPO_ROOT/qemu/install/bin/qemu-img"
APP_DIR="$OUT_DIR/CDJ3K Emulator.app"
MACOS_DIR="$APP_DIR/Contents/MacOS"
RESOURCES_DIR="$APP_DIR/Contents/Resources"

SOCKET_VMNET_VERSION="1.2.2"
SOCKET_VMNET_URL="https://github.com/lima-vm/socket_vmnet/releases/download/v${SOCKET_VMNET_VERSION}/socket_vmnet-${SOCKET_VMNET_VERSION}-arm64.tar.gz"
# SHA-256 of the upstream arm64 release tarball.  Verified by running:
#   shasum -a 256 socket_vmnet-1.2.2-arm64.tar.gz
# against the asset linked from the v1.2.2 GitHub release notes.
SOCKET_VMNET_SHA256="c7bf62308fbcfdc29bdfb8373c9b1951f7ac2396446e4390919796a94972e6dc"
# Cached archive lives under build/ (gitignored).  Re-used across bundle runs
# so a clean build doesn't re-download the same tarball; the SHA-256 check
# below guards against a corrupted or tampered cache.
SOCKET_VMNET_CACHE_DIR="$REPO_ROOT/build/cache"
SOCKET_VMNET_CACHE_FILE="$SOCKET_VMNET_CACHE_DIR/socket_vmnet-${SOCKET_VMNET_VERSION}-arm64.tar.gz"

# ── Build ─────────────────────────────────────────────────────────────────────
if [[ "$DO_BUILD" -eq 1 ]]; then
    echo "==> cargo build $CARGO_PROFILE_FLAG -p cdj3k-emu"
    (cd "$REPO_ROOT" && cargo build $CARGO_PROFILE_FLAG -p cdj3k-emu)
fi

if [[ ! -f "$BINARY" ]]; then
    echo "ERROR: binary not found: $BINARY"
    exit 1
fi
if [[ ! -f "$DYLIB" ]]; then
    echo "ERROR: libcdj3k-emu-qemu.dylib not found: $DYLIB"
    echo "       Run: ./qemu/build.sh"
    exit 1
fi

# ── Assemble bundle ───────────────────────────────────────────────────────────
echo "==> Assembling $APP_DIR"
rm -rf "$APP_DIR"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

cp "$BINARY"   "$MACOS_DIR/cdj3k-emu"
cp "$DYLIB"    "$MACOS_DIR/libcdj3k-emu-qemu.dylib"

if [[ ! -f "$QEMU_IMG" ]]; then
    echo "ERROR: qemu-img not found at $QEMU_IMG"
    echo "       The firmware wizard provisions each slot's eMMC by calling"
    echo "       qemu-img convert; a Finder-launched .app has no useful \$PATH"
    echo "       and will fail at the [emmc] step without a bundled copy."
    echo "       Rebuild QEMU: ./qemu/build.sh  (configured with --enable-tools)."
    exit 1
fi
cp "$QEMU_IMG" "$MACOS_DIR/qemu-img"
echo "     bundled qemu-img"

echo "==> Fetching socket_vmnet ${SOCKET_VMNET_VERSION}"
mkdir -p "$SOCKET_VMNET_CACHE_DIR"
verify_socket_vmnet_sha() {
    local f="$1"
    local actual
    actual=$(shasum -a 256 "$f" | awk '{print $1}')
    [[ "$actual" == "$SOCKET_VMNET_SHA256" ]]
}
if [[ -f "$SOCKET_VMNET_CACHE_FILE" ]] && verify_socket_vmnet_sha "$SOCKET_VMNET_CACHE_FILE"; then
    echo "     reusing cached archive: $SOCKET_VMNET_CACHE_FILE"
else
    if [[ -f "$SOCKET_VMNET_CACHE_FILE" ]]; then
        echo "     cached archive failed SHA-256 verification; re-downloading"
        rm -f "$SOCKET_VMNET_CACHE_FILE"
    fi
    curl -fsSL "$SOCKET_VMNET_URL" -o "$SOCKET_VMNET_CACHE_FILE"
    if ! verify_socket_vmnet_sha "$SOCKET_VMNET_CACHE_FILE"; then
        echo "ERROR: socket_vmnet archive SHA-256 mismatch"
        echo "       expected: $SOCKET_VMNET_SHA256"
        echo "       got:      $(shasum -a 256 "$SOCKET_VMNET_CACHE_FILE" | awk '{print $1}')"
        rm -f "$SOCKET_VMNET_CACHE_FILE"
        exit 1
    fi
fi
SOCKET_VMNET_TMP=$(mktemp -d)
TMP_CLEANUP+=("$SOCKET_VMNET_TMP")
tar -xz -f "$SOCKET_VMNET_CACHE_FILE" -C "$SOCKET_VMNET_TMP"
SOCKET_VMNET_BIN=$(find "$SOCKET_VMNET_TMP" -name "socket_vmnet" -type f | head -1)
if [[ -z "$SOCKET_VMNET_BIN" ]]; then
    echo "ERROR: socket_vmnet binary not found in release tarball"
    rm -rf "$SOCKET_VMNET_TMP"
    exit 1
fi
cp "$SOCKET_VMNET_BIN" "$MACOS_DIR/socket_vmnet"
chmod +x "$MACOS_DIR/socket_vmnet"
rm -rf "$SOCKET_VMNET_TMP"
echo "     bundled socket_vmnet"

# ── Resources: modules, patch scripts, guest tools, PPM assets ───────────────
#
# Prerequisites: build.sh must have been run first so that:
#   build/initramfs-work/rootfs/lib/modules/*.ko  - pre-built kernel modules
#   tools/*_aarch64, guest/out/ep122_shim.so            - pre-built guest tools
echo "==> Assembling Contents/Resources"

ROOTFS_MODULES="$REPO_ROOT/build/initramfs-work/rootfs/lib/modules"
RES_DIR="$RESOURCES_DIR"

# guest/modules/
RES_MODULES="$RES_DIR/modules"
mkdir -p "$RES_MODULES"
if [[ -d "$ROOTFS_MODULES" ]] && compgen -G "$ROOTFS_MODULES/*.ko" > /dev/null; then
    cp "$ROOTFS_MODULES"/*.ko "$RES_MODULES/"
    echo "     bundled $(ls "$RES_MODULES"/*.ko | wc -l | tr -d ' ') .ko files"
else
    echo "WARNING: no .ko files found at $ROOTFS_MODULES"
    echo "         Run ./build.sh before bundling"
fi

# patch/   - single merged dispatcher + per-step assets
#
# The repo keeps initramfs-patch/patch-rootfs.d/ as ~28 numbered scripts for
# debuggability; the bundle ships ONE concatenated patch-rootfs.sh so the
# .app contains a single file instead of the directory tree.
RES_PATCH="$RES_DIR/patch"
mkdir -p "$RES_PATCH"
PATCH_SRC_DIR="$REPO_ROOT/initramfs-patch"
PATCH_STEPS=("$PATCH_SRC_DIR"/patch-rootfs.d/[0-9]*.sh)
{
    cat <<'HDR'
#!/usr/bin/env bash
# patch-rootfs.sh - auto-generated bundle dispatcher.
# Concatenation of every initramfs-patch/patch-rootfs.d/*.sh in numeric order.
# Source: kept modular in the repo at initramfs-patch/patch-rootfs.d/.
set -euo pipefail
ROOTFS="${1:?Usage: $0 <initramfs-root>}"
export ROOTFS
export PATCH_ASSETS_DIR="$(cd "$(dirname "$0")" && pwd)"
# SSH is off by default in shipped builds (no passwordless root in the wild),
# but respect an explicit ENABLE_SSH=1 from the caller's environment so a
# developer can `ENABLE_SSH=1 open dist/CDJ3K\ Emulator.app` (or launch via
# the CLI binary directly) without rebuilding.
export ENABLE_SSH="${ENABLE_SSH:-0}"
echo "=== Patching initramfs rootfs at: $ROOTFS ==="
HDR
    for step in "${PATCH_STEPS[@]}"; do
        name=$(basename "$step")
        printf '\necho "--- %s ---"\n(\n' "$name"
        # Strip per-step shebang and `set -euo pipefail` (already set above).
        sed -E '1{/^#!/d;}; /^set -euo pipefail$/d' "$step"
        printf ')\n'
    done
    printf '\necho "=== All patches applied ==="\n'
} > "$RES_PATCH/patch-rootfs.sh"
chmod +x "$RES_PATCH/patch-rootfs.sh"

[[ -f "$REPO_ROOT/guest/out/cfgd_aarch64" ]] && \
    cp "$REPO_ROOT/guest/out/cfgd_aarch64" "$RES_PATCH/"
echo "     bundled merged patch-rootfs.sh (${#PATCH_STEPS[@]} steps inlined)"

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

# tools/   - aarch64 guest ELFs + ep122_shim.so
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
if [[ -f "$REPO_ROOT/guest/out/ep122_shim.so" ]]; then
    cp "$REPO_ROOT/guest/out/ep122_shim.so" "$RES_TOOLS/ep122_shim.so"
    echo "     bundled ep122_shim.so"
else
    echo "WARNING: guest/out/ep122_shim.so not found - run: make -C guest"
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
    <string>13.0</string>

    <key>NSHighResolutionCapable</key>
    <true/>

    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>

    <!-- Required for HVF (Hypervisor.framework) entitlement. -->
    <!-- Sign with a Developer ID cert for distribution; ad-hoc for local use. -->
    <key>com.apple.security.hypervisor</key>
    <true/>

    <!-- NOTE: com.apple.vm.networking is intentionally absent.
         Bridged Pro DJ Link is handled by the bundled socket_vmnet helper
         (github.com/lima-vm/socket_vmnet), which runs as root via a native
         macOS admin password dialog.  No vm.networking entitlement needed. -->
</dict>
</plist>
PLIST

# ── Codesign ─────────────────────────────────────────────────────────────────
# cdj3k-emu calls Hypervisor.framework via libcdj3k-emu-qemu.dylib - the entitlement
# must be on the process binary (cdj3k-emu), not the dylib.
#
# With a real identity (Apple Development / Developer ID):
#   --options runtime enables the hardened runtime required for notarization
#   and is also what allows TCC (Full Disk Access) to track the app by its
#   bundle ID so it appears in System Settings → Privacy & Security → FDA.
#
# With ad-hoc (-): HVF works locally but TCC cannot identify the app -
#   it will never appear in the FDA list and physical USB passthrough
#   will be denied by macOS even if the user is in the operator group.

HVF_ENT=$(mktemp -t hvf-entitlement)
TMP_CLEANUP+=("$HVF_ENT")
cat > "$HVF_ENT" <<'ENT'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
    "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.hypervisor</key>
    <true/>
</dict>
</plist>
ENT

if [[ -n "$SIGN_IDENTITY" ]]; then
    echo "==> Codesigning bundle (identity: $SIGN_IDENTITY)"
    # Deep-sign all nested binaries first (no entitlements on helpers/dylibs).
    codesign --force --deep --options runtime --sign "$SIGN_IDENTITY" "$APP_DIR"
    # Re-sign the main binary with HVF entitlement - --deep would have stripped it.
    codesign --force --options runtime --sign "$SIGN_IDENTITY" \
        --entitlements "$HVF_ENT" "$MACOS_DIR/cdj3k-emu"
    codesign --verify --deep --strict "$APP_DIR"
    echo "     signed with: $SIGN_IDENTITY"
    echo "     TCC/FDA: grant Full Disk Access to CDJ3K Emulator in"
    echo "              System Settings → Privacy & Security → Full Disk Access"
else
    echo "==> Codesigning bundle (ad-hoc - TCC/FDA will not work)"
    echo "     Pass --sign \"Apple Development\" or set CODESIGN_IDENTITY to enable FDA."
    codesign --force --deep --sign - "$APP_DIR"
    # Re-sign CDJ3K Emulator with HVF entitlement AFTER --deep (deep would strip it).
    codesign --force --sign - --entitlements "$HVF_ENT" "$MACOS_DIR/cdj3k-emu"
fi

rm "$HVF_ENT"

# ── DMG packaging (optional) ─────────────────────────────────────────────────
if [[ "$MAKE_DMG" -eq 1 ]]; then
    # Extract CFBundleShortVersionString so the DMG file matches the bundle's
    # advertised version - keeps GitHub release asset names self-consistent.
    VERSION=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" \
        "$APP_DIR/Contents/Info.plist" 2>/dev/null || echo "0.0.0")
    DMG_PATH="$OUT_DIR/CDJ3K-Emulator-${VERSION}.dmg"
    DMG_STAGING=$(mktemp -d)
    TMP_CLEANUP+=("$DMG_STAGING")

    echo "==> Creating DMG: $DMG_PATH"
    cp -R "$APP_DIR" "$DMG_STAGING/"
    # Convenience symlink so drag-to-install works without the user navigating
    # to /Applications by hand.
    ln -s /Applications "$DMG_STAGING/Applications"

    rm -f "$DMG_PATH"
    hdiutil create \
        -volname "CDJ3K Emulator ${VERSION}" \
        -srcfolder "$DMG_STAGING" \
        -ov \
        -format UDZO \
        -fs HFS+ \
        "$DMG_PATH" >/dev/null

    rm -rf "$DMG_STAGING"
    echo "     wrote $(du -h "$DMG_PATH" | cut -f1) DMG"
fi

echo ""
echo "Done: $APP_DIR"
if [[ "$MAKE_DMG" -eq 1 ]]; then
    echo "      $DMG_PATH"
fi
echo ""
echo "Arguments (override):"
echo "cdj3k-emu --kernel build/Image --initramfs build/initramfs-patched.cpio.gz"
exit 0