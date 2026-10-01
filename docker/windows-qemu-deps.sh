#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# windows-qemu-deps.sh - build QEMU's library dependencies for one Windows
# architecture with llvm-mingw (UCRT), into that target's llvm-mingw sysroot
# (/opt/llvm-mingw/<arch>-w64-mingw32), where the compiler finds headers and
# import libraries without extra flags.
#
#   windows-qemu-deps.sh <x86_64|aarch64>
#
# Runs inside docker/Dockerfile.windows-qemu. Every source archive is pinned by
# sha256.
set -euo pipefail

ARCH="${1:?Usage: $0 <x86_64|aarch64>}"
case "$ARCH" in
    x86_64|aarch64) ;;
    *) echo "ERROR: unsupported arch: $ARCH" >&2; exit 1 ;;
esac

TRIPLE="${ARCH}-w64-mingw32"
PREFIX="/opt/llvm-mingw/${TRIPLE}"
WORK="/tmp/deps-${ARCH}"
JOBS="$(nproc)"

mkdir -p "$PREFIX" "$WORK"
cd "$WORK"

export PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig:$PREFIX/share/pkgconfig"
export PKG_CONFIG_PATH=
export CC="${TRIPLE}-clang"
export CXX="${TRIPLE}-clang++"
export AR="llvm-ar"
export RANLIB="llvm-ranlib"
export STRIP="llvm-strip"

fetch() { # <url> <sha256> -> prints the extracted directory
    local url="$1" sum="$2" file
    file="$(basename "$url")"
    # A tag archive downloads as a bare <tag>.tar.gz.
    case "$file" in [0-9]*.tar.gz) file="proxy-libintl-${file}" ;; esac
    curl -fsSL -o "$file" "$url"
    echo "$sum  $file" | sha256sum -c - >&2
    tar xf "$file"
    local dir
    dir="$(tar tf "$file" | head -1 | cut -d/ -f1)"
    echo "$WORK/$dir"
}

cat > "$WORK/cross.ini" <<EOF
[binaries]
c = '${TRIPLE}-clang'
cpp = '${TRIPLE}-clang++'
ar = 'llvm-ar'
strip = 'llvm-strip'
windres = '${TRIPLE}-windres'
pkg-config = 'pkg-config'

[host_machine]
system = 'windows'
cpu_family = '${ARCH}'
cpu = '${ARCH}'
endian = 'little'

[built-in options]
prefix = '${PREFIX}'
EOF

autotools() { # <dir> [configure args...]
    local dir="$1"; shift
    (cd "$dir" && ./configure --host="$TRIPLE" --prefix="$PREFIX" \
        --enable-shared --disable-static "$@" \
        && make -j"$JOBS" && make install)
}

mesontool() { # <dir> [meson args...]
    local dir="$1"; shift
    meson setup "$dir/build" "$dir" --cross-file "$WORK/cross.ini" \
        --buildtype=release --default-library=shared --wrap-mode=nofallback \
        "$@"
    ninja -C "$dir/build" -j"$JOBS" install
}

# zlib
d="$(fetch https://zlib.net/fossils/zlib-1.3.1.tar.gz \
    9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23)"
(cd "$d" && make -f win32/Makefile.gcc -j"$JOBS" \
    PREFIX="${TRIPLE}-" CC="$CC" AR="$AR" RC="${TRIPLE}-windres" \
    SHARED_MODE=1 \
    BINARY_PATH="$PREFIX/bin" INCLUDE_PATH="$PREFIX/include" \
    LIBRARY_PATH="$PREFIX/lib" install)
mkdir -p "$PREFIX/lib/pkgconfig"
cat > "$PREFIX/lib/pkgconfig/zlib.pc" <<EOF
prefix=$PREFIX
libdir=\${prefix}/lib
includedir=\${prefix}/include
Name: zlib
Description: zlib compression library
Version: 1.3.1
Libs: -L\${libdir} -lz
Cflags: -I\${includedir}
EOF

# libiconv, libffi, pcre2
d="$(fetch https://ftp.gnu.org/pub/gnu/libiconv/libiconv-1.17.tar.gz \
    8f74213b56238c85a50a5329f77e06198771e70dd9a739779f4c02f65d971313)"
autotools "$d"
d="$(fetch https://github.com/libffi/libffi/releases/download/v3.4.6/libffi-3.4.6.tar.gz \
    b0dea9df23c863a7a50e825440f3ebffabd65df1497108e5d437747843895a4e)"
autotools "$d" --disable-docs --disable-symvers
d="$(fetch https://github.com/PCRE2Project/pcre2/releases/download/pcre2-10.44/pcre2-10.44.tar.bz2 \
    d34f02e113cf7193a1ebf2770d3ac527088d485d4e047ed10e5d217c6ef5de96)"
autotools "$d"

# proxy-libintl: glib's libintl on Windows, without gettext
d="$(fetch https://github.com/frida/proxy-libintl/archive/refs/tags/0.4.tar.gz \
    13ef3eea0a3bc0df55293be368dfbcff5a8dd5f4759280f28e030d1494a5dffb)"
mesontool "$d"

# glib
d="$(fetch https://download.gnome.org/sources/glib/2.80/glib-2.80.5.tar.xz \
    9f23a9de803c695bbfde7e37d6626b18b9a83869689dd79019bf3ae66c3e6771)"
mesontool "$d" -Dtests=false -Dlibmount=disabled \
    -Dselinux=disabled -Dxattr=false -Dman-pages=disabled \
    -Dintrospection=disabled -Dsysprof=disabled -Dlibelf=disabled \
    -Dglib_debug=disabled

# pixman, libslirp
d="$(fetch https://cairographics.org/releases/pixman-0.43.4.tar.gz \
    a0624db90180c7ddb79fc7a9151093dc37c646d8c38d3f232f767cf64b85a226)"
mesontool "$d" -Dtests=disabled -Ddemos=disabled -Dgtk=disabled -Dlibpng=disabled
d="$(fetch https://gitlab.freedesktop.org/slirp/libslirp/-/archive/v4.8.0/libslirp-v4.8.0.tar.gz \
    2a98852e65666db313481943e7a1997abff0183bd9bea80caec1b5da89fda28c)"
mesontool "$d"

rm -rf "$WORK"
echo "deps for ${ARCH} installed in ${PREFIX}"
