#!/bin/sh
# Optional. Runs as root from this folder once per boot, before the player app
# starts and after the preload and env from mod.toml are applied. It is
# stopped after 60 s, along with every process it started. See docs/mods.md.
#
# The examples below are commented out; uncomment the ones the mod needs.
set -e
echo "example: $DECK_MODEL $FW_VERSION, app unit $APP_UNIT"

# A library chosen by model and firmware. mod-preload adds it after
# deck_shim.so and the libraries of the mods above this one; a relative path
# starts from this folder.
# case "$DECK_MODEL-$FW_VERSION" in
#     cdj3k-3.19) mod-preload lib/example-3.19.so ;;
#     cdj3k-*)    mod-preload lib/example.so ;;
# esac

# A variable for the player app, with the same rules as env in mod.toml.
# mod-env EXAMPLE_LEVEL=2

# The libraries already added: one file per mod above this one, and this
# mod's own, with one path per line.
# cat /run/cdj3k-mods/mods/*.preload

# A process that keeps running after loader.sh, as its own unit.
# cat > /run/systemd/system/example-daemon.service <<UNIT
# [Service]
# ExecStart=$MOD_DIR/bin/example-daemon
# Restart=on-failure
# UNIT
# systemctl daemon-reload
# systemctl --no-block start example-daemon.service

# A library loaded into another process: a drop-in on its unit. A unit that
# is already running gets it when restarted.
# mkdir -p /run/systemd/system/other.service.d
# cat > /run/systemd/system/other.service.d/50-example.conf <<UNIT
# [Service]
# Environment=LD_PRELOAD=$MOD_DIR/lib/example-other.so
# UNIT
# systemctl daemon-reload
# systemctl try-restart other.service

# A library loaded before deck_shim.so. The player app reads LD_PRELOAD from
# /run/cdj3k-mods/preload.env; a drop-in whose name sorts after 10-qemu.conf
# can rewrite that line into its own file just before the app starts.
# mkdir -p "/run/systemd/system/$APP_UNIT.d"
# cat > "/run/systemd/system/$APP_UNIT.d/50-example.conf" <<UNIT
# [Service]
# ExecStartPre=/bin/sed -n "s|^LD_PRELOAD=|LD_PRELOAD=$MOD_DIR/lib/example-first.so:|w /run/example.env" /run/cdj3k-mods/preload.env
# EnvironmentFile=-/run/example.env
# UNIT
