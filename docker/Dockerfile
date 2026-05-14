# syntax=docker/dockerfile:1
#
# CDJ-3000 QEMU guest build - Linux 6.6 LTS, aarch64/virt target.
# Runs natively on arm64 (Apple Silicon / OrbStack).
#
# Produces all guest artifacts in /out/:
#   Image              - kernel image (PL011 console, virtio built-in)
#   modules/           - out-of-tree kernel modules (.ko)
#   ep122_shim.so      - LD_PRELOAD shim (glibc 2.17-compatible)
#   subucom_forwarder_aarch64  subucom_live_aarch64  cfgd_aarch64 - static tools
#   dummy_drv.so       - Xorg Video Driver ABI 24.0 for headless mode
#
# Single source of truth for guest user-space build flags is `guest/Makefile`.
# Two stages invoke it with different toolchains:
#   builder       (alpine, musl gcc)  → static tools (subucom_*, cfgd) + kernel + modules
#   shim-builder  (ubuntu, glibc gcc) → ep122_shim.so (must be glibc-linked
#                                       for LD_PRELOAD into EP122)

FROM arm64v8/alpine:3.19 AS builder

RUN apk add --no-cache \
    build-base \
    bc \
    flex \
    bison \
    perl \
    openssl-dev \
    lz4-dev \
    lz4-static \
    libdrm-dev \
    bash \
    wget \
    xz \
    git \
    file \
    python3 \
    rsync

# Clone Linux 6.6 LTS (shallow).  Pinned to a specific point release so
# the build is reproducible: re-running the same Dockerfile months apart
# produces the same kernel.  Bump deliberately to pick up CVE fixes.
ARG LINUX_TAG=v6.6.138
RUN git clone --depth=1 --branch ${LINUX_TAG} \
    https://git.kernel.org/pub/scm/linux/kernel/git/stable/linux.git \
    /kernel 2>&1 | tail -3

# Apply our Linux 6.6 kernel patches.  Each modification (Pioneer virtio_blk
# naming, synthetic DSI-2 jog output, dummy MIN_PERIOD_SIZE bump, virtgpu_kms
# VERSION_1 guard drop) lives as a unified-diff patch in guest/kernel-patches/.
# A failure to apply means upstream Linux drifted under our anchors - the
# build aborts here, signalling that the patch needs regeneration.
#
# The .config is the only non-patch artefact and is copied separately.
COPY guest/kernel-patches/ /tmp/kernel-patches/
RUN cp /tmp/kernel-patches/cdj3k-emu-kernel.config /kernel/.config && \
    cd /kernel && \
    for p in /tmp/kernel-patches/*.patch; do \
        echo "    [apply] $(basename $p)"; \
        git apply --check "$p" || { echo "FAIL: $p"; exit 1; }; \
        git apply "$p"; \
    done && \
    rm -rf /tmp/kernel-patches

RUN cd /kernel && make ARCH=arm64 olddefconfig 2>&1 | tail -3

# Build kernel Image
RUN cd /kernel && make ARCH=arm64 -j$(nproc) Image 2>&1 | tail -5

# Prepare kernel headers for out-of-tree module builds
RUN cd /kernel && make ARCH=arm64 -j$(nproc) modules_prepare 2>&1 | tail -3

# arm64 defconfig has many =m drivers; building them all just to get Module.symvers
# is too slow.  Our modules only use core kernel symbols (_printk, spinlocks,
# cdev/class, ALSA PCM) that are unconditionally present in any 6.x kernel and
# CONFIG_MODVERSIONS is off in defconfig, so version CRC mismatches don't cause
# insmod failures at runtime.  KBUILD_MODPOST_WARN turns the "undefined symbol"
# modpost errors into warnings and lets the build produce usable .ko files.

# subucom_virt - virtual /dev/subucom_spi1.0 (CDJ-3000 SPI emulation)
COPY guest/modules/subucom_virt/ /subucom_virt/
RUN cd /kernel && make ARCH=arm64 -j$(nproc) KBUILD_MODPOST_WARN=1 M=/subucom_virt modules

# virtio_snd - custom virtio-sound PCM playback (out-of-tree, replaces SND_VIRTIO=n upstream)
COPY guest/modules/virtio_snd/ /virtio_snd/
RUN cd /kernel && make ARCH=arm64 -j$(nproc) KBUILD_MODPOST_WARN=1 M=/virtio_snd modules

# udev_usb1 - /proc/udev_usb1 shim (EP122 USB event channel, absent from vanilla kernel)
COPY guest/modules/udev_usb1/ /udev_usb1/
RUN cd /kernel && make ARCH=arm64 -j$(nproc) KBUILD_MODPOST_WARN=1 M=/udev_usb1 modules

# dummy_drv.so - Xorg Video Driver ABI 24.0 for headless mode.
# Ubuntu 20.04 focal arm64 ships Xorg 1.20.9 (ABI 24.0), matching Pioneer Xorg 1.20.4.
# Fetched over HTTPS and SHA-256 verified so a substituted upstream package
# can't silently end up in the bundle.
ARG DUMMY_DRV_DEB_SHA256=866876b535aedba516f3495c0b8bdf57abf3e65cf4351e38a45789973081ceab
RUN wget -q \
        "https://ports.ubuntu.com/ubuntu-ports/pool/main/x/xserver-xorg-video-dummy/xserver-xorg-video-dummy_0.3.8-1build3_arm64.deb" \
         -O /tmp/dummy.deb && \
    echo "${DUMMY_DRV_DEB_SHA256}  /tmp/dummy.deb" | sha256sum -c - && \
    cd /tmp && mkdir -p dummy_extract && cd dummy_extract && \
    ar x /tmp/dummy.deb && \
    tar -xJf data.tar.xz ./usr/lib/xorg/modules/drivers/dummy_drv.so && \
    ls -la /tmp/dummy_extract/usr/lib/xorg/modules/drivers/dummy_drv.so

# Static guest tools — driven by guest/Makefile so flags stay in one place.
# Alpine arm64v8's plain `gcc` is the musl-libc cross compiler we want for
# fully-static aarch64 binaries that don't depend on glibc.
COPY guest/ /tmp/guest/
RUN make -C /tmp/guest CC=gcc tools

# Collect everything except the shim into /out/. The shim is built in the
# glibc stage below because it has to be LD_PRELOAD-compatible with EP122
# (a Pioneer aarch64 binary dynamically linked against glibc 2.17). A
# musl-linked .so would NEEDED `libc.so` instead of `libc.so.6` and the
# glibc dynamic linker would refuse to load it.
RUN mkdir -p /out/modules && \
    cp /kernel/arch/arm64/boot/Image                                 /out/Image && \
    cp /subucom_virt/subucom_virt.ko                                 /out/modules/ && \
    cp /virtio_snd/virtio_snd.ko                                     /out/modules/ && \
    cp /udev_usb1/udev_usb1.ko                                       /out/modules/ && \
    cp /tmp/dummy_extract/usr/lib/xorg/modules/drivers/dummy_drv.so /out/ && \
    cp /tmp/guest/out/subucom_forwarder_aarch64                      /out/ && \
    cp /tmp/guest/out/subucom_live_aarch64                           /out/ && \
    cp /tmp/guest/out/cfgd_aarch64                                   /out/ && \
    echo "=== /out/ ===" && ls -la /out/ && echo "=== /out/modules/ ===" && ls -la /out/modules/

# ep122_shim.so — built against glibc 2.27 (Ubuntu 18.04) so it can
# LD_PRELOAD into Pioneer's glibc 2.17 EP122 process. Alpine's musl gcc
# produces a `.so` that lists `libc.so` (musl) in NEEDED; glibc's ld.so
# searches for `libc.so.6` and fails with
# `error while loading shared libraries: libc.so: cannot open shared
# object file`. Same Makefile, different toolchain.
FROM ubuntu:18.04 AS shim-builder
RUN apt-get update -qq && \
    apt-get install -y --no-install-recommends gcc libc6-dev make file && \
    rm -rf /var/lib/apt/lists/*
COPY guest/ /tmp/guest/
RUN make -C /tmp/guest CC=gcc shim

FROM scratch AS artifacts
COPY --from=builder      /out/                         /
COPY --from=shim-builder /tmp/guest/out/ep122_shim.so  /ep122_shim.so
