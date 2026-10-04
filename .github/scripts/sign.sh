#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# sign.sh FILE - print FILE's Ed25519 signature, base64, after checking it
# against packaging/update-ed25519.pub.
#
# UPDATE_SIGNING_KEY is the base64 32-byte seed (the format of Sparkle's
# generate_keys -x). The signature is over the file's bytes, which is what
# Sparkle checks for an enclosure and the app checks for the index.
set -euo pipefail
file="$1"
: "${UPDATE_SIGNING_KEY:?}"
script_dir="$(cd "$(dirname "$0")" && pwd)"
public_key="$script_dir/../../packaging/update-ed25519.pub"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
# PKCS#8 and SubjectPublicKeyInfo wrappers around the raw Ed25519 key bytes.
{ printf '\x30\x2e\x02\x01\x00\x30\x05\x06\x03\x2b\x65\x70\x04\x22\x04\x20'
  printf '%s' "$UPDATE_SIGNING_KEY" | base64 -d; } > "$work/key.der"
openssl pkey -inform DER -in "$work/key.der" -out "$work/key.pem"
{ printf '\x30\x2a\x30\x05\x06\x03\x2b\x65\x70\x03\x21\x00'
  tr -d '[:space:]' < "$public_key" | base64 -d; } > "$work/pub.der"
openssl pkey -pubin -inform DER -in "$work/pub.der" -out "$work/pub.pem"

openssl pkeyutl -sign -inkey "$work/key.pem" -rawin -in "$file" -out "$work/sig.bin"
openssl pkeyutl -verify -pubin -inkey "$work/pub.pem" -rawin -in "$file" -sigfile "$work/sig.bin" >/dev/null || {
    echo "::error::UPDATE_SIGNING_KEY does not match packaging/update-ed25519.pub" >&2
    exit 1
}
base64 < "$work/sig.bin" | tr -d '\n'
