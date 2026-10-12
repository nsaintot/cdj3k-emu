#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# tag-notes.sh TAG - print the message of the annotated tag TAG, with LF line
# endings and without a tag signature. The message holds the release notes for
# users. A lightweight tag or an empty message prints nothing, and
# release-draft.sh then uses GitHub's generated notes.
#
# The message is read through the API because a checkout of a tag can lose
# the tag object.
set -euo pipefail
tag="$1"
: "${GH_REPO:?}"

ref=$(gh api "repos/$GH_REPO/git/ref/tags/$tag" --jq '"\(.object.type) \(.object.sha)"')
if [[ "${ref% *}" != tag ]]; then
    echo "::warning::$tag is a lightweight tag, so the release uses GitHub's generated notes. An annotated tag's message holds the release notes: git tag -a --cleanup=whitespace -F notes.md $tag" >&2
    exit 0
fi
notes=$(gh api "repos/$GH_REPO/git/tags/${ref#* }" --jq .message \
    | tr -d '\r' | sed '/^-----BEGIN [A-Z]* SIGNATURE-----$/,$d')
if [[ -z "${notes//[[:space:]]/}" ]]; then
    echo "::warning::the message of $tag is empty, so the release uses GitHub's generated notes" >&2
    exit 0
fi
printf '%s\n' "$notes"
