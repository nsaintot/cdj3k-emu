#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# build.sh
#
# Build all guest artifacts (the Linux 6.6 LTS kernel, the out-of-tree modules
# and the tools), install them into the Pioneer rootfs, apply the patches and
# repack the initramfs. Run it after changing a module, a patch script or the
# kernel config.
#
# Artifacts (see docker/Dockerfile):
#   Image              - the 6.6 LTS kernel, with PL011 and virtio built in
#   subucom_virt.ko    - the virtual sub-CPU device (/dev/subucom_spi1.0)
#   virtio_snd.ko      - the project's virtio-sound PCM driver
#   udev_usb1.ko       - Pioneer's media event channels in /proc
#   dummy_drv.so       - the Xorg dummy video driver (headless mode)
#   deck_shim.so       - the LD_PRELOAD shim (core/ and per-model code)
#   subucom_forwarder_aarch64, subucom_live_aarch64, cfgd_aarch64,
#   pc_link_bridge_aarch64
#   dropbear_aarch64   - built by dropbear/build.sh
#
# Outputs:
#   build/Image
#   build/initramfs-patched.cpio.gz
#
# --artifacts-only stops after the Docker build: build/Image, build/docker-out/
# and guest/out/, which is everything the packaging stagers take.
# It needs no Pioneer rootfs.
#
# Prerequisites:
#   docker with BuildKit support (Docker Desktop / OrbStack)

set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")" && pwd)"

CLEAN_AFTER_BUILD=0
ARTIFACTS_ONLY=0
# SSH access (dropbear on port 2222) is for development only and is off by
# default in shipping builds: the SSH steps of the rootfs patches
# (02-dropbear-install, 03-dropbear-enable, 04-root-password,
# 05-serial-autologin) do nothing unless this is set. Enabling SSH leaves a
# passwordless root login on the QEMU host-forwarded port 2222, so enable it
# only on a development machine.
ENABLE_SSH=0
for arg in "$@"; do
    case "$arg" in
        --clean)      CLEAN_AFTER_BUILD=1 ;;
        --enable-ssh) ENABLE_SSH=1 ;;
        --artifacts-only) ARTIFACTS_ONLY=1 ;;
        -h|--help)
            echo "Usage: ./build.sh [--clean] [--enable-ssh] [--artifacts-only]"
            echo "  --clean            remove the unpacked rootfs workspace after the build"
            echo "  --enable-ssh       enable dropbear and passwordless root (development only)"
            echo "  --artifacts-only   build the kernel, modules and guest tools only; no initramfs"
            exit 0
            ;;
        *)
            echo "ERROR: unknown argument: $arg" >&2
            echo "Usage: ./build.sh [--clean] [--enable-ssh] [--artifacts-only]" >&2
            exit 1
            ;;
    esac
done
export ENABLE_SSH

DOCKER_OUT="$REPO_ROOT/build/docker-out"
# DOCKER_CACHE is a persistent BuildKit cache on disk. OrbStack's default
# builder cache is too small for the kernel build tree, so GC evicts layers
# between runs even with an unchanged Dockerfile. mode=max also keeps the
# intermediate layers. This needs a docker-container buildx builder (see
# docker/ensure-buildx-builder.sh), because the default "docker" driver cannot
# use --cache-to type=local. CDJ3K_DOCKER_CACHE overrides the directory; `gha`
# puts the layers in the GitHub Actions cache instead.
DOCKER_CACHE="${CDJ3K_DOCKER_CACHE-$REPO_ROOT/build/docker-cache}"
DOCKER_BUILDER="$("$REPO_ROOT/docker/ensure-buildx-builder.sh")"
WORKDIR="$REPO_ROOT/build/work"
ROOTFS_DIR="$WORKDIR/rootfs"
ROOTFS_MODULES="$ROOTFS_DIR/lib/modules"
INITRAMFS_ORIG="$REPO_ROOT/build/initramfs-work/initramfs.cpio.gz"

echo "================================================================"
echo "  cdj3k-emu guest build (Linux 6.6 LTS)"
echo "================================================================"
echo ""

if [[ "$ARTIFACTS_ONLY" -eq 0 && ! -f "$INITRAMFS_ORIG" ]]; then
    echo "ERROR: source initramfs not found: $INITRAMFS_ORIG" >&2
    exit 1
fi

# [1/5] Build the kernel, the modules and the tools in Docker.
echo "[1/5] Building the artifacts (docker/Dockerfile, target artifacts)..."
rm -rf "$DOCKER_OUT"
if [[ "$DOCKER_CACHE" == gha ]]; then
    CACHE_ARGS=(--cache-from "type=gha,scope=guest"
                --cache-to "type=gha,scope=guest,mode=max")
else
    mkdir -p "$DOCKER_CACHE"
    CACHE_ARGS=(--cache-from "type=local,src=$DOCKER_CACHE"
                --cache-to "type=local,dest=$DOCKER_CACHE,mode=max")
fi
docker buildx build \
    --builder "$DOCKER_BUILDER" \
    --platform linux/arm64 \
    --provenance=false \
    --target artifacts \
    "${CACHE_ARGS[@]}" \
    --output "type=local,dest=$DOCKER_OUT" \
    -f "$REPO_ROOT/docker/Dockerfile" \
    "$REPO_ROOT"
echo ""

cp "$DOCKER_OUT/Image" "$REPO_ROOT/build/Image"
echo "  ✓  Copied Image to build/Image"

# Save the tools to guest/out/ for the packaging stagers.
mkdir -p "$REPO_ROOT/guest/out"
# All of these are required: packaging/macos/stage.sh refuses a bundle without
# cfgd or pc_link_bridge, and the patch scripts abort the rootfs provisioning
# when a tool they install is missing. Copying them here makes an incomplete
# build fail at once.
for bin in deck_shim.so subucom_forwarder_aarch64 subucom_live_aarch64 cfgd_aarch64 \
           pc_link_bridge_aarch64; do
    cp "$DOCKER_OUT/$bin" "$REPO_ROOT/guest/out/$bin"
done

# Build dropbear for firmware that ships no SSH server, such as the CDJ-1500X.
# Like qemu/, it has its own clone and build script.
bash "$REPO_ROOT/dropbear/build.sh"
cp "$REPO_ROOT/dropbear/out/dropbear_aarch64" "$REPO_ROOT/dropbear/out/dropbear_aarch64.LICENSE" \
    "$REPO_ROOT/guest/out/"

if [[ "$ARTIFACTS_ONLY" -eq 1 ]]; then
    echo ""
    echo "  Artifacts: build/Image, build/docker-out/, guest/out/"
    exit 0
fi

# [2/5] Restore the Pioneer rootfs.
echo "[2/5] Restoring rootfs from: $INITRAMFS_ORIG"
rm -rf "$ROOTFS_DIR"
mkdir -p "$ROOTFS_DIR"
(
    cd "$ROOTFS_DIR"
    { gzip -dc "$INITRAMFS_ORIG" 2>/dev/null || true; } | cpio -i --make-directories 2>/dev/null
)
mkdir -p "$ROOTFS_MODULES"
echo "      Restored the rootfs at $ROOTFS_DIR"
echo ""

# [3/5] Stage the modules where patch script 22 finds them.
echo "[3/5] Staging the out-of-tree modules..."
VANILLA_MODS_STAGE="$REPO_ROOT/initramfs-patch/vanilla-modules"
rm -rf "$VANILLA_MODS_STAGE"
mkdir -p "$VANILLA_MODS_STAGE"
for ko in subucom_virt.ko virtio_snd.ko udev_usb1.ko; do
    cp "$DOCKER_OUT/modules/$ko" "$VANILLA_MODS_STAGE/"
    echo "  ✓  staged $ko"
done
cp "$DOCKER_OUT/dummy_drv.so" "$REPO_ROOT/initramfs-patch/dummy_drv.so"
echo "  ✓  staged dummy_drv.so"

# Install the shared tools into the rootfs. The Docker artifacts carry an
# `_aarch64` suffix; strip it on install, because the services use the bare
# name (subucom-forwarder.service runs /usr/bin/subucom_forwarder). The .app
# path in packaging/macos/stage.sh strips it the same way.
mkdir -p "$ROOTFS_DIR/usr/bin"
for tool in subucom_live subucom_forwarder; do
    cp "$DOCKER_OUT/${tool}_aarch64" "$ROOTFS_DIR/usr/bin/$tool"
    chmod 755 "$ROOTFS_DIR/usr/bin/$tool"
done

mkdir -p "$ROOTFS_DIR/home/root"
cp "$DOCKER_OUT/deck_shim.so" "$ROOTFS_DIR/home/root/deck_shim.so"
chmod 755 "$ROOTFS_DIR/home/root/deck_shim.so"


USB_IMG_SRC="$REPO_ROOT/build/usb.img"
if [[ -f "$USB_IMG_SRC" ]]; then
    mkdir -p "$ROOTFS_DIR/opt"
    cp "$USB_IMG_SRC" "$ROOTFS_DIR/opt/usb.img"
    echo "  ✓  Copied usb.img to /opt/usb.img"
fi
echo ""

# [4/5] Apply the rootfs patches.
echo "[4/5] Applying the rootfs patches..."
"$REPO_ROOT/initramfs-patch/patch-rootfs.sh" "$ROOTFS_DIR"
rm -rf "$VANILLA_MODS_STAGE"
rm -f "$REPO_ROOT/initramfs-patch/dummy_drv.so"
echo ""

# [5/5] Repack the initramfs.
echo "[5/5] Repacking the initramfs in Docker..."
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
