#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# winit/fetch.sh
#
# Fetch winit at WINIT_VERSION from crates.io, apply winit/patches/*.patch in
# order, and leave the tree at winit/src, where Cargo.toml's
# [patch.crates-io] points. Run once before any cargo command; build.sh,
# bundle.sh and packaging/linux/stage.sh call it.
#
# Re-running is idempotent: nothing happens while winit/src was made from the
# same version and the same patches.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PATCHES_DIR="${SCRIPT_DIR}/patches"
SRC_DIR="${SCRIPT_DIR}/src"

WINIT_VERSION="0.30.13"
# sha256 of the .crate, as crates.io's index records it.
WINIT_SHA256="a6755fa58a9f8350bd1e472d4c3fcc25f824ec358933bba33306d0b63df5978d"
WINIT_URL="https://static.crates.io/crates/winit/winit-${WINIT_VERSION}.crate"

sha256() {
    if command -v sha256sum >/dev/null; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

# What winit/src was made from: the version and every patch's contents.
stamp() {
    {
        echo "${WINIT_VERSION}"
        cat "${PATCHES_DIR}"/*.patch
    } | { if command -v sha256sum >/dev/null; then sha256sum; else shasum -a 256; fi; } | cut -d' ' -f1
}

want="$(stamp)"
if [ -f "${SRC_DIR}/.cdj3k-stamp" ] && [ "$(cat "${SRC_DIR}/.cdj3k-stamp")" = "${want}" ]; then
    echo "==> winit ${WINIT_VERSION} already patched - skipping"
    exit 0
fi

echo "==> Fetching winit ${WINIT_VERSION} …"
tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT
curl -sSfL -o "${tmp}/winit.crate" "${WINIT_URL}"
got="$(sha256 "${tmp}/winit.crate")"
if [ "${got}" != "${WINIT_SHA256}" ]; then
    echo "ERROR: winit-${WINIT_VERSION}.crate sha256 ${got}, expected ${WINIT_SHA256}" >&2
    exit 1
fi
tar -xzf "${tmp}/winit.crate" -C "${tmp}"

for p in "${PATCHES_DIR}"/*.patch; do
    echo "    applying $(basename "${p}")"
    patch -d "${tmp}/winit-${WINIT_VERSION}" -p1 --forward --silent < "${p}"
done

rm -rf "${SRC_DIR}"
mv "${tmp}/winit-${WINIT_VERSION}" "${SRC_DIR}"
echo "${want}" > "${SRC_DIR}/.cdj3k-stamp"
echo "==> winit ready at ${SRC_DIR}"
