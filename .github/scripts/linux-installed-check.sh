#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# linux-installed-check.sh KIND - run inside a container where the deb or the
# rpm was just installed. It checks that the bundled QEMU starts, that all of
# the emulator's libraries resolve, and that the package reports KIND.
set -euo pipefail
kind="$1"
for tool in qemu-system-aarch64 qemu-img; do
    /opt/cdj3k-emu/bin/"$tool" --version
done
if ldd /opt/cdj3k-emu/bin/cdj3k-emu | grep "not found"; then
    echo "ERROR: some of the emulator's libraries do not resolve." >&2
    exit 1
fi
marker=$(cat /opt/cdj3k-emu/share/cdj3k-emu/package-kind)
if [[ "$marker" != "$kind" ]]; then
    echo "ERROR: package-kind is '$marker'; expected '$kind'." >&2
    exit 1
fi
echo "The installed package passed the checks."
