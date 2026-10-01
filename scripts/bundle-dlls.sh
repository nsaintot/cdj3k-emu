#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# bundle-dlls.sh - copy the DLLs a set of Windows binaries load into the
# directory the binaries are in.
#
#   bundle-dlls.sh <dest dir> <search dir>... -- <binary>...
#
# Reads each binary's import table (objdump), finds every DLL that is not part
# of Windows in the search directories, copies it into <dest dir> and follows
# its own imports the same way. Windows resolves a DLL beside the executable
# first, so the tree runs from wherever it is copied to.
#
# Fails when an import is neither a Windows DLL nor in a search directory.
# Works in an MSYS2 shell and in the llvm-mingw Docker image.
set -euo pipefail

DEST="${1:?Usage: $0 <dest dir> <search dir>... -- <binary>...}"; shift
SEARCH=()
while [[ $# -gt 0 && "$1" != "--" ]]; do SEARCH+=("$1"); shift; done
[[ "${1:-}" == "--" ]] || { echo "ERROR: missing -- before the binaries" >&2; exit 1; }
shift
[[ ${#SEARCH[@]} -gt 0 && $# -gt 0 ]] || { echo "ERROR: no search dirs or no binaries" >&2; exit 1; }

OBJDUMP=""
for o in llvm-objdump objdump; do
    if command -v "$o" >/dev/null; then OBJDUMP="$o"; break; fi
done
[[ -n "$OBJDUMP" ]] || { echo "ERROR: neither llvm-objdump nor objdump found" >&2; exit 1; }

lower() { printf '%s' "$1" | tr '[:upper:]' '[:lower:]'; }

# DLLs that ship with Windows (or with the driver/GPU stack); never vendored.
is_windows_dll() {
    case "$(lower "$1")" in
        api-ms-win-*|ext-ms-*|kernel32.dll|kernelbase.dll|user32.dll|gdi32.dll|\
        advapi32.dll|shell32.dll|ole32.dll|oleaut32.dll|combase.dll|rpcrt4.dll|\
        ws2_32.dll|wsock32.dll|mswsock.dll|iphlpapi.dll|dnsapi.dll|netapi32.dll|\
        ntdll.dll|msvcrt.dll|ucrtbase.dll|bcrypt.dll|ncrypt.dll|crypt32.dll|\
        secur32.dll|sspicli.dll|winmm.dll|imm32.dll|setupapi.dll|cfgmgr32.dll|\
        version.dll|shlwapi.dll|dbghelp.dll|comdlg32.dll|comctl32.dll|\
        userenv.dll|mpr.dll|psapi.dll|powrprof.dll|uxtheme.dll|dwmapi.dll|\
        normaliz.dll|hid.dll|wtsapi32.dll|avrt.dll|opengl32.dll|dxgi.dll|\
        d3d11.dll|d3d9.dll|winhvplatform.dll|winhvemulation.dll|wldap32.dll|\
        mfplat.dll|winspool.drv|shcore.dll|cabinet.dll|msvcp_win.dll|\
        bcryptprimitives.dll|newdev.dll|propsys.dll|rstrtmgr.dll|virtdisk.dll)
            return 0 ;;
    esac
    return 1
}

find_dll() { # <name> -> path of the file in a search dir, matched ignoring case
    local want dir f
    want="$(lower "$1")"
    for dir in "${SEARCH[@]}"; do
        [[ -d "$dir" ]] || continue
        for f in "$dir"/*; do
            [[ -f "$f" ]] || continue
            if [[ "$(lower "$(basename "$f")")" == "$want" ]]; then
                printf '%s' "$f"
                return 0
            fi
        done
    done
    return 1
}

imports() { # <file> -> one DLL name per line
    "$OBJDUMP" -p "$1" | sed -n 's/^[[:space:]]*DLL Name: *//p' | tr -d '\r'
}

mkdir -p "$DEST"

copied=()
missing=()
queue=("$@")
seen=" "
while [[ ${#queue[@]} -gt 0 ]]; do
    target="${queue[0]}"
    queue=("${queue[@]:1}")
    [[ -f "$target" ]] || { echo "ERROR: $target does not exist" >&2; exit 1; }

    while IFS= read -r name; do
        [[ -n "$name" ]] || continue
        is_windows_dll "$name" && continue
        key="$(lower "$name")"
        case "$seen" in *" $key "*) continue ;; esac
        seen="$seen$key "
        # Already beside the binaries (the build put it there).
        if [[ -f "$DEST/$name" ]]; then
            queue+=("$DEST/$name")
            continue
        fi
        if path="$(find_dll "$name")"; then
            cp -L "$path" "$DEST/$name"
            chmod u+w "$DEST/$name"
            copied+=("$name")
            queue+=("$DEST/$name")
        else
            missing+=("$name")
        fi
    done < <(imports "$target")
done

if [[ ${#missing[@]} -gt 0 ]]; then
    echo "ERROR: these DLLs are imported but were not found in:" >&2
    printf '         %s\n' "${SEARCH[@]}" >&2
    echo "       and are not part of Windows:" >&2
    printf '         %s\n' "${missing[@]}" >&2
    exit 1
fi

echo "     bundled ${#copied[@]} DLLs into ${DEST}"
if [[ ${#copied[@]} -gt 0 ]]; then
    printf '       %s\n' "${copied[@]}" | sort
fi
