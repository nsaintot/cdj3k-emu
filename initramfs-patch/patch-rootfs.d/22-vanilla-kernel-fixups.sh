#!/bin/sh
# SPDX-License-Identifier: MIT OR Apache-2.0
# 22-vanilla-kernel-fixups.sh
#
# Replace the Pioneer 4.4 kernel modules with their vanilla 6.6 LTS
# equivalents.
#
# On the vanilla kernel (Linux 6.6 LTS on QEMU virt):
#   - virtio_blk, virtio_net, virtio_rng, virtio_mmio, virtio_console and
#     virtio_gpu (DRM_VIRTIO_GPU=y) are built into the kernel image, so no
#     .ko needs loading. The in-tree virtio-sound driver (SND_VIRTIO) is off.
#   - subucom_virt.ko, virtio_snd.ko and udev_usb1.ko are out-of-tree modules
#     and need insmod.
#   - The Pioneer RK3399 services (fix-clock, set-affinity, usb-f-uac) are
#     masked, because their sysfs paths do not exist on QEMU virt.
#
# vanilla-modules/ is expected at $PATCH_ASSETS_DIR/vanilla-modules/.
#   build.sh stages it from build/docker-out/modules/, and
#   packaging/macos/stage.sh copies it into Contents/Resources/patch/vanilla-modules/.
# dummy_drv.so is expected at $PATCH_ASSETS_DIR/dummy_drv.so.
#   build.sh stages it there, and packaging/macos/stage.sh copies it.

set -eu
: "${APP_UNIT:?APP_UNIT must be set by dispatcher}"

# ── 1. Replace /lib/modules with only vanilla out-of-tree modules ─────────────
echo "  [22] removing the Pioneer 4.4 modules from /lib/modules ..."
# Remove every .ko file: the Pioneer rootfs ships 4.4.194 modules, which
# cannot load into a 6.6 kernel.
mkdir -p "${ROOTFS}/lib/modules"
find "${ROOTFS}/lib/modules" -maxdepth 1 -name '*.ko' -delete 2>/dev/null || true

VANILLA_MODS="${PATCH_ASSETS_DIR}/vanilla-modules"
if [ -d "$VANILLA_MODS" ] && ls "$VANILLA_MODS"/*.ko >/dev/null 2>&1; then
    cp "$VANILLA_MODS"/*.ko "${ROOTFS}/lib/modules/"
    echo "  [22] installed vanilla modules: $(ls "$VANILLA_MODS"/*.ko | xargs -n1 basename | tr '\n' ' ')"
else
    echo "  [22] WARNING: no vanilla-modules at $VANILLA_MODS; /lib/modules will be empty"
fi

# ── 2. No-op insmod services for built-in drivers ────────────────────────────
# On vanilla 6.6, virtio_gpu, virtio_console, virtio_blk and virtio_rng are
# built into the kernel, and HW_RANDOM_VIRTIO credits entropy without a
# user-space seeder. Replace the service files with /bin/true, so the ordering
# dependencies of other services resolve at once. The symlinks live under
# multi-user.target.wants.
MULTI_USER_WANTS="${ROOTFS}/etc/systemd/system/multi-user.target.wants"
mkdir -p "$MULTI_USER_WANTS"
for SVC in virtio-gpu virtio-console virtio-blk virtio-rng; do
    SVC_PATH="${ROOTFS}/etc/systemd/system/insmod-${SVC}.service"
    cat > "$SVC_PATH" << EOF
[Unit]
Description=${SVC} driver is built in; nothing to load on the vanilla 6.6 kernel
DefaultDependencies=no
Before=x11-only.service
Before=${APP_UNIT}
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
    echo "  [22] insmod-${SVC}.service now does nothing (the driver is built in)"
done

# ── 3. Remove module autoload config for built-in drivers ────────────────────
# systemd-modules-load.service reads these and runs modprobe, which fails for
# every built-in module because there is no .ko file.
rm -f "${ROOTFS}/etc/modules-load.d/virtio-early.conf"
rm -f "${ROOTFS}/etc/modprobe.d/virtio-force.conf"
echo "  [22] removed virtio-early.conf and virtio-force.conf"

# ── 4. Mask Pioneer RK3399-specific services ─────────────────────────────────
for SVC in fix-clock set-affinity usb-f-uac; do
    ln -sf /dev/null "${ROOTFS}/etc/systemd/system/${SVC}.service"
    echo "  [22] masked ${SVC}.service (RK3399-specific; QEMU virt has no matching hardware)"
done

# ── 5. Install dummy_drv.so for headless Xorg ────────────────────────────────
# 25-xorg-headless.sh configures X to fall back to Driver "dummy" when
# /dev/dri/card0 is absent, so dummy_drv.so must be in the Xorg drivers
# directory. build.sh stages it at $PATCH_ASSETS_DIR/dummy_drv.so;
# packaging/macos/stage.sh places it at Contents/Resources/patch/dummy_drv.so.
DUMMY_SRC="${PATCH_ASSETS_DIR}/dummy_drv.so"
DUMMY_DST="${ROOTFS}/usr/lib/xorg/modules/drivers/dummy_drv.so"
if [ -f "$DUMMY_SRC" ]; then
    mkdir -p "$(dirname "$DUMMY_DST")"
    cp "$DUMMY_SRC" "$DUMMY_DST"
    chmod 755 "$DUMMY_DST"
    echo "  [22] installed dummy_drv.so in /usr/lib/xorg/modules/drivers/"
else
    echo "  [22] WARNING: dummy_drv.so not found at $DUMMY_SRC; headless Xorg will fail"
fi

