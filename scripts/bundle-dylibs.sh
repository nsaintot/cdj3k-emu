#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# bundle-dylibs.sh - copy a macOS .app's non-system dylib dependencies into the
# bundle.
#
# Usage:
#     bundle-dylibs.sh <macos_dir> <root> [<root> ...]
#
# <macos_dir>   the bundle's Contents/MacOS directory (where the binaries live).
# <root>...     binaries/dylibs already inside <macos_dir> whose dependency graph
#               should be bundled (e.g. cdj3k-emu, qemu-img,
#               libcdj3k-emu-qemu.dylib).
#
# For each root it walks the transitive `otool -L` dependency graph, copies every
# non-system dylib (anything outside /usr/lib and /System, such as the ones in
# /opt/homebrew or /usr/local) flat into <macos_dir>, and rewrites every load
# command (and each copied dylib's own id) to @loader_path/<basename> so the app
# runs on a machine without Homebrew. Editing load commands invalidates a
# signature, so each edited Mach-O is re-signed ad hoc; packaging/macos/build.sh
# later signs everything with the final identity.
#
# System libraries (/usr/lib, /System) are left untouched because every Mac has
# them. A dependency that is already @loader_path/<name>, with the file in
# <macos_dir>, counts as bundled and is skipped, so a second run changes
# nothing.
#
# Needs only what ships with the Xcode Command Line Tools: otool,
# install_name_tool, codesign. Runs on the stock /bin/bash 3.2.
set -euo pipefail

if [[ $# -lt 2 ]]; then
    sed -n '3,/^$/p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
fi

MACOS_DIR="$(cd "$1" && pwd)"
shift

is_system() { [[ "$1" == /usr/lib/* || "$1" == /System/* ]]; }

# LC_ID_DYLIB install name of a dylib; empty for a plain executable.
otool_id() { otool -D "$1" 2>/dev/null | sed -n '2p'; }

# Dependency install names from `otool -L` (the first line is the file itself).
otool_deps() { otool -L "$1" | tail -n +2 | awk '{print $1}'; }

# LC_RPATH entries of a Mach-O, one per line.
otool_rpaths() {
    otool -l "$1" | awk '/ cmd LC_RPATH/{r=1; next} r && / path /{print $2; r=0}'
}

# in_list <needle> <haystack...>
in_list() {
    local x="$1"; shift
    local e
    for e in "$@"; do [[ "$e" == "$x" ]] && return 0; done
    return 1
}

# resolve <install_name> <owner> <origin_dir>: print the source file for a
# dependency, or nothing if it cannot be found.
#
# origin_dir is the directory the owner was copied from (its own directory for
# roots). @loader_path and @rpath siblings are resolved there: Homebrew dylibs
# often name a sibling @rpath/libfoo without an rpath of their own (they rely on
# the loading executable's rpath), and the sibling lives next to the original
# file, not next to the copy.
resolve() {
    local dep="$1" owner="$2" origin="$3" tail rp cand
    case "$dep" in
        @loader_path/*)     echo "$origin/${dep#@loader_path/}" ;;
        @executable_path/*) echo "$MACOS_DIR/${dep#@executable_path/}" ;;
        @rpath/*)
            tail="${dep#@rpath/}"
            while IFS= read -r rp; do
                [[ -n "$rp" ]] || continue
                rp="${rp//@loader_path/$origin}"
                rp="${rp//@executable_path/$MACOS_DIR}"
                cand="$rp/$tail"
                if [[ -e "$cand" ]]; then echo "$cand"; return 0; fi
            done < <(otool_rpaths "$owner")
            # Fall back to the sibling next to the original, which covers most
            # Homebrew @rpath dependencies.
            cand="$origin/$tail"
            [[ -e "$cand" ]] && echo "$cand"
            ;;
        *) echo "$dep" ;;   # absolute path
    esac
}

roots=()
for r in "$@"; do
    if [[ ! -e "$MACOS_DIR/$r" ]]; then
        echo "bundle-dylibs: the root $MACOS_DIR/$r was not found." >&2
        exit 1
    fi
    roots+=("$MACOS_DIR/$r")
done

# The work list holds "file<TAB>origin_dir" entries and is read by index, so
# entries can be appended while iterating. processed and bundled are plain lists
# because bash 3.2 has no associative arrays; REWRITES is a file of
# "file<TAB>old<TAB>new" lines.
queue=()
for r in "${roots[@]}"; do queue+=("$r	$(dirname "$r")"); done
processed=()
bundled=()          # basenames now living in MACOS_DIR
REWRITES="$(mktemp -t bundle-dylibs)"
trap 'rm -f "$REWRITES"' EXIT

i=0
while [[ $i -lt ${#queue[@]} ]]; do
    f="${queue[$i]%%	*}"
    origin="${queue[$i]#*	}"
    i=$((i + 1))
    in_list "$f" "${processed[@]+"${processed[@]}"}" && continue
    processed+=("$f")
    own_id="$(otool_id "$f")"

    while IFS= read -r dep; do
        [[ -n "$dep" ]] || continue
        is_system "$dep" && continue
        [[ -n "$own_id" && "$dep" == "$own_id" ]] && continue   # the dylib's own id
        base="$(basename "$dep")"
        new_name="@loader_path/$base"
        # The dependency is already bundled: it is @loader_path/<name> and the
        # file is present.
        if [[ "$dep" == "$new_name" && -e "$MACOS_DIR/$base" ]]; then
            in_list "$base" "${bundled[@]+"${bundled[@]}"}" || bundled+=("$base")
            queue+=("$MACOS_DIR/$base	$MACOS_DIR")
            continue
        fi
        src="$(resolve "$dep" "$f" "$origin")"
        if [[ -z "$src" || ! -e "$src" ]]; then
            echo "  WARN: cannot resolve $dep (from $(basename "$f")); skipping it." >&2
            continue
        fi
        real_src="$(cd "$(dirname "$src")" && pwd -P)/$(basename "$src")"
        is_system "$real_src" && continue
        dest="$MACOS_DIR/$base"
        if ! in_list "$base" "${bundled[@]+"${bundled[@]}"}"; then
            if [[ "$real_src" != "$(cd "$(dirname "$dest")" 2>/dev/null && pwd -P)/$base" ]]; then
                cp "$src" "$dest"
                chmod 755 "$dest"
            fi
            bundled+=("$base")
            queue+=("$dest	$(dirname "$real_src")")
        fi
        printf '%s\t%s\t%s\n' "$f" "$dep" "$new_name" >> "$REWRITES"
    done < <(otool_deps "$f")
done

# Apply the install_name_tool edits and re-sign every processed Mach-O ad hoc.
for f in "${processed[@]}"; do
    args=()
    base="$(basename "$f")"
    if in_list "$base" "${bundled[@]+"${bundled[@]}"}"; then   # a copied dylib: fix its own id too
        args+=(-id "@loader_path/$base")
    fi
    while IFS=$'\t' read -r file old new; do
        [[ "$file" == "$f" ]] && args+=(-change "$old" "$new")
    done < "$REWRITES"
    if [[ ${#args[@]} -gt 0 ]]; then
        install_name_tool "${args[@]}" "$f" 2>&1 | grep -v 'invalidate the code signature' || true
    fi
    codesign --force --sign - "$f" >/dev/null 2>&1
done

echo "  -> bundled ${#bundled[@]} dylib(s) into $MACOS_DIR (@loader_path, ad-hoc signed)"
