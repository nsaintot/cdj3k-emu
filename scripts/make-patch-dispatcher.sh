#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# make-patch-dispatcher.sh - concatenate initramfs-patch/patch-rootfs.d/*.sh
# into the single patch-rootfs.sh a package ships.
#
#   make-patch-dispatcher.sh <patch-rootfs.d dir> <output file>
#
# The repo keeps the steps as numbered scripts so a failure names one; a
# package ships the one file this makes.
set -euo pipefail

SRC_DIR="${1:?Usage: $0 <patch-rootfs.d dir> <output file>}"
OUT="${2:?Usage: $0 <patch-rootfs.d dir> <output file>}"

shopt -s nullglob
STEPS=("$SRC_DIR"/[0-9]*.sh)
shopt -u nullglob
if [[ ${#STEPS[@]} -eq 0 ]]; then
    echo "ERROR: no numbered steps in $SRC_DIR" >&2
    exit 1
fi

mkdir -p "$(dirname "$OUT")"
{
    cat <<'HDR'
#!/usr/bin/env bash
# patch-rootfs.sh - auto-generated bundle dispatcher.
# Concatenation of every initramfs-patch/patch-rootfs.d/*.sh in numeric order.
# Source: kept modular in the repo at initramfs-patch/patch-rootfs.d/.
set -euo pipefail
ROOTFS="${1:?Usage: $0 <initramfs-root>}"
export ROOTFS
export PATCH_ASSETS_DIR="$(cd "$(dirname "$0")" && pwd)"
# The player application's unit, detected from the rootfs: EP122.service on
# the CDJ-3000, EP145.service on the CDJ-3000X.
APP_UNIT="$(cd "$ROOTFS/etc/systemd/system" 2>/dev/null && ls EP1[0-9][0-9].service 2>/dev/null | head -n1 || true)"
export APP_UNIT="${APP_UNIT:-EP122.service}"
export APP_NAME="${APP_UNIT%.service}"
# The slug the emulator names this player by, from the unit the rootfs carries.
case "$APP_UNIT" in
    EP145.service) export APP_SLUG="cdj3kx" ;;
    *)             export APP_SLUG="cdj3k" ;;
esac
# SSH (passwordless root) is off by default. The setup window asks for it
# with `cdj3k.ssh=1` on the provisioning boot's kernel command line; a
# host-side run can pass ENABLE_SSH=1 in the environment.
case " $(cat /proc/cmdline 2>/dev/null) " in
    *" cdj3k.ssh=1 "*) ENABLE_SSH=1 ;;
esac
export ENABLE_SSH="${ENABLE_SSH:-0}"
echo "=== Patching initramfs rootfs at: $ROOTFS (app unit: $APP_UNIT) ==="
HDR
    for step in "${STEPS[@]}"; do
        printf '\necho "--- %s ---"\n(\n' "$(basename "$step")"
        # Strip per-step shebang and `set -euo pipefail` (already set above).
        sed -E '1{/^#!/d;}; /^set -euo pipefail$/d' "$step"
        printf ')\n'
    done
    printf '\necho "=== All patches applied ==="\n'
} > "$OUT"
chmod +x "$OUT"
echo "     patch-rootfs.sh: ${#STEPS[@]} steps inlined"
