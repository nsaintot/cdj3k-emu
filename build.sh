#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# build.sh
#
# Build all guest artifacts (Linux 6.6 LTS kernel + out-of-tree modules + tools),
# install them into the Pioneer rootfs, apply patches, and repack the initramfs.
# Run this whenever you change a module, patch script, or kernel config.
#
# Artifacts built (see docker/Dockerfile):
#   Image              - 6.6 LTS kernel (PL011, virtio built-in)
#   subucom_virt.ko    - virtual /dev/subucom_spi1.0
#   virtio_snd.ko      - custom virtio-sound PCM
#   dummy_drv.so       - Xorg dummy video driver (headless mode)
#   ep122_shim.so      - LD_PRELOAD shim for EP122
#   subucom_forwarder_aarch64 / subucom_live_aarch64 / cfgd_aarch64
#
# Outputs:
#   build/Image
#   build/initramfs-patched.cpio.gz
#
# Prerequisites:
#   docker with BuildKit support (Docker Desktop / OrbStack)

set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")" && pwd)"

CLEAN_AFTER_BUILD=0
# SSH access (dropbear on port 2222) is dev-only.  Off by default for shipping
# builds: the SSH-related rootfs patches (02-dropbear-key, 03-dropbear-enable,
# 04-root-password) are no-ops unless this is set.  Enabling SSH leaves a
# **passwordless** root login bound to the QEMU host-forwarded port 2222 -
# fine on a dev box, hostile elsewhere.
ENABLE_SSH=0
for arg in "$@"; do
    case "$arg" in
        --clean)      CLEAN_AFTER_BUILD=1 ;;
        --enable-ssh) ENABLE_SSH=1 ;;
        -h|--help)
            echo "Usage: ./build.sh [--clean] [--enable-ssh]"
            echo "  --clean        remove unpacked rootfs workspace after build"
            echo "  --enable-ssh   enable dropbear + passwordless root (dev only)"
            exit 0
            ;;
        *)
            echo "ERROR: unknown argument: $arg" >&2
            echo "Usage: ./build.sh [--clean] [--enable-ssh]" >&2
            exit 1
            ;;
    esac
done
export ENABLE_SSH

DOCKER_OUT="$REPO_ROOT/build/docker-out"
WORKDIR="$REPO_ROOT/build/work"
ROOTFS_DIR="$WORKDIR/rootfs"
ROOTFS_MODULES="$ROOTFS_DIR/lib/modules"
INITRAMFS_ORIG="$REPO_ROOT/build/initramfs-work/initramfs.cpio.gz"

echo "================================================================"
echo "  CDJ-3000 build (Linux 6.6 LTS)"
echo "================================================================"
echo ""

# Ensure Pioneer rootfs source exists
if [[ ! -f "$INITRAMFS_ORIG" ]]; then
    echo "ERROR: source initramfs not found: $INITRAMFS_ORIG" >&2
    exit 1
fi

# [1/5] Build kernel + modules + tools via Docker
echo "[1/5] Building artifacts (docker/Dockerfile - target artifacts)..."
rm -rf "$DOCKER_OUT"
docker buildx build \
    --platform linux/arm64 \
    --target artifacts \
    --output "type=local,dest=$DOCKER_OUT" \
    -f "$REPO_ROOT/docker/Dockerfile" \
    "$REPO_ROOT"
echo ""

# Copy kernel image to canonical path
cp "$DOCKER_OUT/Image" "$REPO_ROOT/build/Image"
echo "  ✓  Image → build/Image"

# [2/5] Restore Pioneer rootfs
echo "[2/5] Restoring rootfs from: $INITRAMFS_ORIG"
rm -rf "$ROOTFS_DIR"
mkdir -p "$ROOTFS_DIR"
(
    cd "$ROOTFS_DIR"
    { gzip -dc "$INITRAMFS_ORIG" 2>/dev/null || true; } | cpio -i --make-directories 2>/dev/null
)
mkdir -p "$ROOTFS_MODULES"
echo "      rootfs restored at $ROOTFS_DIR"
echo ""

# [3/5] Stage modules where patch script 22 can find them
echo "[3/5] Staging out-of-tree modules..."
VANILLA_MODS_STAGE="$REPO_ROOT/initramfs-patch/vanilla-modules"
rm -rf "$VANILLA_MODS_STAGE"
mkdir -p "$VANILLA_MODS_STAGE"
for ko in subucom_virt.ko virtio_snd.ko udev_usb1.ko; do
    cp "$DOCKER_OUT/modules/$ko" "$VANILLA_MODS_STAGE/"
    echo "  ✓  staged $ko"
done
cp "$DOCKER_OUT/dummy_drv.so" "$REPO_ROOT/initramfs-patch/dummy_drv.so"
echo "  ✓  staged dummy_drv.so"

# Install shared tools into rootfs
mkdir -p "$ROOTFS_DIR/usr/bin"
for tool in \
    "subucom_live_aarch64:usr/bin" \
    "subucom_forwarder_aarch64:usr/bin" \
; do
    src="${tool%%:*}"
    dst="${tool##*:}"
    cp "$DOCKER_OUT/$src" "$ROOTFS_DIR/$dst"
    chmod 755 "$ROOTFS_DIR/$dst"
done

mkdir -p "$ROOTFS_DIR/home/root"
cp "$DOCKER_OUT/ep122_shim.so" "$ROOTFS_DIR/home/root/ep122_shim.so"
chmod 755 "$ROOTFS_DIR/home/root/ep122_shim.so"

# Save tools to guest/out/ for bundle.sh
mkdir -p "$REPO_ROOT/guest/out"
for bin in ep122_shim.so subucom_forwarder_aarch64 subucom_live_aarch64 cfgd_aarch64; do
    cp "$DOCKER_OUT/$bin" "$REPO_ROOT/guest/out/$bin" 2>/dev/null || true
done

USB_IMG_SRC="$REPO_ROOT/build/usb.img"
if [[ -f "$USB_IMG_SRC" ]]; then
    mkdir -p "$ROOTFS_DIR/opt"
    cp "$USB_IMG_SRC" "$ROOTFS_DIR/opt/usb.img"
    echo "  ✓  usb.img → /opt/usb.img"
fi
echo ""

# [4/5] Apply rootfs patches
echo "[4/5] Applying rootfs patches..."
"$REPO_ROOT/initramfs-patch/patch-rootfs.sh" "$ROOTFS_DIR"
rm -rf "$VANILLA_MODS_STAGE"
rm -f "$REPO_ROOT/initramfs-patch/dummy_drv.so"
echo ""

# [5/5] Repack initramfs
echo "[5/5] Repacking initramfs via Docker..."
docker run --rm \
    -v "$WORKDIR":/work \
    -v "$REPO_ROOT/initramfs-patch/repack.sh":/work/repack.sh:ro \
    arm64v8/alpine:3.19 \
    sh /work/repack.sh
mv "$WORKDIR/initramfs-patched.cpio.gz" "$REPO_ROOT/build/initramfs-patched.cpio.gz"

echo ""
echo "================================================================"
echo "  Build complete."
echo ""
echo "  Kernel:    build/Image"
echo "  Initramfs: build/initramfs-patched.cpio.gz"
echo "================================================================"

if [[ "$CLEAN_AFTER_BUILD" -eq 1 ]]; then
    rm -rf "$WORKDIR/rootfs"
    echo "  Workspace cleaned."
fi
