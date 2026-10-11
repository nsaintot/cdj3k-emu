#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# build.sh - sign the staged .app and, optionally, notarize it and wrap it in
# a .dmg.
#
#   build.sh [--stage DIR] [--out DIR] [--sign IDENTITY] [--profile PATH]
#            [--dmg] [--notarize] [--notary-profile NAME]
#
#   --stage DIR      where packaging/macos/stage.sh put the .app
#                    (default: dist/macos)
#   --out DIR        where the signed .app and the .dmg go (default: dist)
#   --sign IDENTITY  codesign identity (overrides CODESIGN_IDENTITY env var)
#                    "Apple Development" picks your only development certificate.
#                    Without an identity the bundle is signed ad-hoc (-), and TCC
#                    and Full Disk Access do not work.
#   --profile PATH   Apple provisioning profile authorising the restricted
#                    entitlements (vmnet, virtual HID).  Default:
#                    ./cdj3k-emu.provisionprofile; PROVISION_PROFILE env var
#                    overrides.  Copied to Contents/embedded.provisionprofile and
#                    its granted entitlements are added to the signature.  Without
#                    it the bundle signs with the free entitlements only.
#   --dmg            also package the .app into a compressed .dmg,
#                    CDJ3K-Emulator-<version>-macos-<universal|arm64|x86_64>.dmg.
#   --version V      the version in the .dmg's name and volume name (default:
#                    CFBundleShortVersionString)
#   --notarize       after signing with a "Developer ID Application" identity,
#                    submit the .app (and the .dmg, with --dmg) to Apple's notary
#                    service, wait for the verdict and staple the tickets.  Needs
#                    credentials stored once with
#                      xcrun notarytool store-credentials <NAME> \
#                          --apple-id <APPLE_ID> --team-id <TEAM_ID>
#                    (prompts for an app-specific password from appleid.apple.com).
#   --notary-profile NAME
#                    keychain profile for --notarize (default: cdj3k-emu-notarization;
#                    NOTARY_PROFILE env var overrides).  NOTARY_KEYCHAIN names the
#                    keychain that holds it when that is not the default one.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

# shellcheck source=lib.sh
source "$REPO_ROOT/packaging/macos/lib.sh"

# ── Options ──────────────────────────────────────────────────────────────────
STAGE_DIR="$REPO_ROOT/dist/macos"
OUT_DIR="$REPO_ROOT/dist"
# A real identity, such as "Apple Development", lets TCC track the app for
# Full Disk Access. Without one, the bundle is signed ad-hoc ("-").
SIGN_IDENTITY="${CODESIGN_IDENTITY:-}"
MAKE_DMG=0
DMG_VERSION=""
NOTARIZE=0
NOTARY_PROFILE="${NOTARY_PROFILE:-cdj3k-emu-notarization}"
NOTARY_KEYCHAIN="${NOTARY_KEYCHAIN:-}"
PROVISION_PROFILE="${PROVISION_PROFILE:-$REPO_ROOT/cdj3k-emu.provisionprofile}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --stage)      STAGE_DIR="$2"; shift ;;
        --out)        OUT_DIR="$2"; shift ;;
        --sign)       SIGN_IDENTITY="$2"; shift ;;
        --profile)    PROVISION_PROFILE="$2"; shift ;;
        --dmg)        MAKE_DMG=1 ;;
        --version)    DMG_VERSION="$2"; shift ;;
        --notarize)   NOTARIZE=1 ;;
        --notary-profile) NOTARY_PROFILE="$2"; shift ;;
        -h|--help)    sed -n '3,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *)            echo "ERROR: unknown argument: $1" >&2; exit 1 ;;
    esac
    shift
done

NOTARY_AUTH=(--keychain-profile "$NOTARY_PROFILE")
if [[ -n "$NOTARY_KEYCHAIN" ]]; then
    NOTARY_AUTH+=(--keychain "$NOTARY_KEYCHAIN")
fi

# Notarization checks a Developer ID signature, and notarytool refuses ad-hoc
# and Apple Development signatures, so the identity is checked before the
# bundle is assembled.
if [[ "$NOTARIZE" -eq 1 ]]; then
    if [[ "$SIGN_IDENTITY" != *"Developer ID Application"* ]]; then
        echo "ERROR: --notarize needs --sign \"Developer ID Application: ...\"" >&2
        echo "       (got: '${SIGN_IDENTITY:-ad-hoc}')" >&2
        exit 1
    fi
    if ! xcrun notarytool history "${NOTARY_AUTH[@]}" >/dev/null 2>&1; then
        echo "ERROR: the keychain profile '$NOTARY_PROFILE' holds no notarytool credentials." >&2
        echo "       Store them once (needs an app-specific password from appleid.apple.com):" >&2
        echo "         xcrun notarytool store-credentials $NOTARY_PROFILE \\" >&2
        echo "             --apple-id <APPLE_ID> --team-id <TEAM_ID>" >&2
        exit 1
    fi
fi

# ── The staged .app ──────────────────────────────────────────────────────────
STAGED_APP="$STAGE_DIR/CDJ3K Emulator.app"
[[ -d "$STAGED_APP/Contents/MacOS" ]] || {
    echo "ERROR: $STAGED_APP is not a staged .app. Run packaging/macos/stage.sh." >&2
    exit 1
}
mkdir -p "$OUT_DIR"
APP_DIR="$OUT_DIR/CDJ3K Emulator.app"
MACOS_DIR="$APP_DIR/Contents/MacOS"
RESOURCES_DIR="$APP_DIR/Contents/Resources"
rm -rf "$APP_DIR"
cp -R "$STAGED_APP" "$APP_DIR"

# ── Codesign ─────────────────────────────────────────────────────────────────
# cdj3k-emu calls Hypervisor.framework through libcdj3k-emu-qemu.dylib; the
# entitlement must be on the process binary (cdj3k-emu), not on the dylib.
#
# With a real identity (Apple Development / Developer ID):
#   --options runtime enables the hardened runtime required for notarization
#   and is also what allows TCC (Full Disk Access) to track the app by its
#   bundle ID so it appears in System Settings → Privacy & Security → FDA.
#   --timestamp embeds a secure timestamp; notarization rejects a signature
#   without one, and it is what keeps the signature valid after the
#   certificate expires.
#
# With an ad-hoc signature (-), HVF works locally but TCC cannot identify the
#   app: it never appears in the FDA list, and macOS denies physical USB
#   passthrough even if the user is in the operator group.

# Restricted entitlements (vmnet, virtual HID) only take effect when an
# Apple-issued provisioning profile authorising them sits at
# Contents/embedded.provisionprofile.  AMFI kills the process at launch if the
# signature carries a restricted key the profile does not grant, so the keys
# below are read from the profile, and the signature never claims a key that
# the profile does not grant.
PROFILE_ENT=""
if [[ -f "$PROVISION_PROFILE" ]]; then
    echo "==> Provisioning profile: $PROVISION_PROFILE"
    PROFILE_PLIST=$(mktemp -t provisionprofile)
    TMP_CLEANUP+=("$PROFILE_PLIST")
    if ! security cms -D -i "$PROVISION_PROFILE" -o "$PROFILE_PLIST" 2>/dev/null; then
        echo "ERROR: could not decode the provisioning profile $PROVISION_PROFILE." >&2
        exit 1
    fi
    # The profile's App ID must match CFBundleIdentifier or the signature is
    # rejected. Check it here, because otherwise AMFI kills the app at launch
    # without a message.
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
    echo "     signing with the free entitlements only; vmnet and virtual HID"
    echo "     will be unavailable.  Pass --profile PATH to include them."
fi

HVF_ENT=$(mktemp -t cdj3k-entitlements)
TMP_CLEANUP+=("$HVF_ENT")
# allow-jit: the bundled QEMU imports pthread_jit_write_protect_np for its TCG
# backend (CDJ3K_EMU_TCG=1 selects it over HVF).  Under the hardened runtime
# that API needs the entitlement or the JIT mapping fails.  The entitlement is
# free and needs no profile.
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
    # --deep seals code in Resources as data and does not sign it, so the MIDI
    # plug-in is signed first.
    codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" \
        "$RESOURCES_DIR/CDJ3KEmuMIDI.plugin"
    # Sign Sparkle from the inside out: its helpers first, then the framework.
    SPARKLE="$APP_DIR/Contents/Frameworks/Sparkle.framework"
    for code in "$SPARKLE/Versions/B/Autoupdate" "$SPARKLE/Versions/B/Updater.app" "$SPARKLE"; do
        codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$code"
    done
    # Deep-sign all nested binaries first, without entitlements on helpers and
    # dylibs.
    codesign --force --deep --options runtime --timestamp --sign "$SIGN_IDENTITY" "$APP_DIR"
    # --deep strips entitlements; the main binary is signed again here.
    codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" \
        --entitlements "$HVF_ENT" "$MACOS_DIR/cdj3k-emu"
    codesign --verify --deep --strict "$APP_DIR"
    echo "     signed with: $SIGN_IDENTITY"
    echo "     TCC/FDA: grant Full Disk Access to CDJ3K Emulator in"
    echo "              System Settings → Privacy & Security → Full Disk Access"
else
    echo "==> Codesigning bundle (ad-hoc; TCC and Full Disk Access will not work)"
    echo "     Pass --sign \"Apple Development\" or set CODESIGN_IDENTITY to enable FDA."
    codesign --force --deep --sign - "$APP_DIR"
    # --deep strips entitlements, so the main binary is signed again after it.
    codesign --force --sign - --entitlements "$HVF_ENT" "$MACOS_DIR/cdj3k-emu"
fi

rm "$HVF_ENT"

# ── Notarization (optional) ──────────────────────────────────────────────────
# Submit the file, wait for Apple's verdict and staple the ticket.  The .app is notarized
# on its own (zipped: notarytool takes zip/dmg/pkg) so the copy inside the DMG
# already carries a stapled ticket; the DMG is then signed and notarized as a
# second artefact below.  `spctl` afterwards is the same check Gatekeeper runs
# on first launch.
notarize_path() {
    local what="$1" log
    log=$(mktemp -t notary-log)
    TMP_CLEANUP+=("$log")
    echo "==> Notarizing $(basename "$what") (profile: $NOTARY_PROFILE)"
    if ! xcrun notarytool submit "$what" "${NOTARY_AUTH[@]}" \
            --wait 2>&1 | tee "$log"; then
        local id
        id=$(awk '/^ *id:/{print $2; exit}' "$log")
        echo "ERROR: notarization of $(basename "$what") failed" >&2
        if [[ -n "$id" ]]; then
            echo "       Apple's log for submission $id:" >&2
            xcrun notarytool log "$id" "${NOTARY_AUTH[@]}" >&2 || true
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
    VERSION="$DMG_VERSION"
    if [[ -z "$VERSION" ]]; then
        VERSION=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" \
            "$APP_DIR/Contents/Info.plist" 2>/dev/null || echo "0.0.0")
    fi
    # Name the DMG after the slices the binary carries; a binary with both
    # arm64 and x86_64 is the universal build.
    ARCHS=$(lipo -archs "$MACOS_DIR/cdj3k-emu")
    case "$ARCHS" in
        *arm64*x86_64*|*x86_64*arm64*) DMG_ARCH=universal ;;
        *) DMG_ARCH="$ARCHS" ;;
    esac
    DMG_PATH="$OUT_DIR/CDJ3K-Emulator-${VERSION}-macos-${DMG_ARCH}.dmg"
    DMG_STAGING=$(mktemp -d)
    TMP_CLEANUP+=("$DMG_STAGING")

    echo "==> Creating DMG: $DMG_PATH"
    cp -R "$APP_DIR" "$DMG_STAGING/"
    # A symlink to /Applications lets the user install the app by dragging it
    # onto the link.
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
exit 0
