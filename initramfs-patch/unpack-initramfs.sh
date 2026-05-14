#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# initramfs-patch/unpack-initramfs.sh
#
# Extracts the initramfs CPIO archive embedded inside the CDJ-3000 kernel Image,
# applies patches, then repacks it ready for QEMU.
#
# Dependencies:
#   brew install binwalk cpio
#
# Usage:
#   ./unpack-initramfs.sh [--extract-only] <Image> [workdir]
#
# Outputs:
#   <workdir>/initramfs.cpio.gz   - original extracted archive
#   <workdir>/rootfs/             - extracted filesystem tree
#   <workdir>/initramfs-patched.cpio.gz - repacked with patches applied

set -euo pipefail

EXTRACT_ONLY=0
POSITIONAL=()
for arg in "$@"; do
    case "$arg" in
        --extract-only) EXTRACT_ONLY=1 ;;
        -h|--help)
            echo "Usage: $0 [--extract-only] <Image> [workdir]"
            echo "  --extract-only  stop after extracting initramfs.cpio.gz and rootfs/"
            exit 0
            ;;
        *) POSITIONAL+=("$arg") ;;
    esac
done

if [[ "${#POSITIONAL[@]}" -lt 1 ]]; then
    echo "Usage: $0 [--extract-only] <Image> [workdir]" >&2
    exit 1
fi

IMAGE="${POSITIONAL[0]}"
WORKDIR="${POSITIONAL[1]:-$(dirname "$IMAGE")/initramfs-work}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

mkdir -p "$WORKDIR"

echo "=== Step 1: locate initramfs offset in Image ==="
# binwalk finds the gzip/lzma/cpio headers
binwalk "$IMAGE" | tee "$WORKDIR/binwalk.txt"

# Extract the gzip-compressed initramfs (largest gz block after the kernel)
OFFSET=$(grep -E 'gzip|cpio' "$WORKDIR/binwalk.txt" | tail -1 | awk '{print $1}')
if [[ -z "$OFFSET" ]]; then
    echo "ERROR: could not find initramfs offset in Image via binwalk" >&2
    echo "Try: binwalk -e $IMAGE" >&2
    exit 1
fi
echo "Initramfs at offset: $OFFSET"

echo "=== Step 2: extract initramfs ==="
dd if="$IMAGE" bs=1 skip="$OFFSET" of="$WORKDIR/initramfs.cpio.gz" 2>/dev/null
# Trim any trailing garbage (cpio is self-delimiting, gzip will stop at EOF marker)
echo "Extracted: $(ls -lh $WORKDIR/initramfs.cpio.gz)"

echo "=== Step 3: unpack CPIO ==="
mkdir -p "$WORKDIR/rootfs"
cd "$WORKDIR/rootfs"
gunzip -c "$WORKDIR/initramfs.cpio.gz" | cpio -i --make-directories 2>/dev/null || true
cd - >/dev/null
echo "Unpacked rootfs: $(find $WORKDIR/rootfs | wc -l) entries"

if [[ "$EXTRACT_ONLY" -eq 1 ]]; then
    echo ""
    echo "=== DONE (extract-only) ==="
    echo "Extracted initramfs: $WORKDIR/initramfs.cpio.gz"
    echo "Unpacked rootfs:     $WORKDIR/rootfs"
    exit 0
fi

echo "=== Step 4: apply patches ==="
"$SCRIPT_DIR/patch-rootfs.sh" "$WORKDIR/rootfs"

echo "=== Step 5: repack CPIO ==="
cd "$WORKDIR/rootfs"
find . | cpio -H newc -o 2>/dev/null | gzip -9 > "$WORKDIR/initramfs-patched.cpio.gz"
cd - >/dev/null
echo "Repacked: $(ls -lh $WORKDIR/initramfs-patched.cpio.gz)"

echo ""
echo "=== DONE ==="
echo "Patched initramfs: $WORKDIR/initramfs-patched.cpio.gz"
echo "Use with QEMU:"
echo "  -kernel $IMAGE -initrd $WORKDIR/initramfs-patched.cpio.gz"
