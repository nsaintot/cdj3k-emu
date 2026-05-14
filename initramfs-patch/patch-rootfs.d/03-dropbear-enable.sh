#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 07: enable SSH daemon in multi-user.target
#
# Two SSH daemons may be present depending on which initramfs base is used:
#   - dropbear  (pre-installed in build/initramfs-work/initramfs.cpio.gz)
#   - OpenSSH   (built into the Pioneer firmware extracted from Image.hvf)
#
# Prefer dropbear when available; fall back to OpenSSH sshd.socket otherwise.
# Both are ordered After=insmod-virtio-rng.service for the unit graph anchor -
# the service itself is a no-op on vanilla 6.6 (virtio_rng built-in,
# HW_RANDOM_VIRTIO auto-credits before userspace starts).
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

if [[ -f "$ROOTFS/usr/sbin/dropbear" ]]; then
    # ── dropbear path ──────────────────────────────────────────────────────────
    ln -sf /usr/lib/systemd/system/dropbear.service "$WANTS_DIR/dropbear.service"
    echo "  -> dropbear.service enabled in multi-user.target.wants"

    OVERRIDE_DIR="$ROOTFS/etc/systemd/system/dropbear.service.d"
    mkdir -p "$OVERRIDE_DIR"
    cat > "$OVERRIDE_DIR/override.conf" << 'EOF'
[Unit]
# Anchor on insmod-virtio-rng.service (no-op on vanilla 6.6).
After=insmod-virtio-rng.service
Wants=insmod-virtio-rng.service

[Service]
ExecStart=
ExecStart=/usr/sbin/dropbear -F -B
EOF
    echo "  -> dropbear override.conf: After=insmod-virtio-rng.service + blank passwords"
else
    # ── OpenSSH path (Pioneer firmware extracted from Image.hvf) ──────────────
    # sshd.socket uses socket-activation: listens on :22, spawns sshd@.service
    # on connect. sshdgenkeys.service generates host keys before any connection
    # arrives; on vanilla 6.6 the kernel CRNG is seeded before userspace so
    # getrandom() never blocks here.
    ln -sf /lib/systemd/system/sshd.socket "$WANTS_DIR/sshd.socket"
    echo "  -> sshd.socket enabled in multi-user.target.wants (OpenSSH, no dropbear found)"

    # sshdgenkeys must also run at multi-user.target so keys exist before sshd@
    # handles the first connection.
    ln -sf /lib/systemd/system/sshdgenkeys.service "$WANTS_DIR/sshdgenkeys.service"
    echo "  -> sshdgenkeys.service enabled in multi-user.target.wants"

    # Drop-in: order sshd@.service After=insmod-virtio-rng.service so
    # getrandom() during session setup doesn't block.
    SSHD_DROP="$ROOTFS/etc/systemd/system/sshd@.service.d"
    mkdir -p "$SSHD_DROP"
    cat > "$SSHD_DROP/10-entropy.conf" << 'EOF'
[Unit]
After=insmod-virtio-rng.service
Wants=insmod-virtio-rng.service
EOF
    echo "  -> sshd@.service.d/10-entropy.conf installed"

    # Patch sshd_config:
    #   UsePrivilegeSeparation sandbox  → UsePrivilegeSeparation no
    #     sandbox uses seccomp-BPF; on Pioneer 4.9 Yocto this causes sshd to
    #     hang before sending the banner (no seccomp support or syscall whitelist
    #     too restrictive).  Disabling privsep makes sshd a single process - safe
    #     in the QEMU dev environment.
    #   UseDNS yes (default) → UseDNS no
    #     prevents PTR lookup on connecting IP which would block without a resolver.
    SSHD_CFG="$ROOTFS/etc/ssh/sshd_config"
    if [[ -f "$SSHD_CFG" ]]; then
        sed -i '' 's/^UsePrivilegeSeparation.*/UsePrivilegeSeparation no/' "$SSHD_CFG"
        grep -q '^UseDNS' "$SSHD_CFG" \
            && sed -i '' 's/^UseDNS.*/UseDNS no/' "$SSHD_CFG" \
            || echo 'UseDNS no' >> "$SSHD_CFG"
        echo "  -> sshd_config: UsePrivilegeSeparation=no UseDNS=no"
    fi
fi
