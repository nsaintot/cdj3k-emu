#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# boot.sh - boot the CDJ-3000 firmware in QEMU on an Apple silicon Mac, for
# development without the app.
#
# The guest runs the Pioneer EP122 rootfs (Buildroot 2018.02) on the project's
# Linux 6.6 LTS kernel, on QEMU's -machine virt with HVF acceleration.
#
# Prerequisites:
#   brew install qemu
#
# Usage:
#   ./boot.sh [--id N] [--patched] [--no-hvf] [--debug] [--gl] [--shm]
#             [--service] [--audio] [--usb-blk] [--net-bridge=IFACE]
#
#   --id N      the instance number, which selects the socket directory
#               (/tmp/cdj3k-emu-<uid>/instance-N) and the SSH port (2222 + N)
#   --patched   use build/initramfs-patched.cpio.gz, the initramfs that
#               build.sh patches
#   --no-hvf    accepted, but the script always uses HVF
#   --debug     add -s -S, so QEMU waits for GDB on port 1234
#   --gl        expose virtio-gpu-device (MMIO, 2D) to the guest. Xorg's
#               modesetting driver renders to the DRM framebuffer, and the
#               patched QEMU's shm display shares it with the host. The script
#               checks that the initramfs carries virtio_gpu.ko. Homebrew
#               QEMU has no virglrenderer, so 3D virgl is unavailable.
#   --shm       back the guest RAM with ${CDJ3K_SOCK_DIR}/ram.shm, a
#               MAP_SHARED file, so a host tool can write MISO frames through
#               mmap without GDB. Without this flag, QEMU uses anonymous RAM,
#               which emulates faster.
#   --service   boot into EP122TestMode (service mode). subucom_virt.ko holds
#               BTN_CALL_PREV and BTN_TEMPO_RANGE for about 5 s, so
#               subucom_read writes "on1" to /tmp/testmode at boot.
#               Implies --patched.
#   --audio     expose virtio-sound-device to the guest, with CoreAudio on the
#               host. virtio_snd.ko registers as ALSA card 0; EP122 plays to it
#               and the host outputs the sound. Implies --patched.
#   --usb-blk   expose a virtio-blk-device (MMIO). As the first virtio-blk
#               disk, it appears as /dev/mmcblk1 in the guest (kernel patch
#               01-virtio-blk-naming). The script creates build/usb-hot.img
#               (256 MiB, sparse, raw, no file system) if it is missing;
#               format it from the guest with `mkfs.vfat /dev/mmcblk1`. This
#               tests live USB hot-plug; the loopback drive from /opt/usb.img
#               stays active alongside it. Implies --patched.
#   --net-bridge=IFACE
#               bridge the guest's NIC onto IFACE with vmnet-bridged.
#               Implies --patched.
#
# GDB injection port:
#   Port 1235 is open whenever --patched is used. A host tool can write MISO
#   frames into the running guest through it.
#   Connect: gdb-multiarch -ex 'target remote localhost:1235'
#
# The guest kernel has PL011 and the virtio drivers built in. The serial
# console goes to ${CDJ3K_SOCK_DIR}/serial.log; QMP on port 4445 with
# pmemsave can also read the kernel ring buffer.
#
# QMP usage:
#   nc localhost 4445  # then: {"execute":"qmp_capabilities"}

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR" && pwd)"

# ---- QEMU binary ----
# The script uses the system QEMU from Homebrew by default; --gl switches to
# the patched build, which includes the shm display backend
# (-display shm,path=FILE).
QEMU_BIN="qemu-system-aarch64"
PATCHED_QEMU="${REPO_ROOT}/qemu/install/bin/qemu-system-aarch64"

# ---- paths ----
KERNEL="$REPO_ROOT/build/Image"
INITRAMFS_PATCHED="$REPO_ROOT/build/initramfs-patched.cpio.gz"

# The QMP port, through which pmemsave can read the kernel log.
QMP_PORT=4445

# ---- instance ID (--id N) ----
CDJ3K_ID=0
_args=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --id) CDJ3K_ID="$2"; shift 2 ;;
    *) _args+=("$1"); shift ;;
  esac
done
set -- "${_args[@]}"

CDJ3K_SOCK_DIR="/tmp/cdj3k-emu-$(id -u)/instance-${CDJ3K_ID}"
CDJ3K_SSH_PORT=$((2222 + CDJ3K_ID))
mkdir -p "${CDJ3K_SOCK_DIR}"
chmod 0700 "$(dirname "${CDJ3K_SOCK_DIR}")"

# ---- options ----
USE_PATCHED=0
USE_HVF=1
DEBUG_GDB=0
USE_GL=0
USE_SHM=0
NET_BRIDGE_IFACE=""
USE_SERVICE=0
USE_AUDIO=0
USE_USB_BLK=0
for arg in "$@"; do
    case "$arg" in
        --patched)      USE_PATCHED=1 ;;
        --hvf)          USE_HVF=1 ;;
        --no-hvf)       USE_HVF=0 ;;
        --debug)        DEBUG_GDB=1 ;;
        --gl)           USE_GL=1; USE_PATCHED=1 ;;
        --shm)          USE_SHM=1 ;;
        --audio)        USE_AUDIO=1; USE_PATCHED=1 ;;
        --usb-blk)      USE_USB_BLK=1; USE_PATCHED=1 ;;
        --service)      USE_SERVICE=1; USE_PATCHED=1 ;;
        --net-bridge=*) NET_BRIDGE_IFACE="${arg#--net-bridge=}"; USE_PATCHED=1 ;;
    esac
done

ACCEL_ARGS=(-accel hvf)
CPU_ARG=host

if [[ ! -f "$KERNEL" ]]; then
    echo "ERROR: Kernel not found at $KERNEL; run ./build.sh first."
    exit 1
fi

INITRD="$INITRAMFS_PATCHED"
if [[ "$USE_PATCHED" -eq 1 ]]; then
    if [[ ! -f "$INITRAMFS_PATCHED" ]]; then
        echo "ERROR: Patched initramfs not found at $INITRAMFS_PATCHED"
        echo "  Run: ./build.sh"
        exit 1
    fi
else
    echo "Using the original initramfs (raw boot)."
fi

echo "Kernel: $KERNEL"
echo "Initrd: $INITRD"
echo "QMP:    localhost:$QMP_PORT"

# ---- optional GDB wait (kernel debug) ----
GDB_ARGS=()
if [[ "$DEBUG_GDB" -eq 1 ]]; then
    GDB_ARGS=(-s -S)
    echo "GDB debug: waiting for a connection on localhost:1234; -S keeps the CPU stopped until then."
fi

# ---- GDB injection port (always on with --patched) ----
# Port 1235 lets a host tool write MISO frames into the running guest: it
# stops the guest over QMP, writes with the GDB RSP M command and resumes with
# QMP cont. The port costs nothing while no debugger is connected.
GDB_INJECT_ARGS=()
if [[ "$USE_PATCHED" -eq 1 ]]; then
    GDB_INJECT_ARGS=(-gdb tcp::1235)
    echo "GDB inject: localhost:1235"
fi

# ---- RNG (entropy for the dropbear key exchange) ----
# Without it, the guest's entropy pool on ARM64 virt stays unseeded and
# dropbear blocks in getrandom(): TCP connects, but no SSH banner arrives.
RNG_ARGS=()
if [[ "$USE_PATCHED" -eq 1 ]]; then
    RNG_ARGS=(
        -object "rng-random,id=rng0,filename=/dev/urandom"
        -device "virtio-rng-device,rng=rng0"
    )
fi

# ---- virtio-sound (host audio through CoreAudio) ----
# virtio_snd.ko loads first and claims ALSA card 0. EP122's JUCE audio goes
# through the TX virtqueue to the QEMU CoreAudio backend and the Mac's output.
# This is active only with --audio.
AUDIO_ARGS=()
if [[ "$USE_AUDIO" -eq 1 ]]; then
    AUDIO_ARGS=(
        # in.voices=0 disables the CoreAudio capture voice, because macOS
        # requires a microphone permission that QEMU does not have.
        # streams=1 advertises one PCM stream, for output only. With the
        # default streams=2 and in.voices=0, QEMU 11 fails to realise both
        # streams and writes streams=0 to the virtio config space, so the
        # guest's virtio_snd probe fails with "device does not comply with
        # spec version 1.x" (-EINVAL).
        -audiodev "coreaudio,id=audio0,in.voices=0"
        -device  "virtio-sound-device,audiodev=audio0,streams=1"
    )
    echo "Audio:      virtio-sound-device (CoreAudio), ALSA card 0 in the guest"
fi

# ---- virtio-serial channels (main, sub and led) ----
# With --patched, the script always creates three virtserialports backed by
# Unix sockets: ${CDJ3K_SOCK_DIR}/main.sock, sub.sock and led.sock. With
# server=on,wait=off, QEMU listens, and a client can connect and disconnect
# at any time.
VSERIAL_ARGS=()
if [[ "$USE_PATCHED" -eq 1 ]]; then
    VSERIAL_ARGS=(
        -device "virtio-serial-device,max_ports=8"
        -chardev "socket,id=vserial_main,path=${CDJ3K_SOCK_DIR}/main.sock,server=on,wait=off"
        -device "virtserialport,chardev=vserial_main,name=cdj3k.main"
        -chardev "socket,id=vserial_sub,path=${CDJ3K_SOCK_DIR}/sub.sock,server=on,wait=off"
        -device "virtserialport,chardev=vserial_sub,name=cdj3k.sub"
        -chardev "socket,id=vserial_led,path=${CDJ3K_SOCK_DIR}/led.sock,server=on,wait=off"
        -device "virtserialport,chardev=vserial_led,name=cdj3k.led,nr=5"
    )
fi

# ---- ivshmem (jog LCD zero-copy frame buffer) ----
# A 1 MiB shared memory region between the guest and the host.
# Layout (see crates/cdj3k-emu-streams/src/jog_stream.rs and
# guest/deck_shim/core/deck_shim.h):
#   offset 0x0000  u32 magic = 'JOG1' (0x31474F4A LE)
#   offset 0x0004  u32 seq      (seqlock counter; odd = write in progress)
#   offset 0x0008  u16 width, u16 height
#   offset 0x000C  u32 format   (XRGB8888)
#   offset 0x1000  pixel data (W*H*4 bytes)
# The host polls `seq` in the mapped region; there is no wake-up channel.
JOG_SHM_FILE="${CDJ3K_SOCK_DIR}/jog.shm"
IVSHMEM_ARGS=()
if [[ "$USE_PATCHED" -eq 1 ]]; then
    : > "$JOG_SHM_FILE"
    /usr/bin/truncate -s 1M "$JOG_SHM_FILE" 2>/dev/null \
        || dd if=/dev/zero of="$JOG_SHM_FILE" bs=1 count=0 seek=1m status=none
    IVSHMEM_ARGS=(
        -object  "memory-backend-file,id=jogshm,mem-path=${JOG_SHM_FILE},size=1M,share=on"
        -device  "ivshmem-plain,memdev=jogshm,master=on"
    )
    echo "Jog shm:    ivshmem-plain backed by ${JOG_SHM_FILE} (1 MiB, mapped zero-copy by the host)"
fi

# ---- SSH and network (virtio-net NIC) ----
# With --patched, the guest gets a virtio-net NIC: QEMU user-mode networking
# by default, or vmnet-bridged with --net-bridge=IFACE. In user mode,
# link-monitor.sh runs udhcpc, and eth0 gets 10.0.2.15/24 from QEMU's DHCP
# server. dropbear listens on port 22 in the guest, which QEMU forwards to
# host port 2222 + ID; UDP port 8801 is forwarded too.
#   ssh -p 2222 root@localhost           (key or blank password through -B)
#   scp -P 2222 file root@localhost:/tmp/
NET_ARGS=()
if [[ "$USE_PATCHED" -eq 1 ]]; then
    if [[ -n "$NET_BRIDGE_IFACE" ]]; then
        vmnet_mac=$(printf "0a:00:00:00:00:%02x" "$CDJ3K_ID")
        NET_ARGS=(
            -netdev "vmnet-bridged,id=net0,ifname=${NET_BRIDGE_IFACE}"
            -device "virtio-net-device,netdev=net0,mac=${vmnet_mac},mrg_rxbuf=off"
        )
        echo "Network:    vmnet-bridged (Pro DJ Link)  iface=${NET_BRIDGE_IFACE}  MAC=${vmnet_mac}"
        echo "            After boot, find the IP with:  arp -an | grep ${vmnet_mac}"
        echo "            Then connect with:  ssh root@<IP>"
    else
        NET_ARGS=(
            -netdev "user,id=net0,hostfwd=tcp::${CDJ3K_SSH_PORT}-:22,hostfwd=udp::8801-:8801"
            -device "virtio-net-device,netdev=net0,mrg_rxbuf=off"
        )
        echo "Network:    virtio-net-device (MMIO); SSH with ssh -p ${CDJ3K_SSH_PORT} root@localhost; subucom on port 8801; sockets in ${CDJ3K_SOCK_DIR}"
    fi
fi

# ---- Fake rekordbox USB drive (a loop device on /opt/usb.img in the initramfs) ----
# By default (always on with --patched), the initramfs carries usb.img at
# /opt/usb.img. usb-drive-mount.service attaches it to /dev/loop0 with
# losetup, mounts it at /media/usb/sdb1 and reports it in /proc/udev_usb1.
# This path needs no QEMU block device arguments.
#
# With --usb-blk, a virtio-blk-device on the MMIO bus exposes
# build/usb-hot.img as /dev/mmcblk1.
USB_IMG="$REPO_ROOT/build/usb.img"
if [[ "$USE_PATCHED" -eq 1 ]]; then
    if [[ -f "$USB_IMG" ]]; then
        echo "USB drive:  embedded in the initramfs as /opt/usb.img and attached to /dev/loop0 with losetup"
    else
        echo "USB drive:  $USB_IMG not found; rebuild after adding build/usb.img."
        echo "            EP122 will show the 'No Media' screen."
    fi
fi

# ---- Live USB block device (--usb-blk) ----
USB_BLK_IMG="$REPO_ROOT/build/usb-hot.img"
USB_BLK_ARGS=()
if [[ "$USE_USB_BLK" -eq 1 ]]; then
    if [[ ! -f "$USB_BLK_IMG" ]]; then
        echo "USB blk:    creating an empty 256 MiB sparse image at $USB_BLK_IMG"
        mkdir -p "$(dirname "$USB_BLK_IMG")"
        # Create a raw, unformatted sparse image in a way that also works on macOS.
        : > "$USB_BLK_IMG"
        /usr/bin/truncate -s 256M "$USB_BLK_IMG" 2>/dev/null \
            || dd if=/dev/zero of="$USB_BLK_IMG" bs=1 count=0 seek=256m status=none
    fi
    USB_BLK_ARGS=(
        -drive  "file=${USB_BLK_IMG},if=none,id=usbblk0,format=raw,cache=writeback"
        -device "virtio-blk-device,drive=usbblk0,id=usbblk0"
    )
    echo "USB blk:    virtio-blk-device, /dev/mmcblk1 in the guest (image: ${USB_BLK_IMG})"
fi
echo ""

# ---- Shared RAM file (host injection) ----
# With --shm, back the guest RAM with a MAP_SHARED file, so the host can write
# MISO frames directly through mmap without GDB. QEMU's HVF on aarch64 crashes
# on GDB m and M memory commands.
#
# Without --shm, QEMU uses anonymous RAM, which needs no file and emulates
# faster. The file is sparse on APFS and uses no disk space until pages are
# written.
SHARED_RAM=""
# MEM_BYTES is the exact byte count of the CDJ-3000's Linux-visible RAM.
# Always pass it to QEMU as "${MEM_BYTES}B": without the 'B' suffix, QEMU reads
# the value as MiB (about 3.9 PB), which exceeds HVF's 40-bit PA limit.
# The jog LCD framebuffer sits at GPA 0xD6C0C000 (about 3.6 GiB), so the guest
# needs at least that much RAM.
MEM_BYTES=4038197248   # 0xF0B20000; QEMU's mmap needs it 64 KiB-aligned.
MEM_ARGS=(-machine virt -m "${MEM_BYTES}B")   # Anonymous RAM by default.
if [[ "$USE_SHM" -eq 1 ]]; then
    SHARED_RAM="${CDJ3K_SOCK_DIR}/ram.shm"
    # Always recreate the sparse file: a stale SUBUCOM\x00 magic from an
    # earlier session would make the host write to the wrong mailbox offset.
    echo "Clearing shared RAM file (${MEM_BYTES} bytes): $SHARED_RAM"
    python3 -c "
import os
fd = os.open('$SHARED_RAM', os.O_CREAT|os.O_RDWR|os.O_TRUNC, 0o600)
os.ftruncate(fd, $MEM_BYTES)
os.close(fd)
print('  Ready (sparse).')
"
    MEM_ARGS=(
        -object "memory-backend-file,id=ram0,size=${MEM_BYTES}B,mem-path=${SHARED_RAM},share=on"
        -machine virt,memory-backend=ram0
        -m "${MEM_BYTES}B"
    )
    echo "Shared RAM: $SHARED_RAM  (host mmap injection enabled)"
else
    echo "Shared RAM: disabled (anonymous RAM; use --shm for host mmap injection)"
fi

# ---- Display / GPU ----
# By default (--patched without --gl), the script exposes no GPU and passes
# -display none; the guest's Xorg uses the dummy driver (patch 25).
# --gl exposes virtio-gpu-device (MMIO, 2D), and Xorg's modesetting driver
# uses the DRM framebuffer as a dumb KMS buffer. Homebrew QEMU 9.x has no
# virglrenderer, so virtio-gpu-gl and sdl,gl=on are unavailable. The script
# checks that the initramfs carries virtio_gpu.ko.
VIRTIO_GPU_KO="$REPO_ROOT/build/initramfs-work/rootfs/lib/modules/virtio_gpu.ko"
VIRTIO_GPU_KO_ALT="$REPO_ROOT/build/initramfs-work/rootfs/usr/lib/modules/virtio_gpu.ko"
DISPLAY_ARGS=(-display none)
GPU_DEVICE_ARGS=()
if [[ "$USE_PATCHED" -eq 1 ]]; then
    if [[ "$USE_GL" -eq 1 ]]; then
        # In --gl mode, expose virtio-gpu-device (MMIO) for the DRM/KMS display.
        # Pioneer's Xorg uses the modesetting driver on /dev/dri/card0.
        # Headless mode (without --gl) uses the Xorg dummy driver and needs no
        # DRM hardware.
        VIRTIO_GPU_AVAILABLE=0
        if [[ -f "$VIRTIO_GPU_KO" ]] || [[ -f "$VIRTIO_GPU_KO_ALT" ]]; then
            VIRTIO_GPU_AVAILABLE=1
        elif [[ -f "$INITRD" ]]; then
            # build.sh --clean removes the rootfs, so check the packed archive instead.
            if gzip -dc "$INITRD" 2>/dev/null | cpio -it 2>/dev/null | rg -q '(^|/)virtio_gpu\.ko$'; then
                VIRTIO_GPU_AVAILABLE=1
            fi
        fi

        if [[ "$VIRTIO_GPU_AVAILABLE" -eq 0 ]]; then
            echo "WARNING: virtio_gpu.ko not found in the initramfs (needed for --gl)."
            echo "         Run:  ./build.sh"
        else
            # max_outputs=1: Xorg sees only CRTC 29 (the main LCD). The guest
            # kernel adds the jog LCD as a second, synthetic output (CRTC 32,
            # DSI-2), and deck_shim.so reads the app's jog framebuffer and
            # publishes it through ivshmem.
            # Do not use max_outputs=2: Pioneer's _pw_dc crashes inside Xorg's
            # dumb buffer allocation.
            GPU_DEVICE_ARGS=(-device "virtio-gpu-device,id=virtio-gpu0,xres=1280,yres=720,max_outputs=1")
            if [[ -x "${PATCHED_QEMU}" ]]; then
                QEMU_BIN="${PATCHED_QEMU}"
                DISPLAY_ARGS=(-display "shm,path=${CDJ3K_SOCK_DIR}/main.shm")
                echo "GPU:        virtio-gpu-device (id=virtio-gpu0, MMIO 2D)  xres=1280 yres=720  max_outputs=1"
                echo "Display:    shm in ${CDJ3K_SOCK_DIR}/main.shm (the UI maps it and reads it zero-copy)"
            else
                DISPLAY_ARGS=(-display none)
                echo "GPU:        virtio-gpu-device (id=virtio-gpu0, MMIO 2D)  xres=1280 yres=720  max_outputs=1"
                echo "Display:    headless (patched QEMU not found at ${PATCHED_QEMU}; run qemu/build.sh)"
            fi
        fi
    else
        # Headless mode exposes no virtio-gpu-device. The guest's Xorg uses the
        # dummy video driver (ABI 24.0, from Ubuntu 20.04 arm64), which creates
        # a framebuffer in memory and needs no DRM hardware.
        echo "Display:    headless (-display none)  Xorg dummy driver"
    fi
fi
if [[ "$USE_PATCHED" -eq 0 ]]; then
    echo "Display:    headless (no GPU device; original boot)"
fi
echo ""

# ---- QEMU launch ----
# The serial console (console=ttyAMA0) goes to ${CDJ3K_SOCK_DIR}/serial.log.
# Display, headless by default: the Xorg dummy driver in the guest
#   (-display none).
# Display in --gl mode: Xorg modesetting drives DRM KMS on virtio-gpu-device
#   (MMIO 2D), and the patched QEMU's shm display shares the framebuffer.
# Injection: with --shm, the host writes to $SHARED_RAM (MAP_SHARED mmap); a
#   guest physical address maps to file offset GPA - 0x40000000.
#
# Kernel command line:
#   nowatchdog  disables the soft and hard lockup watchdog, whose periodic
#               ticks emulation does not need.
#   Most of the remaining CPU load comes from EP122 rendering its full GUI at
#   about 60 fps in software X. Use --gl for virtio-gpu KMS.
# ---- Service mode (testmode) kernel command line ----
KCMDLINE="root=/dev/ram0 rdinit=/init loglevel=7 nowatchdog rng_core.default_quality=1024 console=ttyAMA0,115200 systemd.journald.forward_to_console=1"
if [[ "$USE_SERVICE" -eq 1 ]]; then
    KCMDLINE="$KCMDLINE subucom_testmode"
    echo "Service mode: EP122TestMode (subucom_virt inject_testmode=1, released after about 5 s)"
    echo ""
fi

echo "CDJ3K instance: ${CDJ3K_ID}  SSH: ${CDJ3K_SSH_PORT}  Sockets: ${CDJ3K_SOCK_DIR}"

SERIAL_LOG="${CDJ3K_SOCK_DIR}/serial.log"
echo "Serial log:  $SERIAL_LOG  (tail -f it to read the PL011 console)"

echo "+ ${QEMU_BIN} ${MEM_ARGS[@]} -cpu $CPU_ARG ${ACCEL_ARGS[@]} -smp 6 -kernel $KERNEL -initrd $INITRD -append \"$KCMDLINE\" ${DISPLAY_ARGS[@]} -serial file:${SERIAL_LOG} -qmp tcp:localhost:${QMP_PORT},server=on,wait=off ${RNG_ARGS[@]+"${RNG_ARGS[@]}"} ${AUDIO_ARGS[@]+"${AUDIO_ARGS[@]}"} ${NET_ARGS[@]+"${NET_ARGS[@]}"} ${GPU_DEVICE_ARGS[@]+"${GPU_DEVICE_ARGS[@]}"} ${USB_BLK_ARGS[@]+"${USB_BLK_ARGS[@]}"} ${VSERIAL_ARGS[@]+"${VSERIAL_ARGS[@]}"} ${GDB_ARGS[@]+"${GDB_ARGS[@]}"} ${GDB_INJECT_ARGS[@]+"${GDB_INJECT_ARGS[@]}"}"

exec "${QEMU_BIN}" \
    "${MEM_ARGS[@]}" \
    -cpu "$CPU_ARG" \
    "${ACCEL_ARGS[@]}" \
    -smp 6 \
    \
    -kernel "$KERNEL" \
    -initrd "$INITRD" \
    -append "$KCMDLINE" \
    \
    "${DISPLAY_ARGS[@]}" \
    -serial "file:${SERIAL_LOG}" \
    \
    -qmp tcp:localhost:${QMP_PORT},server=on,wait=off \
    \
    "${RNG_ARGS[@]+"${RNG_ARGS[@]}"}" \
    \
    "${AUDIO_ARGS[@]+"${AUDIO_ARGS[@]}"}" \
    \
    "${NET_ARGS[@]+"${NET_ARGS[@]}"}" \
    \
    "${GPU_DEVICE_ARGS[@]+"${GPU_DEVICE_ARGS[@]}"}" \
    \
    "${USB_BLK_ARGS[@]+"${USB_BLK_ARGS[@]}"}" \
    \
    "${VSERIAL_ARGS[@]+"${VSERIAL_ARGS[@]}"}" \
    \
    "${IVSHMEM_ARGS[@]+"${IVSHMEM_ARGS[@]}"}" \
    \
    "${GDB_ARGS[@]+""${GDB_ARGS[@]}""}" \
    "${GDB_INJECT_ARGS[@]+"${GDB_INJECT_ARGS[@]}"}"
