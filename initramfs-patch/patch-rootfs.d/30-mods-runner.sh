#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 30: cdj3k-mods - runs this boot's mods before the player app starts.
#
# The host appends the slot's enabled mods to the initramfs as a second cpio:
#   /opt/cdj3k-mods/manifest          deck_model=<slug>, fw_version=<release>
#   /opt/cdj3k-mods/NN-<name>/        one mod folder each, mod.toml at its root
#   /opt/cdj3k-mods/NN-<name>.mod     the preload and env entries of its mod.toml,
#                                     one per line
# A boot without mods has no manifest and cdj3k-mods.service is skipped.
#
# The app unit's drop-in (patch 13) starts this unit before the app, and reads
# /run/cdj3k-mods/preload.env, which the runner writes.
#
# Installs:
#   /usr/sbin/cdj3k-mods              the runner
#   /usr/lib/cdj3k-mods/bin/          mod-preload, mod-env (on each loader.sh's PATH)
#   cdj3k-mods.service                oneshot, skipped without a manifest
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"
: "${APP_UNIT:?APP_UNIT must be set by dispatcher}"

mkdir -p "$ROOTFS/opt/cdj3k-mods" "$ROOTFS/usr/lib/cdj3k-mods/bin"

# ---- /usr/sbin/cdj3k-mods ----
cat > "$ROOTFS/usr/sbin/cdj3k-mods" << 'SCRIPTEOF'
#!/bin/sh
# cdj3k-mods - loads each mod once, in folder order, before the player app
# starts.
#
# For each mod, the libraries and variables from its mod.toml (NN-<name>.mod)
# are added first. Then its loader.sh, if it has one, runs as the transient
# unit cdj3k-mod-<name>.service for at most 60 s, and can add more.
#
# Writes, under /run/cdj3k-mods:
#   preload.env   LD_PRELOAD (deck_shim.so first), then the mods' variables
#   status        per mod: mod <name> script <rc|timeout|none> libs <n>

MODS=/opt/cdj3k-mods
RUN=/run/cdj3k-mods
SHIM=/home/root/deck_shim.so
BIN=/usr/lib/cdj3k-mods/bin
APP_UNIT=@APP_UNIT@

[ -f "$MODS/manifest" ] || exit 0

DECK_MODEL=
FW_VERSION=
while IFS='=' read -r k v; do
    case "$k" in
        deck_model) DECK_MODEL=$v ;;
        fw_version) FW_VERSION=$v ;;
    esac
done < "$MODS/manifest"

rm -rf "$RUN"
mkdir -p "$RUN/mods"

# Prints why KEY=VALUE cannot go into the app's EnvironmentFile= unchanged,
# and returns 0; returns 1 when it can.
env_refused() {
    case "$1" in *=*) ;; *) echo "not KEY=VALUE"; return 0 ;; esac
    k=${1%%=*}
    case "$k" in
        ''|[0-9]*|*[!A-Za-z0-9_]*) echo "not a variable name"; return 0 ;;
        LD_PRELOAD) echo "use mod-preload for libraries"; return 0 ;;
    esac
    case "${1#*=}" in
        *'\'*|*'"'*|*"'"*|*[[:cntrl:]]*)
            echo "the value contains a quote, a backslash or a control character"; return 0 ;;
        [[:space:]]*|*[[:space:]])
            echo "the value starts or ends with a space"; return 0 ;;
    esac
    return 1
}

: > "$RUN/status.tmp"
libs=$SHIM
envs=$RUN/env.tmp
: > "$envs"
ran=0

for dir in "$MODS"/[0-9][0-9]-*/; do
    [ -d "$dir" ] || continue
    dir=${dir%/}
    name=${dir##*/}
    name=${name#[0-9][0-9]-}
    case "$name" in
        ''|*[!a-z0-9._-]*) echo "skipping $dir: not a mod name"; continue ;;
    esac
    preload=$RUN/mods/$name.preload
    : > "$preload"
    : > "$RUN/mods/$name.env"
    ran=1
    if [ -f "$dir.mod" ]; then
        while IFS= read -r line; do
            case "$line" in
                "preload "*) printf '%s\n' "${line#preload }" >> "$preload" ;;
                "env "*) printf '%s\n' "${line#env }" >> "$RUN/mods/$name.env" ;;
            esac
        done < "$dir.mod"
    fi

    if [ -f "$dir/loader.sh" ]; then
        out=$(systemd-run --wait --collect --unit="cdj3k-mod-$name.service" \
            -p RuntimeMaxSec=60 -p TimeoutStopSec=5 -p WorkingDirectory="$dir" \
            --setenv=MOD_DIR="$dir" --setenv=MOD_PRELOAD="$preload" \
            --setenv=MOD_BIN="$BIN" --setenv=PATH="$BIN:/usr/sbin:/usr/bin:/sbin:/bin" \
            --setenv=DECK_MODEL="$DECK_MODEL" --setenv=APP_UNIT="$APP_UNIT" \
            --setenv=FW_VERSION="$FW_VERSION" \
            /bin/sh "$dir/loader.sh" 2>&1)
        rc=$?
        case "$out" in
            *"Finished with result: timeout"*) rc=timeout
                echo "$name: loader.sh stopped after 60 s" ;;
            *) echo "$name: loader.sh exited with status $rc" ;;
        esac
        # What systemd-run itself said, when the loader did not succeed.
        if [ "$rc" != 0 ]; then
            printf '%s\n' "$out" | while IFS= read -r l; do
                echo "$name: systemd-run: $l"
            done
        fi
    else
        rc=none
        echo "$name: no loader.sh"
    fi

    n=0
    while IFS= read -r so; do
        # LD_PRELOAD separates libraries with ':' and spaces.
        case "$so" in
            /*) ;;
            *) printf '%s: preload: %s dropped: not an absolute path\n' "$name" "$so"; continue ;;
        esac
        case "$so" in
            *[[:space:]:\\\"\']*)
                printf '%s: preload: %s dropped: contains a space, a colon, a quote or a backslash\n' \
                    "$name" "$so"
                continue ;;
        esac
        printf '%s: preload: %s\n' "$name" "$so"
        libs="$libs:$so"
        n=$((n + 1))
    done < "$preload"
    while IFS= read -r kv; do
        if why=$(env_refused "$kv"); then
            echo "$name: env: ${kv%%=*} dropped: $why"
        else
            echo "$name: env: $kv"
            printf '%s\n' "$kv" >> "$envs"
        fi
    done < "$RUN/mods/$name.env"
    echo "mod $name script $rc libs $n" >> "$RUN/status.tmp"
done

{
    echo "LD_PRELOAD=$libs"
    cat "$envs"
} > "$RUN/preload.env.tmp"
mv "$RUN/preload.env.tmp" "$RUN/preload.env"
rm -f "$envs"

# Reload systemd so that units and drop-ins a mod installed take effect.
[ "$ran" = 1 ] && systemctl daemon-reload

mv "$RUN/status.tmp" "$RUN/status"
exit 0
SCRIPTEOF
sed -i "s|@APP_UNIT@|${APP_UNIT}|" "$ROOTFS/usr/sbin/cdj3k-mods"
chmod 755 "$ROOTFS/usr/sbin/cdj3k-mods"

# ---- helpers on each loader.sh's PATH ----
cat > "$ROOTFS/usr/lib/cdj3k-mods/bin/mod-preload" << 'SCRIPTEOF'
#!/bin/sh
# mod-preload <library.so>… - loads the libraries into the player app, after
# deck_shim.so and the libraries of earlier mods.
[ -n "$MOD_PRELOAD" ] || { echo "mod-preload: run it from a mod's loader.sh" >&2; exit 2; }
for so; do
    case "$so" in /*) ;; *) so=$PWD/$so ;; esac
    case "$so" in
        *[[:space:]:\\\"\']*)
            echo "mod-preload: $so: a library path cannot contain a space, a colon, a quote or a backslash" >&2
            exit 1 ;;
    esac
    [ -f "$so" ] || { echo "mod-preload: $so: no such file" >&2; exit 1; }
    printf '%s\n' "$so" >> "$MOD_PRELOAD"
done
SCRIPTEOF

cat > "$ROOTFS/usr/lib/cdj3k-mods/bin/mod-env" << 'SCRIPTEOF'
#!/bin/sh
# mod-env KEY=VALUE… - sets variables in the player app's environment.
[ -n "$MOD_PRELOAD" ] || { echo "mod-env: run it from a mod's loader.sh" >&2; exit 2; }
nl='
'
for kv; do
    k=${kv%%=*}
    case "$kv" in *=*) ;; *) echo "mod-env: $kv: expected KEY=VALUE" >&2; exit 1 ;; esac
    case "${kv#*=}" in
        *"$nl"*|*'\'*|*'"'*|*"'"*|*[[:cntrl:]]*)
            echo "mod-env: $k: the value contains a quote, a backslash or a control character" >&2; exit 1 ;;
        [[:space:]]*|*[[:space:]])
            echo "mod-env: $k: the value starts or ends with a space" >&2; exit 1 ;;
    esac
    case "$k" in
        ''|[0-9]*|*[!A-Za-z0-9_]*) echo "mod-env: $k: not a variable name" >&2; exit 1 ;;
        LD_PRELOAD) echo "mod-env: use mod-preload for libraries" >&2; exit 1 ;;
    esac
    printf '%s\n' "$kv" >> "${MOD_PRELOAD%.preload}.env"
done
SCRIPTEOF
chmod 755 "$ROOTFS/usr/lib/cdj3k-mods/bin/mod-preload" "$ROOTFS/usr/lib/cdj3k-mods/bin/mod-env"

# ---- cdj3k-mods.service ----
cat > "$ROOTFS/etc/systemd/system/cdj3k-mods.service" << 'SVCEOF'
[Unit]
Description=cdj3k mods - loads each mod before the player app
ConditionPathExists=/opt/cdj3k-mods/manifest

[Service]
Type=oneshot
RemainAfterExit=yes
# Each loader.sh stops within 65 s, and a slot boots at most 10 mods. The
# player app starts once this unit has finished or timed out.
TimeoutStartSec=12min
ExecStart=/usr/sbin/cdj3k-mods
SVCEOF
chmod 644 "$ROOTFS/etc/systemd/system/cdj3k-mods.service"

echo "  -> cdj3k-mods runner, mod-preload/mod-env and cdj3k-mods.service installed"
