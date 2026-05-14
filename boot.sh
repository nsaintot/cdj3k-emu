#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# boot.sh - boot CDJ-3000 Linux in QEMU/UTM on Apple Silicon Mac
#
# CDJ-3000 hardware: Rockchip RK3399 (aarch64, dual Cortex-A72 + quad A53)
# OS: Buildroot 2018.02 rootfs (Pioneer EP122 firmware), Linux 6.6 LTS kernel
#
# QEMU machine: virt (generic ARM64 virtual platform)
# CPU: cortex-a72 (matches RK3399's big cores)
# Acceleration: HVF (Apple Hypervisor Framework) - native aarch64 on M1/M2/M3
#
# Prerequisites:
#   brew install qemu
#
# Usage:
#   ./qemu/boot.sh [--patched] [--hvf] [--debug] [--gl] [--shm] [--vmnet[=N]]
#
#   --patched   use build/initramfs-patched.cpio.gz (with Xorg + shims)
#   --hvf       (default) use HVF acceleration
#   --debug     add -s -S to wait for GDB on port 1234 (kernel debug)
#   --gl        expose virtio-gpu-device (MMIO, 2D) to the guest and open a
#               native macOS Cocoa window on the host.  Xorg modesetting driver
#               inside the guest renders to the DRM framebuffer; QEMU blits it
#               to the Cocoa window.  Requires patched initramfs (virtio_gpu.ko).
#               Note: Homebrew QEMU 9.x has no virglrenderer/SDL - 3D virgl is
#               unavailable; this path provides a clean 2D display instead.
#   --shm       back guest RAM with ${CDJ3K_SOCK_DIR}/ram.shm (MAP_SHARED file) so the
#               host can inject MISO frames via mmap without GDB RSP.
#               Without this flag, QEMU uses anonymous RAM - faster emulation.
#               Required for tools/cdj-inject.py (Phase 5 injection).
#   --service   boot into EP122TestMode (service/test mode). Injects
#               BTN_CALL_PREV + BTN_TEMPO_RANGE via subucom_virt.ko for ~5s
#               so subucom_read writes "on1" to /tmp/testmode on boot.
#               Implies --patched.
#   --audio     expose virtio-sound-device to the guest (uses CoreAudio on macOS).
#               virtio_snd.ko registers as ALSA card 0 (EP122 writes here → host audio).
#               Implies --patched.
#   --usb-blk   expose virtio-blk-device (MMIO) → /dev/vda inside the guest.
#               Auto-creates build/usb-hot.img (256 MiB sparse, raw, no FS) if
#               missing; format from the guest with `mkfs.vfat /dev/vda`.
#               First-pass smoke test for live-USB hot-plug; loopback-backed
#               /opt/usb.img path remains active in parallel.  Implies --patched.
#
# GDB injection port (Phase 5):
#   Port 1235 is always open when --patched is used.
#   tools/cdj-inject.py uses it to write MISO frames into the running guest.
#   Connect: gdb-multiarch -ex 'target remote localhost:1235'
#
# NOTE: The CDJ kernel was built for RK3399 but we use -machine virt.
#       - Kernel has NO PL011 UART support: serial console is non-functional.
#         Use QMP (port 4445) + pmemsave to read kernel ring buffer.
#       - Kernel has NO built-in virtio or display drivers; the patched
#         initramfs loads them as modules: virtio.ko, virtio_mmio.ko,
#         virtio_net.ko, virtio_blk.ko, virtio_gpu.ko (if --gl is used).
#         Network: LD_PRELOAD djlink_shim captures UDP traffic.
#       - Image.hvf patches out arm_smccc_smc to work with HVF acceleration.
#
# QMP usage:
#   nc localhost 4445  # then: {"execute":"qmp_capabilities"}
#   See tools/read_ringbuf.py for kernel log extraction via pmemsave.

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR" && pwd)"

# ---- QEMU binary ----
# Default: system QEMU (Homebrew).  --gl switches to our patched build which
# includes the shm display backend (-display shm,path=FILE).
QEMU_BIN="qemu-system-aarch64"
PATCHED_QEMU="${REPO_ROOT}/qemu/install/bin/qemu-system-aarch64"

# ---- paths ----
KERNEL="$REPO_ROOT/build/Image"
INITRAMFS_PATCHED="$REPO_ROOT/build/initramfs-patched.cpio.gz"

# QMP port - used by tools/read_ringbuf.py to extract kernel log
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
    echo "ERROR: Kernel not found at $KERNEL - run ./build-initramfs.sh first"
    exit 1
fi

INITRD="$INITRAMFS_PATCHED"
if [[ "$USE_PATCHED" -eq 1 ]]; then
    if [[ ! -f "$INITRAMFS_PATCHED" ]]; then
        echo "ERROR: Patched initramfs not found at $INITRAMFS_PATCHED"
        echo "  Run: ./build-initramfs.sh"
        exit 1
    fi
else
    echo "Using ORIGINAL initramfs (raw boot)"
fi

echo "Kernel: $KERNEL"
echo "Initrd: $INITRD"
echo "QMP:    localhost:$QMP_PORT"

# ---- optional GDB wait (kernel debug) ----
GDB_ARGS=()
if [[ "$DEBUG_GDB" -eq 1 ]]; then
    GDB_ARGS=(-s -S)
    echo "GDB debug: waiting for connection on localhost:1234 (-S freeze)"
fi

# ---- GDB injection port (Phase 5 - always-on for --patched) ----
# Port 1235 is opened so tools/cdj-inject.py can write MISO frames into
# the running guest via QMP stop / GDB RSP M / QMP cont.
# Zero overhead when nothing is connected.
GDB_INJECT_ARGS=()
if [[ "$USE_PATCHED" -eq 1 ]]; then
    GDB_INJECT_ARGS=(-gdb tcp::1235)
    echo "GDB inject: localhost:1235  (tools/cdj-inject.py)"
fi

# ---- RNG (entropy for dropbear key exchange) ----
# Without this, the guest kernel pool on ARM64 virt is unseeded and dropbear
# blocks forever in getrandom() - TCP connects but no SSH banner is sent.
RNG_ARGS=()
if [[ "$USE_PATCHED" -eq 1 ]]; then
    RNG_ARGS=(
        -object "rng-random,id=rng0,filename=/dev/urandom"
        -device "virtio-rng-device,rng=rng0"
    )
fi

# ---- virtio-sound (host audio via CoreAudio) ----
# virtio_snd.ko loads first and claims ALSA card 0; EP122/JUCE routes its audio
# PCM playback through the TX virtqueue → QEMU CoreAudio backend → Mac speakers.
# Only active when --audio is passed.
AUDIO_ARGS=()
if [[ "$USE_AUDIO" -eq 1 ]]; then
    AUDIO_ARGS=(
        # in.voices=0: suppress CoreAudio capture voice - macOS requires
        # explicit microphone permission and QEMU doesn't have it.
        # streams=1: advertise one PCM stream (output only).  With the default
        # streams=2 and in.voices=0, QEMU 11 fails to realize both streams and
        # writes streams=0 to the virtio config space → guest virtio_snd probe
        # fails with "device does not comply with spec version 1.x" (-EINVAL).
        -audiodev "coreaudio,id=audio0,in.voices=0"
        -device  "virtio-sound-device,audiodev=audio0,streams=1"
    )
    echo "Audio:      virtio-sound-device (CoreAudio)  →  ALSA card 0 in guest"
fi

# ---- virtio-serial channels (main / jog / sub) ----
# Three Unix-socket-backed virtserialports are always created when --patched.
# cdj-ui connects to ${CDJ3K_SOCK_DIR}/{main,jog,sub}.sock as a client.
# server=on,wait=off - QEMU listens; cdj-ui may connect/disconnect at will.
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
# A 1 MiB shared memory region between guest and host.
# Layout (see crates/cdj3k-emu-streams/src/jog_stream.rs and guest/shim/jog.c):
#   offset 0x0000  u32 magic = 'JOG1' (0x31474F4A LE)
#   offset 0x0004  u32 seq      (seqlock counter; odd = write in progress)
#   offset 0x0008  u16 width, u16 height
#   offset 0x000C  u32 format   (XRGB8888)
#   offset 0x1000  pixel data (W*H*4 bytes)
# Host polls `seq` from the mmap'd region - no wake channel.
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
    echo "Jog shm:    ivshmem-plain  →  ${JOG_SHM_FILE}  (1 MiB, host mmap zero-copy)"
fi

# ---- SSH / network (e1000 user-mode NIC) ----
# When --patched, expose a QEMU user-mode e1000 NIC.
# The guest insmod-e1000.service loads e1000.ko → eth0 appears.
# link-monitor.sh runs udhcpc → eth0 gets 10.0.2.15/24 from QEMU DHCP.
# dropbear listens on :22 inside the guest → forwarded to host :2222.
#   ssh -p 2222 root@localhost           (key auth or blank password via -B)
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
        echo "            After boot, find IP:  arp -an | grep ${vmnet_mac}"
        echo "            Then SSH:  ssh root@<IP>"
    else
        NET_ARGS=(
            -netdev "user,id=net0,hostfwd=tcp::${CDJ3K_SSH_PORT}-:22,hostfwd=udp::8801-:8801"
            -device "virtio-net-device,netdev=net0,mrg_rxbuf=off"
        )
        echo "Network:    virtio-net-device (MMIO)  →  ssh -p ${CDJ3K_SSH_PORT} root@localhost subucom:8801  socks: ${CDJ3K_SOCK_DIR}"
    fi
fi

# ---- Fake rekordbox USB drive (loop device from /opt/usb.img in initramfs) ----
# Default path (always on with --patched): usb.img embedded in initramfs at
# /opt/usb.img → losetup /dev/loop0 → mount /media/usb/sda[1] → /proc/udev_usb1.
# Driven by usb-drive-mount.service.  No QEMU block device args needed for this.
#
# Live path (--usb-blk): virtio-blk-device on the MMIO bus exposes
# build/usb-hot.img as /dev/vda.
USB_IMG="$REPO_ROOT/build/usb.img"
if [[ "$USE_PATCHED" -eq 1 ]]; then
    if [[ -f "$USB_IMG" ]]; then
        echo "USB drive:  embedded in initramfs as /opt/usb.img  →  /dev/loop0 (losetup)"
    else
        echo "USB drive:  $USB_IMG not found - rebuild after adding build/usb.img"
        echo "            EP122 will show 'No Media' screen"
    fi
fi

# ---- Live USB block device (--usb-blk) ----
USB_BLK_IMG="$REPO_ROOT/build/usb-hot.img"
USB_BLK_ARGS=()
if [[ "$USE_USB_BLK" -eq 1 ]]; then
    if [[ ! -f "$USB_BLK_IMG" ]]; then
        echo "USB blk:    creating empty 256 MiB sparse image at $USB_BLK_IMG"
        mkdir -p "$(dirname "$USB_BLK_IMG")"
        # macOS-friendly sparse allocation; raw, unformatted.
        : > "$USB_BLK_IMG"
        /usr/bin/truncate -s 256M "$USB_BLK_IMG" 2>/dev/null \
            || dd if=/dev/zero of="$USB_BLK_IMG" bs=1 count=0 seek=256m status=none
    fi
    USB_BLK_ARGS=(
        -drive  "file=${USB_BLK_IMG},if=none,id=usbblk0,format=raw,cache=writeback"
        -device "virtio-blk-device,drive=usbblk0,id=usbblk0"
    )
    echo "USB blk:    virtio-blk-device  →  /dev/vda  (image: ${USB_BLK_IMG})"
fi
echo ""

# ---- Shared RAM file (Phase 5 injection) ----
# With --shm, back the guest RAM with a MAP_SHARED file so the Mac host
# can inject MISO frames directly via mmap without any GDB RSP.
# (QEMU 9.x HVF on aarch64 crashes on any GDB m/M memory command.)
#
# Without --shm, QEMU uses anonymous RAM - no file, faster emulation.
# File is sparse on APFS - no actual disk usage until pages are written.
SHARED_RAM=""
# MEM_BYTES is the exact byte count matching the CDJ-3000's Linux-visible RAM.
# IMPORTANT: always pass as "${MEM_BYTES}B" to QEMU - without the 'B' suffix,
# QEMU interprets the value as MiB (~3.9 PB), which exceeds HVF's 40-bit PA limit.
# The jog-LCD framebuffer sits at GPA ~0xD6C0C000 (~3.6 GiB), so we need at least
# that much RAM - the old "size=3G" (3072 MiB) was too small and caused jog garbage.
MEM_BYTES=4038197248   # 0xF0B20000 - must be 64 KiB-aligned (QEMU 9.x mmap requirement)
MEM_ARGS=(-machine virt -m "${MEM_BYTES}B")   # default: anonymous RAM
if [[ "$USE_SHM" -eq 1 ]]; then
    SHARED_RAM="${CDJ3K_SOCK_DIR}/ram.shm"
    # Always recreate as a fresh sparse file - stale SUBUCOM\x00 magic from
    # previous sessions would cause the host to write to the wrong mailbox offset.
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
    echo "Shared RAM: disabled (anonymous RAM - use --shm for host mmap injection)"
fi

# ---- Display / GPU ----
# virtio-gpu-device is always exposed when --patched.  The guest Xorg modesetting
# driver needs /dev/dri/card0 (from virtio_gpu.ko) to start X :0; without it
# x11-only.service fails and EP122 never spawns.
# Default (--patched, no --gl): -display none - GPU renders to memory, no host window.
# --gl: switches host display to a native Cocoa window (virtio-gpu-device MMIO, 2D).
#   Xorg modesetting uses the DRM framebuffer as a dumb KMS buffer.
#   NOTE: Homebrew QEMU 9.x has no virglrenderer - virtio-gpu-gl / sdl,gl=on are
#   unavailable; virtio-gpu-device + cocoa is the supported 2D path.
#   Requires: virtio_gpu.ko in initramfs (./guest/modules/build/build-initramfs.sh)
VIRTIO_GPU_KO="$REPO_ROOT/build/initramfs-work/rootfs/lib/modules/virtio_gpu.ko"
VIRTIO_GPU_KO_ALT="$REPO_ROOT/build/initramfs-work/rootfs/usr/lib/modules/virtio_gpu.ko"
DISPLAY_ARGS=(-display none)
GPU_DEVICE_ARGS=()
if [[ "$USE_PATCHED" -eq 1 ]]; then
    if [[ "$USE_GL" -eq 1 ]]; then
        # --gl mode: expose virtio-gpu-device (MMIO) + Cocoa window for DRM/KMS display.
        # Pioneer Xorg uses modesetting driver on /dev/dri/card0 (from virtio_gpu.ko).
        # NOTE: virtio_gpu.ko has struct ABI issues with Pioneer kernel; --gl may crash.
        #       Headless mode (no --gl) uses Xorg dummy driver - no DRM hardware needed.
        VIRTIO_GPU_AVAILABLE=0
        if [[ -f "$VIRTIO_GPU_KO" ]] || [[ -f "$VIRTIO_GPU_KO_ALT" ]]; then
            VIRTIO_GPU_AVAILABLE=1
        elif [[ -f "$INITRD" ]]; then
            # rootfs may be removed by build-initramfs.sh --clean; verify packed archive.
            if gzip -dc "$INITRD" 2>/dev/null | cpio -it 2>/dev/null | rg -q '(^|/)virtio_gpu\.ko$'; then
                VIRTIO_GPU_AVAILABLE=1
            fi
        fi

        if [[ "$VIRTIO_GPU_AVAILABLE" -eq 0 ]]; then
            echo "WARNING: virtio_gpu.ko not found in initramfs (needed for --gl)."
            echo "         Run:  ./build-initramfs.sh"
        else
            # max_outputs=2: exposes both virtual outputs to the Pioneer kernel.
            #   output 0 (1280×720) → CRTC 29 (main LCD / DSI-1)
            #   output 1 (1280×240) → CRTC 32 (jog LCD / DSI-2)
            # With max_outputs=1 (old default), the Pioneer kernel's SETCRTC for CRTC 32
            # fails silently → EP122TestMode detects the failure and stops rendering to
            # the jog framebuffer → ep122_shim.so captures only zeros.
            # max_outputs=1: Xorg only sees CRTC 29 (main LCD). CRTC 32 (jog LCD) is
            # intercepted by ep122_shim.so which fakes SETCRTC success so EP122TestMode
            # renders into the GEM buffer; ep122_shim then copies it to the memfd.
            # max_outputs=2 caused Pioneer _pw_dc to crash inside Xorg's dumb alloc.
            GPU_DEVICE_ARGS=(-device "virtio-gpu-device,id=virtio-gpu0,xres=1280,yres=720,max_outputs=1")
            if [[ -x "${PATCHED_QEMU}" ]]; then
                QEMU_BIN="${PATCHED_QEMU}"
                DISPLAY_ARGS=(-display "shm,path=${CDJ3K_SOCK_DIR}/main.shm")
                echo "GPU:        virtio-gpu-device (id=virtio-gpu0, MMIO 2D)  xres=1280 yres=720  max_outputs=1"
                echo "Display:    shm → ${CDJ3K_SOCK_DIR}/main.shm  (zero-copy mmap, cdj-ui reads directly)"
            else
                DISPLAY_ARGS=(-display none)
                echo "GPU:        virtio-gpu-device (id=virtio-gpu0, MMIO 2D)  xres=1280 yres=720  max_outputs=1"
                echo "Display:    headless (patched QEMU not found at ${PATCHED_QEMU}, run qemu/build.sh)"
            fi
        fi
    else
        # Headless mode: no virtio-gpu-device exposed.
        # Guest Xorg uses the 'dummy' video driver (ABI 24.0, from Ubuntu 20.04 arm64).
        # dummy_drv.so creates a virtual framebuffer in memory - no DRM hardware required.
        echo "Display:    headless (-display none)  Xorg dummy driver"
    fi
fi
if [[ "$USE_PATCHED" -eq 0 ]]; then
    echo "Display:    headless (no GPU device - original boot)"
fi
echo ""

# ---- QEMU launch ----
# NOTE: CDJ kernel has no PL011 UART support - console= is decorative.
#       Use QMP pmemsave + tools/read_ringbuf.py to read kernel ring buffer.
#       virtio-net-device and virtio_gpu are loaded as modules (patched initramfs).
#       Display (headless default):  Xorg dummy driver inside the guest (-display none)
#       Display (--gl mode):         virtio-gpu-device (MMIO 2D) → Xorg modesetting → DRM KMS;
#                                    host Cocoa window shows framebuffer via shm display.
#       DJ Link UDP packets captured by djlink_shim.so → /tmp/djlink.log
#       Phase 5 injection: host writes to $SHARED_RAM (MAP_SHARED mmap).
#         tools/cdj-inject.py uses file_offset = GPA - 0x40000000.
#
# CPU/performance notes:
#   loglevel=1  - suppresses DEBUG/INFO/NOTICE/WARNING kernel messages.
#                 The CDJ kernel is a debug build (CONFIG_DEBUG_PREEMPT=y,
#                 CONFIG_DEBUG_SPINLOCK=y, SCHED_DEBUG) - every printk hits
#                 the spinlock-guarded ring buffer with debug validation.
#                 loglevel=7 during boot spams thousands of messages.
#   nowatchdog  - disables the soft/hard lockup watchdog. Saves periodic
#                 watchdog-tick overhead (not needed in emulation).
#   Remaining CPU load is inherent: EP122 renders its full GUI at ~60fps
#   through Xvfb (pure software X, no GPU). Use --gl for virtio-gpu KMS.
# ---- Service mode (testmode) kernel cmdline ----
KCMDLINE="root=/dev/ram0 rdinit=/init loglevel=7 nowatchdog rng_core.default_quality=1024 console=ttyAMA0,115200 systemd.journald.forward_to_console=1"
if [[ "$USE_SERVICE" -eq 1 ]]; then
    KCMDLINE="$KCMDLINE subucom_testmode"
    echo "Service mode: EP122TestMode (subucom_virt inject_testmode=1, auto-clear ~5s)"
    echo ""
fi

echo "CDJ3K instance: ${CDJ3K_ID}  SSH: ${CDJ3K_SSH_PORT}  Sockets: ${CDJ3K_SOCK_DIR}"

SERIAL_LOG="${CDJ3K_SOCK_DIR}/serial.log"
echo "Serial log:  $SERIAL_LOG  (tail -f to read PL011 console)"

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
