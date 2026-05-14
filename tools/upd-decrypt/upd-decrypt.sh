#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Host wrapper: builds the cdj3k-upd-decrypt Docker image (once) and runs it.
#
# Usage:
#   ./upd-decrypt.sh <input.UPD> <output.img> [keyfile]
#
# Default keyfile is the bundled aes256.key in this folder.
# Requires Docker. Works on macOS and Linux.
#
# WARNING: This wrapper runs the helper container with `--privileged` because
# the decrypt pipeline needs `losetup` + `cryptsetup` inside the container.
# A privileged container has near-host kernel access; only feed it .UPD files
# you actually trust.  This tool is a dev/research helper, not part of the
# shipping app — the in-app firmware wizard does its own decryption in pure
# Rust (no Docker, no privileged execution) from within a user-owned sandbox.

set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
IMAGE=cdj3k-upd-decrypt:latest

usage() {
  echo "Usage: $0 <input.UPD> <output.img> [keyfile]" >&2
  echo "  Default keyfile: ${HERE}/aes256.key" >&2
  exit 1
}

[[ $# -ge 2 && $# -le 3 ]] || usage

INPUT=$1
OUTPUT=$2
KEYFILE=${3:-${HERE}/aes256.key}

[[ -f $INPUT  ]] || { echo "input not found: $INPUT" >&2; exit 1; }
[[ -f $KEYFILE ]] || { echo "keyfile not found: $KEYFILE" >&2; exit 1; }
command -v docker >/dev/null || { echo "docker not installed" >&2; exit 1; }

# Build image if it doesn't already exist.
if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo "[+] building $IMAGE..."
  docker build -t "$IMAGE" "$HERE"
fi

# Resolve absolute paths for bind mounts.
ABS_INPUT=$(cd "$(dirname "$INPUT")"   && pwd)/$(basename "$INPUT")
ABS_KEY=$(cd "$(dirname "$KEYFILE")"   && pwd)/$(basename "$KEYFILE")
mkdir -p "$(dirname "$OUTPUT")"
ABS_OUT_DIR=$(cd "$(dirname "$OUTPUT")" && pwd)
OUT_NAME=$(basename "$OUTPUT")

echo "[+] running container..."
docker run --rm --privileged \
  -v "$ABS_INPUT:/in/upd:ro" \
  -v "$ABS_KEY:/in/key:ro" \
  -v "$ABS_OUT_DIR:/out" \
  -e OUT_NAME="$OUT_NAME" \
  "$IMAGE"

echo "[+] done: $OUTPUT"
