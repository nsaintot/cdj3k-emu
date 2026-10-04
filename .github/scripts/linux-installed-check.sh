#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# linux-installed-check.sh KIND - run inside a container where the deb or the
# rpm was just installed: the bundled QEMU starts, the emulator's libraries
# all resolve, and the package says it is KIND.
set -euo pipefail
kind="$1"
for tool in qemu-system-aarch64 qemu-img; do
    /opt/cdj3k-emu/bin/"$tool" --version
done
if ldd /opt/cdj3k-emu/bin/cdj3k-emu | grep "not found"; then
    echo "ERROR: unresolved libraries" >&2
    exit 1
fi
marker=$(cat /opt/cdj3k-emu/share/cdj3k-emu/package-kind)
if [[ "$marker" != "$kind" ]]; then
    echo "ERROR: package-kind is '$marker', not '$kind'" >&2
    exit 1
fi
echo "installed package OK"
