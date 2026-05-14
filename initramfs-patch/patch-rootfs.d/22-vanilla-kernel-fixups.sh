#!/bin/sh
# SPDX-License-Identifier: MIT OR Apache-2.0
# 22-vanilla-kernel-fixups.sh
#
# Replace Pioneer 4.4 kernel module artifacts with vanilla 6.6 LTS equivalents.
#
# On the vanilla kernel (linux 6.6 LTS / QEMU virt):
#   - virtio_blk, virtio_net, virtio_rng, virtio_mmio, virtio_console,
#     virtio_gpu (DRM_VIRTIO_GPU=y), virtio_snd (SND_VIRTIO=y) are ALL
#     built into the kernel image - no .ko loading needed.
#   - Only subucom_virt.ko, virtio_snd.ko, and udev_usb1.ko are out-of-tree and need insmod.
#   - Pioneer RK3399-specific services (fix-clock, set-affinity, usb-f-uac)
#     are masked - their sysfs paths don't exist on QEMU virt.
#
# vanilla-modules/ is expected at $PATCH_ASSETS_DIR/vanilla-modules/ -
#   staged by build-initramfs.sh (from build/docker-out/modules/)
#   and by bundle.sh (into Contents/Resources/patch/vanilla-modules/).
# dummy_drv.so is expected at $PATCH_ASSETS_DIR/dummy_drv.so -
#   staged by build-initramfs.sh and copied by bundle.sh.

set -eu

# ── 1. Replace /lib/modules with only vanilla out-of-tree modules ─────────────
echo "  [22] cleaning Pioneer 4.4 modules from /lib/modules ..."
# Remove all .ko files - the Pioneer rootfs ships 4.4.194 modules that can't
# load into a 6.6 kernel.
find "${ROOTFS}/lib/modules" -maxdepth 1 -name '*.ko' -delete 2>/dev/null || true

VANILLA_MODS="${PATCH_ASSETS_DIR}/vanilla-modules"
if [ -d "$VANILLA_MODS" ] && ls "$VANILLA_MODS"/*.ko >/dev/null 2>&1; then
    cp "$VANILLA_MODS"/*.ko "${ROOTFS}/lib/modules/"
    echo "  [22] installed vanilla modules: $(ls "$VANILLA_MODS"/*.ko | xargs -n1 basename | tr '\n' ' ')"
else
    echo "  [22] WARNING: no vanilla-modules at $VANILLA_MODS - /lib/modules will be empty"
fi

# ── 2. No-op insmod services for built-in drivers ────────────────────────────
# On vanilla 6.6 virtio_gpu/console/blk/rng are all built into the kernel (=y),
# and HW_RANDOM_VIRTIO auto-credits entropy without needing a userspace seeder.
# Replace the service files with /bin/true so ordering dependencies from other
# services resolve instantly. Symlinks live under multi-user.target.wants.
MULTI_USER_WANTS="${ROOTFS}/etc/systemd/system/multi-user.target.wants"
mkdir -p "$MULTI_USER_WANTS"
for SVC in virtio-gpu virtio-console virtio-blk virtio-rng; do
    SVC_PATH="${ROOTFS}/etc/systemd/system/insmod-${SVC}.service"
    cat > "$SVC_PATH" << EOF
[Unit]
Description=${SVC} driver built-in - no-op on vanilla 6.6 kernel
DefaultDependencies=no
Before=x11-only.service
Before=EP122.service
Before=multi-user.target

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/bin/true

[Install]
WantedBy=multi-user.target
EOF
    ln -sf "/etc/systemd/system/insmod-${SVC}.service" \
        "$MULTI_USER_WANTS/insmod-${SVC}.service"
    echo "  [22] insmod-${SVC}.service → no-op (driver built-in)"
done

# ── 3. Remove module autoload config for built-in drivers ────────────────────
# systemd-modules-load.service reads these and tries modprobe - all fail since
# the modules are built-in (modinfo won't find a .ko file).
rm -f "${ROOTFS}/etc/modules-load.d/virtio-early.conf"
rm -f "${ROOTFS}/etc/modprobe.d/virtio-force.conf"
echo "  [22] removed virtio-early.conf + virtio-force.conf"

# ── 4. Mask Pioneer RK3399-specific services ─────────────────────────────────
for SVC in fix-clock set-affinity usb-f-uac; do
    ln -sf /dev/null "${ROOTFS}/etc/systemd/system/${SVC}.service"
    echo "  [22] masked ${SVC}.service (RK3399-specific, no matching hw on QEMU virt)"
done

# ── 5. Install dummy_drv.so for headless Xorg ────────────────────────────────
# 25-xorg-headless.sh configures X to fall back to Driver "dummy" when
# /dev/dri/card0 is absent.  dummy_drv.so must be in the Xorg drivers dir.
# build-initramfs.sh stages it at $PATCH_ASSETS_DIR/dummy_drv.so;
# bundle.sh places it at Contents/Resources/patch/dummy_drv.so for the wizard.
DUMMY_SRC="${PATCH_ASSETS_DIR}/dummy_drv.so"
DUMMY_DST="${ROOTFS}/usr/lib/xorg/modules/drivers/dummy_drv.so"
if [ -f "$DUMMY_SRC" ]; then
    mkdir -p "$(dirname "$DUMMY_DST")"
    cp "$DUMMY_SRC" "$DUMMY_DST"
    chmod 755 "$DUMMY_DST"
    echo "  [22] installed dummy_drv.so → /usr/lib/xorg/modules/drivers/"
else
    echo "  [22] WARNING: dummy_drv.so not found at $DUMMY_SRC - headless Xorg will fail"
fi

