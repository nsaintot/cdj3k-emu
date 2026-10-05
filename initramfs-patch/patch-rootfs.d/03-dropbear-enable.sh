#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 03: enable the dropbear SSH daemon in multi-user.target
#
# 02-dropbear-install.sh has put dropbear and its unit in every rootfs, the
# firmware's own or the bundled one. The override is ordered
# After=insmod-virtio-rng.service for the unit graph anchor - the service
# itself is a no-op on vanilla 6.6 (virtio_rng built-in, HW_RANDOM_VIRTIO
# auto-credits before userspace starts).
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

if [[ "${ENABLE_SSH:-0}" != "1" ]]; then
    echo "  -> SSH disabled (ENABLE_SSH != 1) - skipping SSH service enable"
    exit 0
fi

WANTS_DIR="$ROOTFS/etc/systemd/system/multi-user.target.wants"
mkdir -p "$WANTS_DIR"

cat <<'BANNER'
================================================================================
  WARNING: ENABLE_SSH=1 — passwordless root SSH will be active in this guest.
  The dropbear override uses `-B` (blank-password login) and 04-root-password.sh
  clears the root hash.  Anyone who can reach the guest's SSH port can log in
  as root with no credentials.  Do NOT bridge this guest onto an untrusted
  network with SSH enabled.  ENABLE_SSH is off by default in shipped builds.
================================================================================
BANNER

ln -sf /usr/lib/systemd/system/dropbear.service "$WANTS_DIR/dropbear.service"
echo "  -> dropbear.service enabled in multi-user.target.wants"

# -R only where 02-dropbear-install.sh left no host key.
shopt -s nullglob
keys=("$ROOTFS"/etc/dropbear/dropbear_*_host_key)
FLAGS="-F -B"
[[ ${#keys[@]} -gt 0 ]] || FLAGS="-F -R -B"

OVERRIDE_DIR="$ROOTFS/etc/systemd/system/dropbear.service.d"
mkdir -p "$OVERRIDE_DIR"
cat > "$OVERRIDE_DIR/override.conf" << EOF
[Unit]
# Anchor on insmod-virtio-rng.service (no-op on vanilla 6.6).
After=insmod-virtio-rng.service
Wants=insmod-virtio-rng.service

[Service]
ExecStart=
ExecStart=/usr/sbin/dropbear $FLAGS
EOF
echo "  -> dropbear override.conf: After=insmod-virtio-rng.service, $FLAGS"
