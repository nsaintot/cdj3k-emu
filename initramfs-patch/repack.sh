#!/bin/sh
# SPDX-License-Identifier: MIT OR Apache-2.0
# POSIX sh: `set -o pipefail` is bash-only, so this script catches mid-pipeline
# failures via explicit exit checks on the python3/find/cpio invocations below.
set -eu
# Install cpio if not already present (works with Alpine apk or Debian apt-get).
#
# IMPORTANT - DO NOT repack with macOS bsdcpio directly:
#   macOS bsdcpio stores the macOS user UID (e.g. 502) in cpio headers instead
#   of root (0).  The Linux kernel unpacks with that UID → dropbear sees wrong
#   ownership on /etc/shadow and host keys → falls back to password auth.
#
# Always repack via Docker (OrbStack/Rosetta maps macOS user → UID 0):
#   docker run --rm -v "$(pwd)/build/initramfs-work":/work arm64v8/alpine:3.19 sh /work/repack.sh
if command -v apk >/dev/null 2>&1; then
    apk add -q --no-cache cpio python3 2>/dev/null || true
elif command -v apt-get >/dev/null 2>&1; then
    apt-get update -qq 2>/dev/null || true
    apt-get install -y -qq cpio python3 2>/dev/null || true
fi
cd /work/rootfs
COUNT=$(find . | wc -l)
echo "Files to pack: $COUNT"
find . | cpio -H newc -o > /work/initramfs-patched.cpio

# Patch all UID/GID fields in the NEWC cpio to 0:0.
# On OrbStack/Docker Desktop the virtiofs mount exposes macOS UID 502 to
# stat() inside the container; cpio records that literal value.  Neither
# --owner=0:0 nor chown on the mount fixes it reliably.  We post-process the
# raw archive: NEWC magic "070701" + 13×8-hex fields; uid is field 3 (offset
# 6+8+8=22), gid is field 4 (offset 30) - both 8 ASCII hex digits.
python3 - /work/initramfs-patched.cpio << 'PYEOF'
import sys, re

path = sys.argv[1]
with open(path, 'rb') as f:
    data = bytearray(f.read())

MAGIC = b'070701'
pos = 0
patched = 0
while pos < len(data) - 110:
    if data[pos:pos+6] != MAGIC:
        pos += 1
        continue
    # uid at offset +22 (8 bytes), gid at offset +30 (8 bytes)
    data[pos+22:pos+30] = b'00000000'   # uid = 0
    data[pos+30:pos+38] = b'00000000'   # gid = 0
    patched += 1
    # advance past this header: read namesize and filesize to skip entry
    try:
        namesize = int(data[pos+94:pos+102], 16)
        filesize = int(data[pos+54:pos+62], 16)
        # header is 110 bytes, then name padded to 4, then data padded to 4
        header_name_size = 110 + namesize
        header_name_pad = (4 - header_name_size % 4) % 4
        data_pad = (4 - filesize % 4) % 4 if filesize else 0
        pos += header_name_size + header_name_pad + filesize + data_pad
    except Exception:
        pos += 110

with open(path, 'wb') as f:
    f.write(data)
print(f'UID/GID patched to 0:0 in {patched} cpio entries')
PYEOF
echo "CPIO size: $(ls -lh /work/initramfs-patched.cpio)"
gzip -9 -f /work/initramfs-patched.cpio
echo "GZ size: $(ls -lh /work/initramfs-patched.cpio.gz)"
echo "REPACK_DONE"
