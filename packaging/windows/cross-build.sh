#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# cross-build.sh - build cdj3k-emu.exe for Windows on a Linux or macOS host.
#
#   packaging/windows/cross-build.sh x64|arm64
#
# Builds <arch>-pc-windows-gnullvm in the image of docker/Dockerfile.windows-qemu,
# the llvm-mingw toolchain QEMU's Windows build uses, and collects the DLLs the
# executable loads beside it.
#
# Output: dist/windows-bin-<arch>/ (cdj3k-emu.exe, libunwind.dll), the -Binary
# directory stage.ps1 takes.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

case "${1:-}" in
    x64)   TRIPLE=x86_64-pc-windows-gnullvm;  MINGW=x86_64-w64-mingw32 ;;
    arm64) TRIPLE=aarch64-pc-windows-gnullvm; MINGW=aarch64-w64-mingw32 ;;
    *) echo "Usage: $0 x64|arm64" >&2; exit 1 ;;
esac
ARCH="$1"

if [ -z "${CDJ3K_WINDOWS_CROSS_INNER:-}" ]; then
    IMAGE="${CDJ3K_WINDOWS_QEMU_IMAGE:-cdj3k-emu-windows-qemu}"
    command -v docker >/dev/null || { echo "ERROR: docker not found" >&2; exit 1; }
    echo "==> Image ${IMAGE}"
    docker build -q -f "${REPO_ROOT}/docker/Dockerfile.windows-qemu" -t "${IMAGE}" "${REPO_ROOT}" >/dev/null
    bash "${REPO_ROOT}/winit/fetch.sh"
    exec docker run --rm \
        --user "$(id -u):$(id -g)" \
        -e HOME=/tmp \
        -e CDJ3K_WINDOWS_CROSS_INNER=1 \
        -v "${REPO_ROOT}:/work" \
        "${IMAGE}" \
        bash packaging/windows/cross-build.sh "${ARCH}"
fi

# -- Inside the image ----------------------------------------------------------
CROSS="${REPO_ROOT}/target/windows-cross"
export CARGO_HOME="${CROSS}/cargo-home"
export CARGO_TARGET_DIR="${CROSS}/target"
upper="$(printf '%s' "${TRIPLE}" | tr 'a-z-' 'A-Z_')"
export "CARGO_TARGET_${upper}_LINKER=${MINGW}-clang"
export "CC_${TRIPLE//-/_}=${MINGW}-clang"
export "CXX_${TRIPLE//-/_}=${MINGW}-clang++"
export "AR_${TRIPLE//-/_}=llvm-ar"

echo "==> cargo build --release --target ${TRIPLE}"
cargo build --locked --release -p cdj3k-emu --target "${TRIPLE}"

OUT="${REPO_ROOT}/dist/windows-bin-${ARCH}"
rm -rf "${OUT}"
mkdir -p "${OUT}"
cp "${CARGO_TARGET_DIR}/${TRIPLE}/release/cdj3k-emu.exe" "${OUT}/"
bash "${REPO_ROOT}/scripts/bundle-dlls.sh" "${OUT}" "/opt/llvm-mingw/${MINGW}/bin" -- "${OUT}/cdj3k-emu.exe"
echo "==> ${OUT}"
ls -l "${OUT}"
