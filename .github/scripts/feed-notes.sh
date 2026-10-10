#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# feed-notes.sh < BODY.md > NOTES.md - convert the release notes to the form
# the update feeds carry: LF line endings and no Downloads section.
#
# The section runs from a "Downloads" heading to the next heading or the
# "**Full Changelog**" line. Runs of blank lines left behind collapse into one.
set -euo pipefail
tr -d '\r' | awk '
    /^#+[[:space:]]*Downloads[[:space:]]*$/ { skip = 1; next }
    skip && (/^#/ || /^\*\*Full Changelog\*\*/) { skip = 0 }
    skip { next }
    /^[[:space:]]*$/ { if (blank++) next; print; next }
    { blank = 0; print }
'
