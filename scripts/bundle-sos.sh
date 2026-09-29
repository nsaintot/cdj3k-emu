#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# bundle-sos.sh - vendor an ELF's shared libraries beside it and point the
# binaries at them with $ORIGIN.
#
#   bundle-sos.sh <bin dir> <lib dir> <binary> [binary...]
#
# Anything that talks to a kernel driver, a display server or a sound daemon
# is the user's own copy; everything else is vendored. The host-owned list
# matches the dependencies in packaging/linux/nfpm.yaml.in.
set -euo pipefail

BIN_DIR="${1:?Usage: $0 <bin dir> <lib dir> <binary>...}"; shift
LIB_DIR="${1:?Usage: $0 <bin dir> <lib dir> <binary>...}"; shift
[[ $# -gt 0 ]] || { echo "ERROR: no binaries given" >&2; exit 1; }

command -v patchelf >/dev/null || { echo "ERROR: patchelf not installed" >&2; exit 1; }

mkdir -p "$LIB_DIR"

# Never vendored. The first group would bind us to the wrong driver or
# daemon; the second is the C runtime, which the glibc floor covers and which
# cannot be mixed with the host's loader.
is_host_owned() {
    case "$1" in
        # Drivers, display, sound: the user's stack, not ours.
        libGL.so*|libGLX.so*|libGLdispatch.so*|libEGL.so*|libgbm.so*|libdrm.so*|\
        libOpenGL.so*|libGLESv2.so*|\
        libX11.so*|libX11-xcb.so*|libxcb*.so*|libXau.so*|libXdmcp.so*|libXext.so*|\
        libXi.so*|libXrandr.so*|libXcursor.so*|libXrender.so*|libXfixes.so*|\
        libwayland-*.so*|libxkbcommon*.so*|\
        libpipewire-*.so*|libpulse*.so*|libasound.so*|libjack*.so*|libspa-*.so*)
            return 0 ;;
        # The C runtime and the loader.
        ld-linux*.so*|libc.so*|libm.so*|libdl.so*|libpthread.so*|librt.so*|\
        libresolv.so*|libutil.so*|libgcc_s.so*|libstdc++.so*|libatomic.so*)
            return 0 ;;
    esac
    return 1
}

copied=()
missing=()
# Breadth-first over the dependency graph: a vendored library's own
# dependencies have to come too, or the loader finds nothing at run time.
queue=("$@")
seen=""
while [[ ${#queue[@]} -gt 0 ]]; do
    target="${queue[0]}"
    queue=("${queue[@]:1}")
    [[ -f "$target" ]] || continue

    while read -r name _arrow path _addr; do
        is_host_owned "$name" && continue
        # `ldd` prints "not found" for a library this machine does not have;
        # it is collected and the run fails below.
        if [[ -z "${path:-}" || ! -f "$path" ]]; then
            case " ${missing[*]-} " in *" $name "*) ;; *) missing+=("$name") ;; esac
            continue
        fi
        case " $seen " in *" $name "*) continue ;; esac
        seen="$seen $name"
        cp -L "$path" "$LIB_DIR/$name"
        chmod u+w "$LIB_DIR/$name"
        copied+=("$name")
        queue+=("$LIB_DIR/$name")
    done < <(ldd "$target" 2>/dev/null | sed -E 's/^\s+//' | awk '$2 == "=>" {print $1, $2, $3, $4}')
done

# Executables look one directory up and across into lib/; a vendored library
# looks beside itself. Both are relative to the file, so the tree can be
# installed anywhere or mounted at a path that changes every launch.
for target in "$@"; do
    [[ -f "$target" ]] || continue
    patchelf --set-rpath '$ORIGIN/../lib' "$target"
done
for name in "${copied[@]}"; do
    patchelf --set-rpath '$ORIGIN' "$LIB_DIR/$name"
done

if [[ ${#missing[@]} -gt 0 ]]; then
    echo "ERROR: these libraries are needed but not installed here, so they" >&2
    echo "       cannot be vendored and the package would not run:" >&2
    printf '         %s\n' "${missing[@]}" >&2
    echo "       Install them in the build environment and run again." >&2
    exit 1
fi

echo "     vendored ${#copied[@]} libraries into ${LIB_DIR#"$BIN_DIR"/../}"
if [[ ${#copied[@]} -gt 0 ]]; then
    printf '       %s\n' "${copied[@]}" | sort | head -40
fi
