#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 05: autologin root on the serial console.
#
# The guest runs a getty on ttyAMA0 that asks for root's password.  With
# ENABLE_SSH=1, patch 04 clears that password and pam_unix rejects an empty
# one without `nullok`, so the console cannot log in; an --autologin override
# skips authentication and makes the console a root shell.
#
# This is what traces the guest: the emulator can put that console on a unix
# socket (CDJ3K_SERIAL_SOCKET=1), which gives a scriptable shell for strace and
# /proc without a debugger or an sshd.
#
# Applied only with ENABLE_SSH=1, like patches 03 and 04; a default build keeps
# the stock password prompt.  The console is reachable only through the
# instance's own socket directory.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

if [[ "${ENABLE_SSH:-0}" != "1" ]]; then
    echo "  -> SSH disabled (ENABLE_SSH != 1) - leaving the serial getty stock"
    exit 0
fi

UNIT_DIR="$ROOTFS/etc/systemd/system/serial-getty@ttyAMA0.service.d"
mkdir -p "$UNIT_DIR"

cat > "$UNIT_DIR/autologin.conf" << 'CONFEOF'
[Service]
ExecStart=
ExecStart=-/sbin/agetty --autologin root --noclear %I 115200 linux
CONFEOF
chmod 644 "$UNIT_DIR/autologin.conf"

echo "  -> serial-getty@ttyAMA0 autologin root"
