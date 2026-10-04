#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# feed-notes.sh < BODY.md > NOTES.md - the release's notes as the update feeds
# carry them: LF line ends, without the Downloads section.
#
# The section runs from a "Downloads" heading to the next heading or the
# "**Full Changelog**" line; blank runs it leaves collapse to one.
set -euo pipefail
tr -d '\r' | awk '
    /^#+[[:space:]]*Downloads[[:space:]]*$/ { skip = 1; next }
    skip && (/^#/ || /^\*\*Full Changelog\*\*/) { skip = 0 }
    skip { next }
    /^[[:space:]]*$/ { if (blank++) next; print; next }
    { blank = 0; print }
'
