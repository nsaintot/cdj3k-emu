# SPDX-License-Identifier: MIT OR Apache-2.0
# lib.sh - sourced by build.sh and stage.sh; runs under macOS's bash 3.2.

# ── Temp-file cleanup ────────────────────────────────────────────────────────
# Each step that creates a path with mktemp appends it to TMP_CLEANUP; the
# EXIT trap removes them all, including when `set -e` stops the script early.
TMP_CLEANUP=()
cleanup_tmp() {
    # The `+` expansion is needed because bash 3.2 under `set -u` rejects
    # "${TMP_CLEANUP[@]}" while the array is empty.
    for p in ${TMP_CLEANUP[@]+"${TMP_CLEANUP[@]}"}; do
        [[ -n "$p" && -e "$p" ]] && rm -rf "$p"
    done
    # An EXIT trap's last status replaces the script's `exit N`; a final
    # entry that is already gone would otherwise turn `exit 0` into 1.
    return 0
}
trap cleanup_tmp EXIT
