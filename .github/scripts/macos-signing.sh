#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# macos-signing.sh - give a CI runner the Developer ID identity, the
# provisioning profile and the notary credentials packaging/macos/build.sh
# signs with.
#
#   macos-signing.sh setup     import into a temporary keychain; exports
#                              CODESIGN_IDENTITY, PROVISION_PROFILE,
#                              NOTARY_PROFILE and NOTARY_KEYCHAIN to $GITHUB_ENV
#   macos-signing.sh cleanup   delete that keychain and the decoded files
#
# Inputs (environment, from repository secrets):
#   MACOS_CERTIFICATE_P12_BASE64     Developer ID Application certificate and key
#   MACOS_CERTIFICATE_PASSWORD       its export password
#   MACOS_PROVISION_PROFILE_BASE64   optional; grants vmnet and virtual HID
#   NOTARY_API_KEY_BASE64            App Store Connect API key (.p8)
#   NOTARY_API_KEY_ID                its key ID
#   NOTARY_API_ISSUER_ID             its issuer ID
# Without the API key the identity is still imported; packaging/macos/build.sh
# --notarize then refuses.
set -euo pipefail

: "${RUNNER_TEMP:?RUNNER_TEMP not set}"
KEYCHAIN="$RUNNER_TEMP/cdj3k-signing.keychain-db"
WORK="$RUNNER_TEMP/cdj3k-signing"
NOTARY_PROFILE_NAME="cdj3k-emu-notarization"

setup() {
    : "${MACOS_CERTIFICATE_P12_BASE64:?}" "${MACOS_CERTIFICATE_PASSWORD:?}"
    mkdir -p "$WORK"
    local password
    password="$(openssl rand -hex 24)"

    security create-keychain -p "$password" "$KEYCHAIN"
    security set-keychain-settings -lut 21600 "$KEYCHAIN"
    security unlock-keychain -p "$password" "$KEYCHAIN"

    printf '%s' "$MACOS_CERTIFICATE_P12_BASE64" | base64 --decode > "$WORK/cert.p12"
    security import "$WORK/cert.p12" -k "$KEYCHAIN" -f pkcs12 \
        -P "$MACOS_CERTIFICATE_PASSWORD" -T /usr/bin/codesign -T /usr/bin/security
    rm -f "$WORK/cert.p12"
    # codesign reads the key without a UI prompt only once the partition list
    # names Apple's tools.
    security set-key-partition-list -S apple-tool:,apple:,codesign: -s \
        -k "$password" "$KEYCHAIN" >/dev/null
    # codesign and notarytool look the identity and the profile up through the
    # search list.
    local existing
    existing=$(security list-keychains -d user | tr -d '"' | xargs)
    # shellcheck disable=SC2086
    security list-keychains -d user -s "$KEYCHAIN" $existing

    local identity
    identity=$(security find-identity -v -p codesigning "$KEYCHAIN" \
        | sed -n 's/.*"\(Developer ID Application: .*\)"$/\1/p' | head -n1)
    if [[ -z "$identity" ]]; then
        echo "ERROR: no Developer ID Application identity in the certificate" >&2
        security find-identity -v -p codesigning "$KEYCHAIN" >&2
        exit 1
    fi
    echo "identity: $identity"
    echo "CODESIGN_IDENTITY=$identity" >> "$GITHUB_ENV"

    if [[ -n "${MACOS_PROVISION_PROFILE_BASE64:-}" ]]; then
        printf '%s' "$MACOS_PROVISION_PROFILE_BASE64" | base64 --decode \
            > "$WORK/cdj3k-emu.provisionprofile"
        echo "PROVISION_PROFILE=$WORK/cdj3k-emu.provisionprofile" >> "$GITHUB_ENV"
        echo "provisioning profile: installed"
    else
        echo "provisioning profile: none (vmnet and virtual HID stay ungranted)"
    fi

    if [[ -n "${NOTARY_API_KEY_BASE64:-}" ]]; then
        : "${NOTARY_API_KEY_ID:?}" "${NOTARY_API_ISSUER_ID:?}"
        printf '%s' "$NOTARY_API_KEY_BASE64" | base64 --decode > "$WORK/notary.p8"
        xcrun notarytool store-credentials "$NOTARY_PROFILE_NAME" \
            --key "$WORK/notary.p8" --key-id "$NOTARY_API_KEY_ID" \
            --issuer "$NOTARY_API_ISSUER_ID" --keychain "$KEYCHAIN"
        rm -f "$WORK/notary.p8"
        echo "notary: credentials stored"
    else
        echo "notary: no API key"
    fi
    echo "NOTARY_PROFILE=$NOTARY_PROFILE_NAME" >> "$GITHUB_ENV"
    echo "NOTARY_KEYCHAIN=$KEYCHAIN" >> "$GITHUB_ENV"
}

cleanup() {
    security delete-keychain "$KEYCHAIN" 2>/dev/null || true
    rm -rf "$WORK"
}

case "${1:-}" in
    setup)   setup ;;
    cleanup) cleanup ;;
    *) echo "Usage: $0 setup|cleanup" >&2; exit 2 ;;
esac
