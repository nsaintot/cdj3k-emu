#!/usr/bin/env bash
# qemu/build.sh
#
# Clone QEMU v10.2.2, apply the shm display patch, and build qemu-system-aarch64.
#
# Usage:
#   cd <repo-root>
#   bash qemu/build.sh
#
# Output:
#   qemu/src/        - QEMU source tree
#   qemu/build/      - build artefacts
#   qemu/install/    - installed binaries (qemu-system-aarch64 lives here)
#
# Re-running is idempotent: clone and patch steps are skipped if already done.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PATCHES_DIR="${SCRIPT_DIR}/patches"
SHIM_DIR="${SCRIPT_DIR}/shim"
SRC_DIR="${SCRIPT_DIR}/src"
BUILD_DIR="${SCRIPT_DIR}/build"
INSTALL_DIR="${SCRIPT_DIR}/install"

# Pinned to a master snapshot that includes the HVF in-kernel GIC support
# (Mohamed Mediouni's series, merged 2026-05-05). Stable QEMU 11.0.0 was cut
# 2026-04-21, before the GIC work landed, so a master pin is required for
# in-kernel vGIC. Tip-of-staging snapshot 2026-05-06.
QEMU_REF="ee7eb612be8f8886d48c1d0c1f1c65e495138f83"
QEMU_REPO="https://github.com/qemu/qemu.git"

# --------------------------------------------------------------------------
# 1. Clone
# --------------------------------------------------------------------------

need_fetch=0
if [ ! -d "${SRC_DIR}/.git" ]; then
    need_fetch=1
else
    current_sha="$(git -C "${SRC_DIR}" rev-parse HEAD 2>/dev/null || echo unknown)"
    if [ "${current_sha}" != "${QEMU_REF}" ]; then
        echo "==> QEMU src at ${current_sha:0:12} does not match required ${QEMU_REF:0:12}"
        echo "    Wiping ${SRC_DIR} and re-fetching."
        rm -rf "${SRC_DIR}" "${BUILD_DIR}"
        need_fetch=1
    fi
fi

if [ "${need_fetch}" = "1" ]; then
    echo "==> Fetching QEMU @ ${QEMU_REF} …"
    git init -q "${SRC_DIR}"
    git -C "${SRC_DIR}" remote add origin "${QEMU_REPO}"
    git -C "${SRC_DIR}" fetch --depth=1 origin "${QEMU_REF}"
    git -C "${SRC_DIR}" checkout -q FETCH_HEAD
    # No submodules needed for a minimal aarch64-softmmu+cocoa build.
else
    echo "==> Source tree already at ${QEMU_REF:0:12} - skipping fetch"
fi

# --------------------------------------------------------------------------
# 2. Apply patches
# --------------------------------------------------------------------------
# Every QEMU modification (overlay file replacements, ivshmem gating, virtio
# hot-swap fix, etc.) is captured as a unified-diff patch in qemu/patches/.
# Patches are pinned to the QEMU_REF above; if the SHA is bumped, regenerate
# them by editing src/ and running `git -C src diff > patches/NN-foo.patch`.
#
# A failure to apply means upstream QEMU drifted under one of our anchors -
# fail loudly here so the user catches it before a half-patched src/ silently
# produces a binary missing features or behaving oddly.

if ! ls "${PATCHES_DIR}"/*.patch >/dev/null 2>&1; then
    echo "ERROR: no patches found in ${PATCHES_DIR}"
    exit 1
fi

# Check whether any patch is already applied to the current tree (idempotent
# re-runs after a successful build land here).  We probe with --reverse
# --check: if reversing applies cleanly, the patch is already in.
patch_changed=0
echo "==> Applying QEMU patches from $(basename "${PATCHES_DIR}")/"
for p in "${PATCHES_DIR}"/*.patch; do
    name="$(basename "$p")"
    if git -C "${SRC_DIR}" apply --reverse --check "$p" >/dev/null 2>&1; then
        echo "    [skip]   ${name} - already applied"
        continue
    fi
    if ! git -C "${SRC_DIR}" apply --check "$p" 2>/dev/null; then
        echo "    [FAIL]   ${name} - does not apply cleanly"
        echo
        echo "Upstream QEMU likely drifted under this patch's anchors."
        echo "Regenerate the patch:"
        echo "  - inspect: git -C ${SRC_DIR} apply --check ${p}"
        echo "  - rebase the change manually, then:"
        echo "      git -C ${SRC_DIR} diff -- <paths> > ${p}"
        exit 1
    fi
    git -C "${SRC_DIR}" apply "$p"
    echo "    [apply]  ${name}"
    patch_changed=1
done

# Force reconfigure when any patch landed - meson caches Kconfig results in
# build.ninja, so a stale build dir won't pick up newly-enabled symbols.
if [ "${patch_changed}" = "1" ] && [ -f "${BUILD_DIR}/build.ninja" ]; then
    echo "    invalidating ${BUILD_DIR}/build.ninja to trigger reconfigure"
    rm -f "${BUILD_DIR}/build.ninja"
fi

# --------------------------------------------------------------------------
# 3. Configure
# --------------------------------------------------------------------------

mkdir -p "${BUILD_DIR}"

if [ ! -f "${BUILD_DIR}/build.ninja" ]; then
    echo "==> Configuring …"
    # configure must be invoked from the build directory:
    # it runs `meson setup "$PWD" "$source_path"` internally.
    (
        cd "${BUILD_DIR}"
        "${SRC_DIR}/configure" \
            --prefix="${INSTALL_DIR}"   \
            --target-list=aarch64-softmmu \
            --enable-hvf                \
            --enable-cocoa              \
            --disable-gtk               \
            --disable-sdl               \
            --disable-curses            \
            --disable-vnc               \
            --disable-fuse              \
            --disable-docs              \
            --disable-guest-agent       \
            --disable-plugins           \
            --enable-tools              \
            --disable-install-blobs     \
            --enable-trace-backends=nop \
            --disable-werror            \
            --extra-cflags="-O2"
    )
else
    echo "==> Build already configured - skipping configure"
fi

# --------------------------------------------------------------------------
# 4. Build
# --------------------------------------------------------------------------

JOBS="${JOBS:-$(sysctl -n hw.logicalcpu 2>/dev/null || nproc)}"
echo "==> Building with ${JOBS} jobs …"
# `--enable-tools` configures a fleet of small helper tools (qemu-img,
# qemu-io, qemu-nbd, qemu-storage-daemon, qemu-edid, elf2dmp,
# ivshmem-client, ivshmem-server) for install; `meson install` aborts if
# any of them is missing, so build them all even though the .app only
# ships qemu-img.  They link fast, so the extra ninja work is negligible.
ninja -C "${BUILD_DIR}" -j"${JOBS}" \
    qemu-system-aarch64 \
    qemu-img qemu-io qemu-nbd storage-daemon/qemu-storage-daemon \
    qemu-edid contrib/elf2dmp/elf2dmp \
    contrib/ivshmem-client/ivshmem-client \
    contrib/ivshmem-server/ivshmem-server



# --------------------------------------------------------------------------
# 5. Install
# --------------------------------------------------------------------------

echo "==> Installing …"
# Use meson install --no-rebuild instead of `ninja install` - the latter
# pulls in test binaries (e.g. tests/audio/test-audio) that don't link
# virtio-snd.c.o and therefore can't resolve our coreaudio.m bypass refs.
# Meson is bundled inside the build dir's pyvenv (no system meson).
"${BUILD_DIR}/pyvenv/bin/meson" install -C "${BUILD_DIR}" --no-rebuild

echo ""
echo "Done. Binary at: ${INSTALL_DIR}/bin/qemu-system-aarch64"

# --------------------------------------------------------------------------
# 6. Build embedding dylib (libcdj3k-emu-qemu.dylib)
# --------------------------------------------------------------------------
#
# Same object files and link flags as the binary, but built as a -dynamiclib.
# The cdj3k_emu_shim.c entry point replaces main(); exit() is intercepted via
# Mach-O dylib interposing so cdj3k_emu_qemu_run() can return cleanly.
# Consumed by crates/cdj3k-emu-runtime via FFI.

echo "==> Building libcdj3k-emu-qemu.dylib …"

SHIM_SRC="${SHIM_DIR}/cdj3k_emu_shim.c"
SHIM_OBJ="${BUILD_DIR}/cdj3k_emu_shim.o"
DYLIB_OUT="${INSTALL_DIR}/lib/libcdj3k-emu-qemu.dylib"

# Include paths matching QEMU's own compilation (same as -I flags in build.ninja).
QEMU_CFLAGS="-O2 -I${BUILD_DIR} -I${SRC_DIR} -I${BUILD_DIR}/qapi -I${BUILD_DIR}/trace -I${BUILD_DIR}/ui"

cc ${QEMU_CFLAGS} -c -o "${SHIM_OBJ}" "${SHIM_SRC}"
echo "  shim compiled: ${SHIM_OBJ}"

# Extract the object-file list and LINK_ARGS from build.ninja, then re-link
# as a dylib. Entitlement symbol-list exports (@block.syms, @qemu.syms,
# -Wl,-exported_symbols_list,...) are stripped - they are binary-only.
NINJA="${BUILD_DIR}/build.ninja"

# 1. The (objc_|c_)LINKER build rule line. Prefer the unsigned target (some
#    configs skip the signing step). Master switched qemu-system-aarch64 to
#    objc_LINKER once coreaudio.m became part of the binary's direct link
#    (vs. v10.2.2 where it lived only inside libqemuaudio.a). The line can
#    be 50 KB+; awk handles it.
BUILD_LINE=$(awk '
    /^build qemu-system-aarch64-unsigned: (objc_|c_)LINKER / { print; found=1; exit }
    END { if (!found) exit 1 }
' "${NINJA}" || awk '
    /^build qemu-system-aarch64: (objc_|c_)LINKER / { print; exit }
' "${NINJA}")
if [ -z "${BUILD_LINE}" ]; then
    echo "ERROR: no (objc_|c_)LINKER rule for qemu-system-aarch64 in ${NINJA}" >&2
    exit 1
fi

# 2. Object/archive inputs: tokens ending in .o or .a, after the linker name.
#    Resolve relative paths to BUILD_DIR. Swap libcurl.a → libcurl.dylib
#    so we don't pull its transitive static deps (ngtcp2, nghttp3, ICU) into
#    the dylib link.
declare -a OBJS
seen_linker=0
for tok in ${BUILD_LINE}; do
    if [ "${seen_linker}" -eq 0 ]; then
        case "${tok}" in
            c_LINKER|objc_LINKER) seen_linker=1 ;;
        esac
        continue
    fi
    case "${tok}" in
        -*)         continue ;;
        *.o|*.a)    : ;;
        *)          continue ;;
    esac
    case "${tok}" in
        /*) full="${tok}" ;;
        *)  full="${BUILD_DIR}/${tok}" ;;
    esac
    [ -e "${full}" ] || continue
    if [ "$(basename "${full}")" = "libcurl.a" ]; then
        dylib="${full%/libcurl.a}/libcurl.dylib"
        [ -e "${dylib}" ] && full="${dylib}"
    fi
    OBJS+=("${full}")
done

# 3. The indented LINK_ARGS line that follows the build rule.
LINK_ARGS=$(awk '
    /^build qemu-system-aarch64(-unsigned)?: (objc_|c_)LINKER / { found=1; next }
    found && /^ +LINK_ARGS = / {
        sub(/^ +LINK_ARGS = /, "")
        print
        exit
    }
    found && /^[a-zA-Z]/ { exit }   # left the build rule block
' "${NINJA}")

# Strip binary-only flags. The exported_symbols_list path and @*.syms response
# files are entitlement gates we don't reproduce in the dylib.
LINK_ARGS=$(printf '%s' "${LINK_ARGS}" \
    | sed -E 's/-Wl,-exported_symbols_list,[^ ]+//g; s/@[^ ]+\.syms//g; s/  +/ /g')

echo "  linking ${#OBJS[@]} objects → ${DYLIB_OUT}"
(
    cd "${BUILD_DIR}"
    cc -dynamiclib \
       -install_name @rpath/libcdj3k-emu-qemu.dylib \
       -o "${DYLIB_OUT}" \
       "${SHIM_OBJ}" \
       "${OBJS[@]}" \
       ${LINK_ARGS}
)

echo ""
echo "Dylib at: ${DYLIB_OUT}"
