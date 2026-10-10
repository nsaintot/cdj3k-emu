#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Print a buildx builder name that supports --cache-to type=local.
#
# The default "docker" driver cannot export a local cache ("Cache export is
# not supported for the docker driver"). A builder with the docker-container
# driver can; the script creates it on first use.
set -euo pipefail

BUILDER="${CDJ3K_BUILDX_BUILDER:-cdj3k-emu}"

if ! docker buildx inspect "$BUILDER" >/dev/null 2>&1; then
    docker buildx create \
        --name "$BUILDER" \
        --driver docker-container \
        --driver-opt default-load=true \
        >/dev/null
fi

printf '%s\n' "$BUILDER"
