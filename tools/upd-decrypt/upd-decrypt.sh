#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# This host wrapper builds the cdj3k-upd-decrypt Docker image once and runs it.
#
# Usage:
#   ./upd-decrypt.sh <input.UPD> <output.img> [keyfile]
#
# The default keyfile is aes256.key in this folder, which is not part of the
# repository.
# The wrapper requires Docker and works on macOS and Linux.
#
# WARNING: This wrapper runs the helper container with `--privileged` because
# the decrypt pipeline needs `losetup` and `cryptsetup` inside the container.
# A privileged container has near-host kernel access; only feed it .UPD files
# you actually trust.  This tool is a development and research helper.  The
# app's firmware wizard decrypts updates itself, in Rust, without Docker or
# privileges.

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

[[ -f $INPUT  ]] || { echo "The input $INPUT was not found." >&2; exit 1; }
[[ -f $KEYFILE ]] || { echo "The keyfile $KEYFILE was not found." >&2; exit 1; }
command -v docker >/dev/null || { echo "docker is not installed." >&2; exit 1; }

if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo "[+] building $IMAGE..."
  docker build -t "$IMAGE" "$HERE"
fi

# Resolve absolute paths for the bind mounts.
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
