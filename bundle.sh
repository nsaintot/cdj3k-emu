#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# bundle.sh - build and package cdj3k-emu as a macOS .app bundle.
#
# Usage:
#   ./bundle.sh [--debug] [--no-build] [--universal] [--out DIR] [--sign IDENTITY] [--dmg]
#               [--version VERSION] [--build BUILD]
#               [--notarize] [--notary-profile NAME] [--profile PATH]
#
#   --debug          build debug profile (default: release)
#   --no-build       skip cargo build; reuse last build output
#   --universal      arm64 + x86_64 in one .app (an Intel Mac runs the guest
#                    under TCG).  Each slice links the QEMU of
#                    `qemu/build.sh --macos <arch>` (qemu/install-macos-<arch>/)
#                    and builds for the macOS 15.0 deployment target.
#                    Without it the .app is this Mac's arch, from qemu/install/.
#   --out DIR        output directory for cdj3k-emu.app (default: ./dist)
#   --sign IDENTITY  codesign identity string (overrides CODESIGN_IDENTITY env var)
#                    Use "Apple Development" to pick your only dev cert automatically.
#                    Falls back to ad-hoc (-) when omitted - TCC/FDA will not work.
#   --profile PATH   Apple provisioning profile authorising the restricted
#                    entitlements (vmnet, virtual HID).  Default:
#                    ./cdj3k-emu.provisionprofile; PROVISION_PROFILE env var
#                    overrides.  Copied to Contents/embedded.provisionprofile and
#                    its granted entitlements are added to the signature.  Without
#                    it the bundle signs with the free entitlements only.
#   --dmg            after bundling, package the .app into a compressed .dmg
#                    image alongside it (matches CFBundleShortVersionString).
#   --version VER    CFBundleShortVersionString to embed (default: 0.2.0).
#                    Also names the DMG: CDJ3K-Emulator-<VER>.dmg.
#   --build N        CFBundleVersion build number (default: 1).
#   --notarize       after signing with a "Developer ID Application" identity,
#                    submit the .app (and the .dmg, with --dmg) to Apple's notary
#                    service, wait for the verdict and staple the tickets.  Needs
#                    credentials stored once with
#                      xcrun notarytool store-credentials <NAME> \
#                          --apple-id <APPLE_ID> --team-id <TEAM_ID>
#                    (prompts for an app-specific password from appleid.apple.com).
#   --notary-profile NAME
#                    keychain profile for --notarize (default: cdj3k-emu-notarization;
#                    NOTARY_PROFILE env var overrides).
#
# Prerequisites:
#   - qemu/install/lib/libcdj3k-emu-qemu.dylib  (from qemu/build.sh), or
#     qemu/install-macos-{arm64,x86_64}/        (from qemu/build.sh --macos <arch>)
#     with --universal
#   - qemu/install/bin/qemu-img             (likewise)
#   - build/docker-out/modules/*.ko  (from build.sh)
#   - guest/out/*_aarch64, guest/out/deck_shim.so  (from build.sh)
#
# The script:
#   1. Builds tools/cdj3k-emu with cargo, once per arch
#   2. Stages each arch's cdj3k-emu, libcdj3k-emu-qemu.dylib and qemu-img,
#      with its Homebrew dylib graph beside them (@loader_path) so the .app
#      runs without Homebrew installed
#   3. Creates cdj3k-emu.app/Contents/{MacOS,Resources}; Contents/MacOS is the
#      one slice, or both merged by lipo
#   4. Populates Contents/Resources: patch/ (with vanilla-modules/*.ko), tools/, assets/
#   5. Writes Info.plist
#   6. Builds the CoreMIDI driver plugin for the same arches
#   7. Codesigns the bundle (real identity when provided, ad-hoc otherwise)
#   8. Optionally notarizes + staples the .app and the .dmg

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
APP_VERSION="0.2.0"
APP_BUILD="1"
NOTARIZE=0
UNIVERSAL=0
NOTARY_PROFILE="${NOTARY_PROFILE:-cdj3k-emu-notarization}"
PROVISION_PROFILE="${PROVISION_PROFILE:-$REPO_ROOT/cdj3k-emu.provisionprofile}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --debug)      PROFILE="debug"; CARGO_PROFILE_FLAG="" ;;
        --no-build)   DO_BUILD=0 ;;
        --universal)  UNIVERSAL=1 ;;
        --out=*)      OUT_DIR="${1#--out=}" ;;
        --out)        OUT_DIR="$2"; shift ;;
        --sign=*)     SIGN_IDENTITY="${1#--sign=}" ;;
        --sign)       SIGN_IDENTITY="$2"; shift ;;
        --dmg)        MAKE_DMG=1 ;;
        --version=*)  APP_VERSION="${1#--version=}" ;;
        --version)    APP_VERSION="$2"; shift ;;
        --build=*)    APP_BUILD="${1#--build=}" ;;
        --build)      APP_BUILD="$2"; shift ;;
        --notarize)   NOTARIZE=1 ;;
        --notary-profile=*) NOTARY_PROFILE="${1#--notary-profile=}" ;;
        --notary-profile)   NOTARY_PROFILE="$2"; shift ;;
        --profile=*)  PROVISION_PROFILE="${1#--profile=}" ;;
        --profile)    PROVISION_PROFILE="$2"; shift ;;
        -h|--help)    sed -n '3,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *)            echo "ERROR: unknown argument: $1" >&2; exit 1 ;;
    esac
    shift
done

# Notarization is an Apple-side check of a Developer ID signature: it is
# meaningless (and refused by notarytool) for ad-hoc or Apple Development
# signatures, so fail here rather than after the whole bundle is assembled.
if [[ "$NOTARIZE" -eq 1 ]]; then
    if [[ "$SIGN_IDENTITY" != *"Developer ID Application"* ]]; then
        echo "ERROR: --notarize needs --sign \"Developer ID Application: ...\"" >&2
        echo "       (got: '${SIGN_IDENTITY:-ad-hoc}')" >&2
        exit 1
    fi
    if ! xcrun notarytool history --keychain-profile "$NOTARY_PROFILE" >/dev/null 2>&1; then
        echo "ERROR: no notarytool credentials under keychain profile '$NOTARY_PROFILE'." >&2
        echo "       Store them once (needs an app-specific password from appleid.apple.com):" >&2
        echo "         xcrun notarytool store-credentials $NOTARY_PROFILE \\" >&2
        echo "             --apple-id <APPLE_ID> --team-id <TEAM_ID>" >&2
        exit 1
    fi
fi

# ── Paths ─────────────────────────────────────────────────────────────────────
APP_DIR="$OUT_DIR/CDJ3K Emulator.app"
MACOS_DIR="$APP_DIR/Contents/MacOS"
RESOURCES_DIR="$APP_DIR/Contents/Resources"

# A slice is one arch's cdj3k-emu, libcdj3k-emu-qemu.dylib and qemu-img;
# the bundle's Contents/MacOS is its single slice, or both merged by lipo.
if [[ "$UNIVERSAL" -eq 1 ]]; then
    ARCHS=(arm64 x86_64)
    export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-15.0}"
else
    ARCHS=("$(uname -m)")
fi

rust_triple() {
    case "$1" in
        arm64)  echo aarch64-apple-darwin ;;
        x86_64) echo x86_64-apple-darwin ;;
    esac
}

qemu_dir() {
    if [[ "$UNIVERSAL" -eq 1 ]]; then
        echo "$REPO_ROOT/qemu/install-macos-$1"
    else
        echo "$REPO_ROOT/qemu/install"
    fi
}

binary_path() {
    if [[ "$UNIVERSAL" -eq 1 ]]; then
        echo "$REPO_ROOT/target/$(rust_triple "$1")/$PROFILE/cdj3k-emu"
    else
        echo "$REPO_ROOT/target/$PROFILE/cdj3k-emu"
    fi
}

# ── Build ─────────────────────────────────────────────────────────────────────
if [[ "$DO_BUILD" -eq 1 ]]; then
    "$REPO_ROOT/winit/fetch.sh"
    for arch in "${ARCHS[@]}"; do
        if [[ "$UNIVERSAL" -eq 1 ]]; then
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
# ad-hoc-signs what it rewrites; the real codesign below re-seals everything
# with the final identity.
SLICES=()
for arch in "${ARCHS[@]}"; do
    binary=$(binary_path "$arch")
    qemu=$(qemu_dir "$arch")
    for f in "$binary" "$qemu/lib/libcdj3k-emu-qemu.dylib" "$qemu/bin/qemu-img"; do
        if [[ ! -f "$f" ]]; then
            echo "ERROR: $arch: not found: $f" >&2
            if [[ "$UNIVERSAL" -eq 1 ]]; then
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

    <!-- Required for HVF (Hypervisor.framework) entitlement. -->
    <!-- Sign with a Developer ID cert for distribution; ad-hoc for local use. -->
    <key>com.apple.security.hypervisor</key>
    <true/>

    <!-- Entitlements that matter live in the code signature, not here; this
         key is informational.  The restricted ones (vmnet, virtual HID) come
         from Contents/embedded.provisionprofile - see the codesign step.
         Pro DJ Link networking is QEMU's own -netdev vmnet-bridged /
         vmnet-host, which vmnet authorises through that entitlement. -->
</dict>
</plist>
PLIST

# ── CoreMIDI driver plugin ───────────────────────────────────────────────────
# Host bundle (not a cargo/qemu artifact), so build it here regardless of
# --no-build.  Shipped in Resources; pc_link::midi_driver::ensure_driver_installed
# copies it into ~/Library/Audio/MIDI Drivers on the first PC Link toggle.
# --deep does not descend into Resources, so the codesign step signs it
# explicitly before sealing the app.
echo "==> Building CoreMIDI driver plugin"
make -C "$REPO_ROOT/tools/midi-driver" clean >/dev/null 2>&1 || true
midi_archflags=""
for arch in "${ARCHS[@]}"; do midi_archflags+=" -arch $arch"; done
make -C "$REPO_ROOT/tools/midi-driver" ARCHFLAGS="$midi_archflags"
cp -R "$REPO_ROOT/tools/midi-driver/CDJ3KEmuMIDI.plugin" "$RESOURCES_DIR/"
echo "     bundled CDJ3KEmuMIDI.plugin"

# ── Codesign ─────────────────────────────────────────────────────────────────
# cdj3k-emu calls Hypervisor.framework via libcdj3k-emu-qemu.dylib - the entitlement
# must be on the process binary (cdj3k-emu), not the dylib.
#
# With a real identity (Apple Development / Developer ID):
#   --options runtime enables the hardened runtime required for notarization
#   and is also what allows TCC (Full Disk Access) to track the app by its
#   bundle ID so it appears in System Settings → Privacy & Security → FDA.
#   --timestamp embeds a secure timestamp; notarization rejects a signature
#   without one, and it is what keeps the signature valid after the
#   certificate expires.
#
# With ad-hoc (-): HVF works locally but TCC cannot identify the app -
#   it will never appear in the FDA list and physical USB passthrough
#   will be denied by macOS even if the user is in the operator group.

# Restricted entitlements (vmnet, virtual HID) only take effect when an
# Apple-issued provisioning profile authorising them sits at
# Contents/embedded.provisionprofile.  AMFI kills the process at launch if the
# signature carries a restricted key the profile does not grant, so the keys
# below are read back out of the profile: the signature is a subset of the
# grant by construction.
PROFILE_ENT=""
if [[ -f "$PROVISION_PROFILE" ]]; then
    echo "==> Provisioning profile: $PROVISION_PROFILE"
    PROFILE_PLIST=$(mktemp -t provisionprofile)
    TMP_CLEANUP+=("$PROFILE_PLIST")
    if ! security cms -D -i "$PROVISION_PROFILE" -o "$PROFILE_PLIST" 2>/dev/null; then
        echo "ERROR: could not decode provisioning profile: $PROVISION_PROFILE" >&2
        exit 1
    fi
    # The profile's App ID must match CFBundleIdentifier or the signature is
    # rejected; catching it here beats a launch-time AMFI kill with no message.
    PROFILE_APPID=$(/usr/libexec/PlistBuddy -c \
        "Print :Entitlements:com.apple.application-identifier" "$PROFILE_PLIST" 2>/dev/null || echo "")
    if [[ "$PROFILE_APPID" != *".com.cdj3k.emu" ]]; then
        echo "ERROR: profile App ID '$PROFILE_APPID' does not match com.cdj3k.emu" >&2
        exit 1
    fi
    # keychain-access-groups is deliberately not carried over: the app does not
    # use the keychain, and claiming a group changes its keychain scope.
    for key in com.apple.application-identifier \
               com.apple.developer.team-identifier \
               com.apple.developer.hid.virtual.device \
               com.apple.developer.networking.vmnet; do
        val=$(/usr/libexec/PlistBuddy -c "Print :Entitlements:$key" "$PROFILE_PLIST" 2>/dev/null) || continue
        case "$val" in
            true)  PROFILE_ENT+="    <key>$key</key>"$'\n'"    <true/>"$'\n' ;;
            false) ;;
            *)     PROFILE_ENT+="    <key>$key</key>"$'\n'"    <string>$val</string>"$'\n' ;;
        esac
        echo "     granted: $key"
    done
    cp "$PROVISION_PROFILE" "$APP_DIR/Contents/embedded.provisionprofile"
else
    echo "==> No provisioning profile at $PROVISION_PROFILE"
    echo "     signing with the free entitlements only - vmnet and virtual HID"
    echo "     will be unavailable.  Pass --profile PATH to include them."
fi

HVF_ENT=$(mktemp -t cdj3k-entitlements)
TMP_CLEANUP+=("$HVF_ENT")
# allow-jit: the bundled QEMU imports pthread_jit_write_protect_np for its TCG
# backend (CDJ3K_EMU_TCG=1 selects it over HVF).  Under the hardened runtime
# that API needs the entitlement or the JIT mapping fails.  Free - no profile.
cat > "$HVF_ENT" <<ENT
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
    "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.hypervisor</key>
    <true/>
    <key>com.apple.security.cs.allow-jit</key>
    <true/>
${PROFILE_ENT}</dict>
</plist>
ENT

if [[ -n "$SIGN_IDENTITY" ]]; then
    echo "==> Codesigning bundle (identity: $SIGN_IDENTITY)"
    # Nested code in Resources is sealed as data by --deep, not re-signed.
    codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" \
        "$RESOURCES_DIR/CDJ3KEmuMIDI.plugin"
    # Deep-sign all nested binaries first (no entitlements on helpers/dylibs).
    codesign --force --deep --options runtime --timestamp --sign "$SIGN_IDENTITY" "$APP_DIR"
    # --deep strips entitlements; the main binary is signed again here.
    codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" \
        --entitlements "$HVF_ENT" "$MACOS_DIR/cdj3k-emu"
    codesign --verify --deep --strict "$APP_DIR"
    echo "     signed with: $SIGN_IDENTITY"
    echo "     TCC/FDA: grant Full Disk Access to CDJ3K Emulator in"
    echo "              System Settings → Privacy & Security → Full Disk Access"
else
    echo "==> Codesigning bundle (ad-hoc - TCC/FDA will not work)"
    echo "     Pass --sign \"Apple Development\" or set CODESIGN_IDENTITY to enable FDA."
    codesign --force --deep --sign - "$APP_DIR"
    # --deep strips entitlements; CDJ3K Emulator is signed again after it.
    codesign --force --sign - --entitlements "$HVF_ENT" "$MACOS_DIR/cdj3k-emu"
fi

rm "$HVF_ENT"

# ── Notarization (optional) ──────────────────────────────────────────────────
# Submit, wait for Apple's verdict, staple the ticket.  The .app is notarized
# on its own (zipped: notarytool takes zip/dmg/pkg) so the copy inside the DMG
# already carries a stapled ticket; the DMG is then signed and notarized as a
# second artefact below.  `spctl` afterwards is the same check Gatekeeper runs
# on first launch.
notarize_path() {
    local what="$1" log
    log=$(mktemp -t notary-log)
    TMP_CLEANUP+=("$log")
    echo "==> Notarizing $(basename "$what") (profile: $NOTARY_PROFILE)"
    if ! xcrun notarytool submit "$what" --keychain-profile "$NOTARY_PROFILE" \
            --wait 2>&1 | tee "$log"; then
        local id
        id=$(awk '/^ *id:/{print $2; exit}' "$log")
        echo "ERROR: notarization of $(basename "$what") failed" >&2
        if [[ -n "$id" ]]; then
            echo "       Apple's log for submission $id:" >&2
            xcrun notarytool log "$id" --keychain-profile "$NOTARY_PROFILE" >&2 || true
        fi
        exit 1
    fi
    if ! grep -q "status: Accepted" "$log"; then
        echo "ERROR: notarization of $(basename "$what") was not accepted (see above)" >&2
        exit 1
    fi
}

if [[ "$NOTARIZE" -eq 1 ]]; then
    NOTARY_STAGING=$(mktemp -d)
    TMP_CLEANUP+=("$NOTARY_STAGING")
    APP_ZIP="$NOTARY_STAGING/$(basename "$APP_DIR").zip"
    ditto -c -k --keepParent "$APP_DIR" "$APP_ZIP"
    notarize_path "$APP_ZIP"
    xcrun stapler staple "$APP_DIR"
    spctl -a -t exec -vv "$APP_DIR"
    echo "     stapled: $APP_DIR"
fi

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

    if [[ -n "$SIGN_IDENTITY" ]]; then
        codesign --force --timestamp --sign "$SIGN_IDENTITY" "$DMG_PATH"
        echo "     signed DMG with: $SIGN_IDENTITY"
    fi
    if [[ "$NOTARIZE" -eq 1 ]]; then
        notarize_path "$DMG_PATH"
        xcrun stapler staple "$DMG_PATH"
        spctl -a -t open --context context:primary-signature -vv "$DMG_PATH"
        echo "     stapled: $DMG_PATH"
    fi
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