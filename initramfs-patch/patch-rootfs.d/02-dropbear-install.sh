#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 02: install dropbear where the firmware lacks it, and fix the host key.
#
# Most rootfs images ship /usr/sbin/dropbear with its unit, disabled. A rootfs
# with no SSH server (OpenSSH binaries removed, only /etc/ssh left; the
# CDJ-1500X) gets the bundled dropbear_aarch64 (dropbear/build.sh) and a unit,
# so every rootfs carries the same disabled daemon that 03-dropbear-enable.sh
# turns on.
#
# With ENABLE_SSH=1 the host key is made usable: a firmware ECDSA key is
# mode 644, which dropbear refuses, so every key is tightened to 600; a
# rootfs without one gets an ed25519 key, generated here so it stays the
# same across boots.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"
: "${PATCH_ASSETS_DIR:?PATCH_ASSETS_DIR must be set by dispatcher}"
: "${APP_SLUG:?APP_SLUG must be set by dispatcher}"

if [[ "$APP_SLUG" == cdj1500x ]]; then
    # dropbear_aarch64 is in PATCH_ASSETS_DIR in a bundle, or in guest/out/ in
    # a dev checkout.
    if [[ -f "$PATCH_ASSETS_DIR/dropbear_aarch64" ]]; then
        BIN="$PATCH_ASSETS_DIR/dropbear_aarch64"
    elif [[ -f "$PATCH_ASSETS_DIR/../guest/out/dropbear_aarch64" ]]; then
        BIN="$PATCH_ASSETS_DIR/../guest/out/dropbear_aarch64"
    else
        echo "ERROR: dropbear_aarch64 not found in $PATCH_ASSETS_DIR or guest/out/" >&2
        echo "       Run: ./build.sh" >&2
        exit 1
    fi
    install -m 0755 "$BIN" "$ROOTFS/usr/sbin/dropbear"
    # One multi-call binary; the name it is run by picks the program.
    for prog in dropbearkey dbclient scp; do
        ln -sf ../sbin/dropbear "$ROOTFS/usr/bin/$prog"
    done
    echo "  -> /usr/sbin/dropbear installed (bundled), with dropbearkey dbclient scp"

    cat > "$ROOTFS/usr/lib/systemd/system/dropbear.service" << 'EOF'
[Unit]
Description=Dropbear SSH daemon
After=network.target

[Service]
ExecStart=/usr/sbin/dropbear -F -R
ExecReload=/bin/kill -HUP $MAINPID

[Install]
WantedBy=multi-user.target
EOF
    chmod 644 "$ROOTFS/usr/lib/systemd/system/dropbear.service"
    echo "  -> dropbear.service written"
fi

if [[ "${ENABLE_SSH:-0}" != "1" ]]; then
    echo "  -> SSH disabled (ENABLE_SSH != 1) - leaving the host key alone"
    exit 0
fi

KEY_DIR="$ROOTFS/etc/dropbear"
mkdir -p "$KEY_DIR"
shopt -s nullglob
keys=("$KEY_DIR"/dropbear_*_host_key)
if [[ ${#keys[@]} -gt 0 ]]; then
    chmod 600 "${keys[@]}"
    echo "  -> host keys set to 600: ${keys[*]##*/}"
    exit 0
fi

# dropbearkey runs in the aarch64 provisioning guest; where it cannot run,
# dropbear -R generates the key at first connection.
KEY="$KEY_DIR/dropbear_ed25519_host_key"
if "$ROOTFS/usr/bin/dropbearkey" -t ed25519 -f "$KEY" > /dev/null 2>&1; then
    chmod 600 "$KEY"
    echo "  -> ${KEY##*/} generated"
else
    rm -f "$KEY"
    echo "  -> host key left to dropbear -R (dropbearkey cannot run here)"
fi
