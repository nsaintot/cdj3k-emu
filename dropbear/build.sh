#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# dropbear/build.sh
#
# Clone dropbear at DROPBEAR_TAG and build it as one static aarch64 binary
# for firmware that ships no SSH server (e.g. the CDJ-1500X). The build runs
# in an arm64 Alpine container, so musl links it fully static and it runs on
# any guest libc. build.sh calls this and copies dropbear/out/ into guest/out/.
#
# Usage:
#   bash dropbear/build.sh
#
# Output:
#   dropbear/src/                       - dropbear source tree
#   dropbear/out/dropbear_aarch64       - multi-call binary: dropbear, dbclient,
#                                         dropbearkey and scp by argv[0]
#   dropbear/out/dropbear_aarch64.LICENSE
#
# Idempotent: nothing happens while dropbear/out/ was built from the same tag
# and build flags.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC_DIR="${SCRIPT_DIR}/src"
OUT_DIR="${SCRIPT_DIR}/out"

DROPBEAR_URL="https://github.com/mkj/dropbear.git"
DROPBEAR_TAG="DROPBEAR_2026.94"
DROPBEAR_COMMIT="28216cd9af822732a1549b78621c2ea9a76a0fe5"
ALPINE_IMAGE="arm64v8/alpine:3.19"

CONFIGURE_FLAGS="--enable-static --disable-zlib --disable-pam \
--disable-lastlog --disable-utmp --disable-utmpx --disable-wtmp --disable-wtmpx"
PROGRAMS="dropbear dbclient dropbearkey scp"

stamp="${DROPBEAR_COMMIT} ${ALPINE_IMAGE} ${CONFIGURE_FLAGS} ${PROGRAMS}"
if [ -f "${OUT_DIR}/dropbear_aarch64" ] && [ -f "${OUT_DIR}/.cdj3k-stamp" ] \
    && [ "$(cat "${OUT_DIR}/.cdj3k-stamp")" = "${stamp}" ]; then
    echo "==> dropbear ${DROPBEAR_TAG} already built - skipping"
    exit 0
fi

if [ ! -d "${SRC_DIR}/.git" ]; then
    echo "==> Cloning dropbear ${DROPBEAR_TAG} …"
    git -c advice.detachedHead=false clone --quiet --depth 1 --branch "${DROPBEAR_TAG}" "${DROPBEAR_URL}" "${SRC_DIR}"
fi
got="$(git -C "${SRC_DIR}" rev-parse HEAD)"
if [ "${got}" != "${DROPBEAR_COMMIT}" ]; then
    echo "ERROR: dropbear/src is at ${got}, expected ${DROPBEAR_COMMIT} (${DROPBEAR_TAG})" >&2
    echo "       Remove dropbear/src and re-run." >&2
    exit 1
fi

echo "==> Building dropbear ${DROPBEAR_TAG} (static aarch64) …"
rm -rf "${OUT_DIR}"
mkdir -p "${OUT_DIR}"
# The tree is built in a copy inside the container, so dropbear/src stays clean.
docker run --rm --platform linux/arm64 \
    -v "${SRC_DIR}:/src:ro" -v "${OUT_DIR}:/out" \
    "${ALPINE_IMAGE}" sh -euc "
        apk add --no-cache build-base > /dev/null
        cp -r /src /build && cd /build
        ./configure ${CONFIGURE_FLAGS} > /tmp/build.log 2>&1 \\
            && make -j\$(nproc) PROGRAMS='${PROGRAMS}' MULTI=1 STATIC=1 >> /tmp/build.log 2>&1 \\
            || { tail -40 /tmp/build.log >&2; exit 1; }
        strip dropbearmulti
        cp dropbearmulti /out/dropbear_aarch64
        cp LICENSE /out/dropbear_aarch64.LICENSE
    "
echo "${stamp}" > "${OUT_DIR}/.cdj3k-stamp"
echo "  ✓  dropbear/out/dropbear_aarch64 ($(wc -c < "${OUT_DIR}/dropbear_aarch64" | tr -d ' ') bytes)"
