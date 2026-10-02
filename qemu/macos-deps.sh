#!/usr/bin/env bash
# qemu/macos-deps.sh
#
# Build the libraries QEMU links on macOS - glib, pixman, libslirp, libusb -
# as static archives for one architecture, at the bundle's deployment target.
# qemu/build.sh --macos ARCH calls it; a slice of the universal .app is QEMU
# linked against this prefix, so it names no dylib outside the OS.
#
# Usage:
#   bash qemu/macos-deps.sh x86_64|arm64
#
# Output:
#   qemu/deps-macos-<arch>/              prefix: lib/*.a, include/, lib/pkgconfig/
#   qemu/deps-macos-<arch>/pkg-config    pkg-config that always answers --static
#   qemu/deps-macos-<arch>/cross.ini     meson cross file for <arch>
#   qemu/deps-src/                       downloaded tarballs, shared by both arches
#
# Needs python3, ninja, pkg-config and curl. Meson is installed into a venv
# under qemu/deps-src/. Re-running skips every library already installed.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

ARCH="${1:-}"
case "${ARCH}" in
    x86_64|arm64) ;;
    *) echo "Usage: $0 x86_64|arm64" >&2; exit 1 ;;
esac
case "${ARCH}" in
    x86_64) CPU_FAMILY=x86_64 ;;
    arm64)  CPU_FAMILY=aarch64 ;;
esac

# Info.plist's LSMinimumSystemVersion.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-15.0}"

SRC_ROOT="${SCRIPT_DIR}/deps-src"
PREFIX="${SCRIPT_DIR}/deps-macos-${ARCH}"
WORK="${PREFIX}/build"
SDK="$(xcrun --sdk macosx --show-sdk-path)"
JOBS="${JOBS:-$(sysctl -n hw.logicalcpu)}"

MESON_VERSION="1.9.1"
GLIB_VERSION="2.84.4"
PIXMAN_VERSION="0.46.4"
SLIRP_VERSION="4.9.1"
LIBUSB_VERSION="1.0.29"

# name|url|sha256
SOURCES=(
    "glib-${GLIB_VERSION}.tar.xz|https://download.gnome.org/sources/glib/${GLIB_VERSION%.*}/glib-${GLIB_VERSION}.tar.xz|8a9ea10943c36fc117e253f80c91e477b673525ae45762942858aef57631bb90"
    "pixman-${PIXMAN_VERSION}.tar.gz|https://cairographics.org/releases/pixman-${PIXMAN_VERSION}.tar.gz|d09c44ebc3bd5bee7021c79f922fe8fb2fb57f7320f55e97ff9914d2346a591c"
    "libslirp-v${SLIRP_VERSION}.tar.gz|https://gitlab.freedesktop.org/slirp/libslirp/-/archive/v${SLIRP_VERSION}/libslirp-v${SLIRP_VERSION}.tar.gz|3970542143b7c11e6a09a4d2b50f30a133473c41f15ed0bdcc3b7a1c450d9a5c"
    "libusb-${LIBUSB_VERSION}.tar.bz2|https://github.com/libusb/libusb/releases/download/v${LIBUSB_VERSION}/libusb-${LIBUSB_VERSION}.tar.bz2|5977fc950f8d1395ccea9bd48c06b3f808fd3c2c961b44b0c2e6e29fc3a70a85"
)

mkdir -p "${SRC_ROOT}" "${PREFIX}/lib/pkgconfig" "${WORK}"

# --------------------------------------------------------------------------
# Tools
# --------------------------------------------------------------------------

for tool in python3 ninja pkg-config curl; do
    command -v "${tool}" >/dev/null || { echo "ERROR: ${tool} not found" >&2; exit 1; }
done

VENV="${SRC_ROOT}/meson-venv"
if [ ! -x "${VENV}/bin/meson" ] || [ "$("${VENV}/bin/meson" --version)" != "${MESON_VERSION}" ]; then
    echo "==> Installing meson ${MESON_VERSION} into ${VENV}"
    python3 -m venv "${VENV}"
    "${VENV}/bin/pip" install -q "meson==${MESON_VERSION}" packaging
fi
MESON="${VENV}/bin/meson"

# Every lookup resolves inside the prefix, statically: a static glib needs
# its Requires.private (pcre2, libffi) on QEMU's link line.
REAL_PKG_CONFIG="$(command -v pkg-config)"
cat > "${PREFIX}/pkg-config" <<EOF
#!/bin/sh
PKG_CONFIG_LIBDIR="${PREFIX}/lib/pkgconfig" PKG_CONFIG_PATH= exec "${REAL_PKG_CONFIG}" --static "\$@"
EOF
chmod +x "${PREFIX}/pkg-config"

# -isysroot keeps /usr/local (an Intel Homebrew) off the search paths. An SDK
# newer than the deployment target declares calls the target lacks (pipe2 in
# the macOS 27 SDK); as errors, configure checks find them missing instead of
# the binary failing to load on an older macOS.
AVAIL_FLAGS="-Werror=unguarded-availability -Werror=unguarded-availability-new"
MIN_FLAGS="-isysroot ${SDK} -mmacosx-version-min=${MACOSX_DEPLOYMENT_TARGET}"
ARCH_FLAGS="-arch ${ARCH} ${MIN_FLAGS} ${AVAIL_FLAGS}"

# A cross file even for the host's own arch: it is what pins the arch flags
# and the pkg-config above for every meson project.
cat > "${PREFIX}/cross.ini" <<EOF
[binaries]
c = ['clang', '-arch', '${ARCH}']
cpp = ['clang++', '-arch', '${ARCH}']
objc = ['clang', '-arch', '${ARCH}']
ar = 'ar'
strip = 'strip'
pkg-config = '${PREFIX}/pkg-config'

[constants]
min = ['-isysroot', '${SDK}', '-mmacosx-version-min=${MACOSX_DEPLOYMENT_TARGET}']
avail = ['-Werror=unguarded-availability', '-Werror=unguarded-availability-new']

[built-in options]
c_args = min + avail
cpp_args = min + avail
objc_args = min + avail
c_link_args = min
cpp_link_args = min
objc_link_args = min

[host_machine]
system = 'darwin'
subsystem = 'macos'
kernel = 'xnu'
cpu_family = '${CPU_FAMILY}'
cpu = '${CPU_FAMILY}'
endian = 'little'
EOF

# libffi is the SDK's.
cat > "${PREFIX}/lib/pkgconfig/libffi.pc" <<EOF
Name: libffi
Description: libffi from the macOS SDK
Version: 3.4.0
Cflags: -I${SDK}/usr/include/ffi
Libs: -lffi
EOF

# --------------------------------------------------------------------------
# Sources
# --------------------------------------------------------------------------

fetch() {
    local name="$1" url="$2" sum="$3" file="${SRC_ROOT}/$1"
    if [ ! -f "${file}" ]; then
        echo "==> Fetching ${name}"
        curl -fsSL -o "${file}.part" "${url}"
        mv "${file}.part" "${file}"
    fi
    if [ "$(shasum -a 256 "${file}" | cut -d' ' -f1)" != "${sum}" ]; then
        echo "ERROR: ${name}: sha256 mismatch" >&2
        exit 1
    fi
}

# A configure check links against the SDK's stubs, so it finds a call the
# deployment target lacks. scrub_config <config.h> drops each HAVE_<CALL>
# whose call the compiler rejects as newer than the target.
scrub_config() {
    local header="$1" define call probe
    probe="${WORK}/availability.c"
    for define in $(sed -n 's/^#define \(HAVE_[A-Z0-9_]*\) 1$/\1/p' "${header}"); do
        call="$(printf '%s' "${define#HAVE_}" | tr '[:upper:]' '[:lower:]')"
        cat > "${probe}" <<PROBE
#include <fcntl.h>
#include <poll.h>
#include <pthread.h>
#include <signal.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>
void *probe(void) { return (void *)&${call}; }
PROBE
        if [[ "$(clang ${ARCH_FLAGS} -fsyntax-only "${probe}" 2>&1)" == *"is only available on macOS"* ]]; then
            echo "    ${call}: newer than macOS ${MACOSX_DEPLOYMENT_TARGET}, ${define} dropped"
            sed -i '' "/^#define ${define} 1\$/d" "${header}"
        fi
    done
}

# unpack <tarball> -> prints the source directory, fresh for this arch
unpack() {
    local file="${SRC_ROOT}/$1" dir
    dir="${WORK}/$(tar -tf "${file}" | head -1 | cut -d/ -f1)"
    rm -rf "${dir}"
    tar -xf "${file}" -C "${WORK}"
    echo "${dir}"
}

for s in "${SOURCES[@]}"; do
    IFS='|' read -r name url sum <<<"${s}"
    fetch "${name}" "${url}" "${sum}"
done

# --------------------------------------------------------------------------
# Libraries
# --------------------------------------------------------------------------

# meson_lib <pc-name> <tarball> [meson options...]
meson_lib() {
    local pc="$1" tarball="$2"
    shift 2
    if [ -f "${PREFIX}/lib/pkgconfig/${pc}.pc" ]; then
        echo "==> ${pc} already installed - skipping"
        return
    fi
    echo "==> Building ${pc} (${ARCH})"
    local src
    src="$(unpack "${tarball}")"
    "${MESON}" setup "${src}/_build" "${src}" \
        --cross-file "${PREFIX}/cross.ini" \
        --prefix "${PREFIX}" --libdir lib \
        --buildtype release \
        -Ddefault_library=static \
        --wrap-mode=default \
        "$@"
    scrub_config "${src}/_build/config.h"
    ninja -C "${src}/_build" -j"${JOBS}"
    "${MESON}" install -C "${src}/_build" --no-rebuild >/dev/null
}

# pcre2 comes from glib's own wrap; no libintl, so glib's messages stay English.
meson_lib glib-2.0 "glib-${GLIB_VERSION}.tar.xz" \
    -Dnls=disabled -Dtests=false -Dintrospection=disabled -Dman-pages=disabled \
    -Ddocumentation=false -Dselinux=disabled -Dxattr=false -Dlibmount=disabled \
    -Dsysprof=disabled -Ddtrace=disabled -Dglib_debug=disabled \
    -Dpcre2:test=false -Dpcre2:grep=false

# a64-neon is ELF-only assembly.
meson_lib pixman-1 "pixman-${PIXMAN_VERSION}.tar.gz" \
    -Dtests=disabled -Ddemos=disabled -Dgtk=disabled -Dlibpng=disabled \
    -Da64-neon=disabled

meson_lib slirp "libslirp-v${SLIRP_VERSION}.tar.gz"

if [ -f "${PREFIX}/lib/pkgconfig/libusb-1.0.pc" ]; then
    echo "==> libusb-1.0 already installed - skipping"
else
    echo "==> Building libusb-1.0 (${ARCH})"
    src="$(unpack "libusb-${LIBUSB_VERSION}.tar.bz2")"
    (
        cd "${src}"
        ./configure --host="${ARCH/arm64/aarch64}-apple-darwin" \
            --prefix="${PREFIX}" --disable-shared --enable-static \
            CC="clang ${ARCH_FLAGS}" >/dev/null
        scrub_config config.h
        make -j"${JOBS}" >/dev/null
        make install >/dev/null
    )
fi

rm -rf "${WORK}"
echo ""
echo "Done. ${ARCH} static libraries in ${PREFIX}"
