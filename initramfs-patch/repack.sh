#!/bin/sh
# SPDX-License-Identifier: MIT OR Apache-2.0
# POSIX sh has no pipefail, so set -e sees only the exit status of the last
# command in a pipeline: a find failure in `find | cpio` goes unnoticed.
set -eu
# Install cpio and python3 if they are missing (Alpine apk or Debian apt-get).
#
# Do not repack with macOS bsdcpio directly:
#   macOS bsdcpio stores the macOS user's UID (for example 502) in the cpio
#   headers instead of root (0). The Linux kernel unpacks the files with that
#   UID, so dropbear sees the wrong owner on /etc/shadow and the host keys and
#   falls back to password authentication.
#
# Repack in Docker. The Python step below sets every UID and GID to 0,
# because the virtiofs mount can expose the macOS UID. Run it with:
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

# Set every UID and GID field in the NEWC cpio to 0.
# On OrbStack and Docker Desktop the virtiofs mount exposes macOS UID 502 to
# stat() inside the container, and cpio records that value. Neither
# --owner=0:0 nor chown on the mount fixes it reliably, so the script
# post-processes the raw archive. A NEWC header is the magic "070701"
# followed by thirteen fields of 8 ASCII hex digits; uid is field 3 at offset
# 22 (6+8+8) and gid is field 4 at offset 30.
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
    data[pos+22:pos+30] = b'00000000'
    data[pos+30:pos+38] = b'00000000'
    patched += 1
    # Skip the rest of the entry, using namesize and filesize.
    try:
        namesize = int(data[pos+94:pos+102], 16)
        filesize = int(data[pos+54:pos+62], 16)
        # The 110-byte header and the name are padded together to a multiple
        # of 4 bytes, and the data is padded to a multiple of 4.
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
