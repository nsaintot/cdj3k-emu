#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# linux-installed-check.sh - run inside a container where the deb or the rpm
# was just installed: the bundled QEMU starts and the emulator's libraries
# all resolve.
set -euo pipefail
for tool in qemu-system-aarch64 qemu-img; do
    /opt/cdj3k-emu/bin/"$tool" --version
done
if ldd /opt/cdj3k-emu/bin/cdj3k-emu | grep "not found"; then
    echo "ERROR: unresolved libraries" >&2
    exit 1
fi
echo "installed package OK"
