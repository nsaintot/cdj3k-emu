# syntax=docker/dockerfile:1
#
# Guest build for the emulated decks: Linux 6.6 LTS for the aarch64 virt
# machine. It runs natively on arm64 hosts.
#
# The build writes every guest artefact to /out/:
#   Image              - the kernel image (PL011 console, virtio built in)
#   modules/           - the out-of-tree kernel modules (.ko)
#   deck_shim.so       - the LD_PRELOAD shim (compatible with glibc 2.17)
#   subucom_forwarder_aarch64, subucom_live_aarch64, cfgd_aarch64,
#   pc_link_bridge_aarch64 - the static guest tools
#   dummy_drv.so       - the Xorg dummy video driver (ABI 24.0) for headless mode
#
# `guest/Makefile` holds the build flags for the guest user-space code. Two
# stages run it with different toolchains:
#   - builder (Alpine, musl gcc) builds the static tools (subucom_*, cfgd, ...),
#     the kernel and the modules.
#   - shim-builder (Ubuntu, glibc gcc) builds deck_shim.so, which must link
#     against glibc to be preloaded into EP122; `make abi-check` enforces this.

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

# Clone Linux 6.6 LTS (shallow). The tag pins a point release so the build is
# reproducible; bump it to pick up security fixes.
ARG LINUX_TAG=v6.6.138
RUN git clone --depth=1 --branch ${LINUX_TAG} \
    https://git.kernel.org/pub/scm/linux/kernel/git/stable/linux.git \
    /kernel 2>&1 | tail -3

# Apply the kernel patches in guest/kernel-patches/: Pioneer's virtio_blk
# naming, a synthetic DSI-2 output for the jog LCD, a larger MIN_PERIOD_SIZE in
# the dummy sound driver, and the removal of the VERSION_1 guard in
# virtgpu_kms. When a patch no longer applies, the build stops here and the
# patch must be regenerated against the new kernel.
#
# The .config in the same directory is copied as is.
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

RUN cd /kernel && make ARCH=arm64 -j$(nproc) Image 2>&1 | tail -5

# Prepare the kernel headers for the out-of-tree module builds.
RUN cd /kernel && make ARCH=arm64 -j$(nproc) modules_prepare 2>&1 | tail -3

# The kernel config has many =m drivers, and building them all only to get
# Module.symvers is too slow. The out-of-tree modules use only core kernel
# symbols (_printk, spinlocks, cdev/class, ALSA PCM) that every 6.x kernel has,
# and CONFIG_MODVERSIONS is off, so insmod does not check symbol CRCs.
# KBUILD_MODPOST_WARN turns the "undefined symbol" modpost errors into warnings
# so the build still produces usable .ko files.

# subucom_virt provides a virtual /dev/subucom_spi1.0 for the panel SPI link.
COPY guest/modules/subucom_virt/ /subucom_virt/
RUN cd /kernel && make ARCH=arm64 -j$(nproc) KBUILD_MODPOST_WARN=1 M=/subucom_virt modules

# virtio_snd is a custom virtio-sound PCM playback driver. It is built out of
# tree in place of the upstream driver, which the config disables (SND_VIRTIO=n).
COPY guest/modules/virtio_snd/ /virtio_snd/
RUN cd /kernel && make ARCH=arm64 -j$(nproc) KBUILD_MODPOST_WARN=1 M=/virtio_snd modules

# udev_usb1 provides /proc/udev_usb1, the USB event channel that EP122 reads
# and that the vanilla kernel lacks.
COPY guest/modules/udev_usb1/ /udev_usb1/
RUN cd /kernel && make ARCH=arm64 -j$(nproc) KBUILD_MODPOST_WARN=1 M=/udev_usb1 modules

# dummy_drv.so is the Xorg dummy video driver (ABI 24.0) for headless mode.
# Ubuntu 20.04 (focal) arm64 ships Xorg 1.20.9, whose ABI 24.0 matches Pioneer's
# Xorg 1.20.4. The package is fetched over HTTPS and its SHA-256 is checked, so
# a substituted package cannot reach the bundle.
ARG DUMMY_DRV_DEB_SHA256=866876b535aedba516f3495c0b8bdf57abf3e65cf4351e38a45789973081ceab
RUN wget -q \
        "https://ports.ubuntu.com/ubuntu-ports/pool/main/x/xserver-xorg-video-dummy/xserver-xorg-video-dummy_0.3.8-1build3_arm64.deb" \
         -O /tmp/dummy.deb && \
    echo "${DUMMY_DRV_DEB_SHA256}  /tmp/dummy.deb" | sha256sum -c - && \
    cd /tmp && mkdir -p dummy_extract && cd dummy_extract && \
    ar x /tmp/dummy.deb && \
    tar -xJf data.tar.xz ./usr/lib/xorg/modules/drivers/dummy_drv.so && \
    ls -la /tmp/dummy_extract/usr/lib/xorg/modules/drivers/dummy_drv.so

# Build the static guest tools through guest/Makefile so the flags stay in one
# place. Alpine arm64v8's plain `gcc` is the musl compiler, which builds fully
# static aarch64 binaries that do not depend on glibc.
COPY guest/ /tmp/guest/
RUN make -C /tmp/guest CC=gcc tools

# Collect everything except the shim into /out/. The shim is built in the
# glibc stage below because EP122, a Pioneer aarch64 binary linked against
# glibc 2.17, must be able to preload it. A musl-linked .so lists `libc.so`
# instead of `libc.so.6` in NEEDED, and the glibc dynamic linker refuses to
# load it.
RUN mkdir -p /out/modules && \
    cp /kernel/arch/arm64/boot/Image                                 /out/Image && \
    cp /subucom_virt/subucom_virt.ko                                 /out/modules/ && \
    cp /virtio_snd/virtio_snd.ko                                     /out/modules/ && \
    cp /udev_usb1/udev_usb1.ko                                       /out/modules/ && \
    cp /tmp/dummy_extract/usr/lib/xorg/modules/drivers/dummy_drv.so /out/ && \
    cp /tmp/guest/out/subucom_forwarder_aarch64                      /out/ && \
    cp /tmp/guest/out/subucom_live_aarch64                           /out/ && \
    cp /tmp/guest/out/cfgd_aarch64                                   /out/ && \
    cp /tmp/guest/out/pc_link_bridge_aarch64                         /out/ && \
    echo "=== /out/ ===" && ls -la /out/ && echo "=== /out/modules/ ===" && ls -la /out/modules/

# Build deck_shim.so against glibc 2.27 (Ubuntu 18.04) so that it can be
# preloaded into Pioneer's EP122 process, which uses glibc 2.17. A shim built
# with Alpine's musl gcc lists `libc.so` in NEEDED, and glibc's ld.so fails with
# `error while loading shared libraries: libc.so: cannot open shared object
# file`. This stage runs the same Makefile with a different toolchain.
FROM ubuntu:18.04 AS shim-builder
RUN apt-get update -qq && \
    apt-get install -y --no-install-recommends gcc g++ libc6-dev make file binutils && \
    rm -rf /var/lib/apt/lists/*
COPY guest/ /tmp/guest/
RUN make -C /tmp/guest CC=gcc shim
# Every process that apl_start.sh starts preloads the shim. abi-check verifies
# the glibc 2.17 ceiling, the libc flavour and that every NEEDED library exists
# on the deck; if any of these fails, the deck boots with no application. This
# is the last step with a real toolchain before the shim reaches a deck.
RUN make -C /tmp/guest CC=gcc abi-check

FROM scratch AS artifacts
COPY --from=builder      /out/                         /
COPY --from=shim-builder /tmp/guest/out/deck_shim.so  /deck_shim.so
