#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# release-draft.sh VERSION TAG BUILD DIR NOTES_MD - fill the draft release TAG
# from the packages in DIR and the release notes in NOTES_MD: write and sign
# both update feeds, write SHA256SUMS.txt, set the release text and upload
# every file in DIR. The draft is created if it does not exist; a published
# release fails the run.
#
# BUILD is the CFBundleVersion of the macOS bundle in DIR (see appcast.sh).
# The release text is the notes, a table of the downloads, and GitHub's
# generated list of pull requests in a collapsed block. When NOTES_MD is empty,
# that generated list is the notes, for the release text and the feeds.
set -euo pipefail
version="$1"
tag="$2"
build="$3"
dir="$4"
notes_md="$5"
: "${GH_REPO:?}"
script_dir="$(cd "$(dirname "$0")" && pwd)"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

draft=$(gh release view "$tag" --json isDraft --jq .isDraft 2>/dev/null || true)
if [[ "$draft" == false ]]; then
    echo "::error::$tag is published; its text and files are no longer changed" >&2
    exit 1
fi

gh api "repos/$GH_REPO/releases/generate-notes" -f tag_name="$tag" --jq .body \
    | tr -d '\r' > "$work/generated.md"
if [[ -z "$(tr -d '[:space:]' < "$notes_md")" ]]; then
    notes_md="$work/generated.md"
fi

# The update feeds, with the notes as Markdown for the app's window and as
# HTML for Sparkle's.
gh api markdown -f text="$(cat "$notes_md")" -f mode=gfm -f context="$GH_REPO" > "$work/notes.html"
"$script_dir/update-index.sh" "$version" "$tag" "$dir" "$notes_md"
"$script_dir/sign.sh" "$dir/cdj3k-emu-update.json" > "$dir/cdj3k-emu-update.json.sig"
"$script_dir/appcast.sh" "$version" "$tag" "$build" "$dir" "$work/notes.html"
rm -f "$dir/SHA256SUMS.txt"
(cd "$dir" && sha256sum -- * > "$work/SHA256SUMS.txt")
mv "$work/SHA256SUMS.txt" "$dir/"
cat "$dir/SHA256SUMS.txt"

# Prints one table row per package format, with a link per architecture.
downloads() {
    local base="https://github.com/$GH_REPO/releases/download/$tag"
    local row ext host path name arch links
    echo "| Host | Package |"
    echo "|---|---|"
    for row in "dmg:macOS 15 or later" "exe:Windows 10 2004 or later" \
        "deb:Debian, Ubuntu" "rpm:Fedora, RHEL, openSUSE" "AppImage:Any Linux distribution"; do
        ext="${row%%:*}"
        host="${row#*:}"
        links=""
        for path in "$dir/CDJ3K-Emulator-$version-"*".$ext"; do
            [[ -e "$path" ]] || continue
            name=$(basename "$path")
            arch="${name#"CDJ3K-Emulator-$version-"*-}"
            arch="${arch%".$ext"}"
            links+="${links:+ · }[$arch]($base/$name)"
        done
        if [[ -n "$links" ]]; then
            echo "| $host | $links |"
        fi
    done
}

{
    cat "$notes_md"
    echo
    echo "## Downloads"
    echo
    downloads
    if [[ "$notes_md" != "$work/generated.md" ]]; then
        echo
        echo "<details>"
        echo "<summary>Pull requests</summary>"
        echo
        cat "$work/generated.md"
        echo
        echo "</details>"
    fi
} > "$work/body.md"

if [[ -n "$draft" ]]; then
    gh release edit "$tag" --notes-file "$work/body.md" >/dev/null
else
    args=(--draft --verify-tag --title "$tag" --notes-file "$work/body.md")
    if [[ "$version" == *-* ]]; then args+=(--prerelease); fi
    gh release create "$tag" "${args[@]}" >/dev/null
fi
(cd "$dir" && gh release upload "$tag" --clobber -- *)
