# Mods

The emulator supports loading mods into a slot.
A mod is a compatible file/folder with a `mod.toml` file that describes it.
A mod can load shared libraries (`.so`) into the deck's player app,  
set environment variables for the player app, and run a `loader.sh` script as root just before the player app starts,  
for example to start a background service or put files in place.

Each slot has its own mods. To see them, open **Emulation › Manage Emulation**,
then **Manage mods**. Each time the slot boots, its enabled mods are applied in
list order. The deck's file system is rebuilt at every boot, so nothing a mod
changes carries over to the next boot. A slot installed by an older version of
the app runs no mods until it is reinstalled.

**Emulation › Enable Mods** turns all of a slot's mods on or off, and restarts
the emulation if it is running.

## `mod.toml`

```toml
[mod]
name = "cdj3k-mods"                    # required
version = "0.1.1"                      # required
author = "author"                      # optional
description = "Runs Doom"              # optional
url = "https://github.com/…"           # optional
preload = ["ep122_shim.so"]            # optional
env = { EP122_MOD_LOGLEVEL = "info" }  # optional

[compat]                               # optional
cdj3k  = ">=3.13, <=3.22"
cdj3kx = "*"
```


| Key           | Meaning                                                                                                                                     | Rules                                                                                                                                                                                |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `name`        | Name of the mod.                                                                                                                            | Required. Lowercase letters, digits, `.`, `_` and `-`, starting with a letter or digit; at most 48 characters; not ending in `.`; not a Windows device name (`con`, `nul`, `com1`…). |
| `version`     | Version of the mod.                                                                                                                         | Required. One word.                                                                                                                                                                  |
| `author`      | Author of the mod.                                                                                                                          | Optional.                                                                                                                                                                            |
| `description` | A short description of what the mod do.                                                                                                     | Optional.                                                                                                                                                                            |
| `url`         | Web page of the mod's project. The log page links to it.                                                                                  | Optional. An `http://` or `https://` address; anything else is shown as text.                                                                                                       |
| `preload`     | Libraries (`.so`) loaded into the player app, after the emulator's `deck_shim.so` and after the libraries of the mods above it in the list. | Optional. Paths relative to the mod's folder, without `..`, `:`, spaces, quotes or backslashes; each file must exist.                                                               |
| `env`         | Environment variables for the player app.                                                                                                   | Optional. Values may not contain `"`, `\` or control characters, nor start or end with a space. `LD_PRELOAD` is refused: use `preload` instead.                    |
| `[compat]`    | Compatibility table of what the mod does support (model/firmware version)                                                                   | Optional. See Firmware ranges below.                                                                                                                                                 |




### Firmware ranges

A range is either `*`, which matches any release, or a comma-separated list of
conditions, each an operator (`>=`, `<=`, `>`, `<` or `=`) followed by a
release, such as `">=3.13, <=3.22"`. A release without an operator, such as
`"3.20"`, matches only 3.20. Releases are compared number by number, so 3.9 is
older than 3.10.

- Without a `[compat]` table, or with an empty one, the mod runs on every
firmware.
- Otherwise, the mod is incompatible with any model the table does not list.
- If the slot's release is unknown, every range matches.

An incompatible mod stays in the list with its on/off state, and is skipped at
boot until the slot's firmware matches.

## Adding a mod

A slot holds at most 10 mods.

- **Archive**: a compatible `.tgz`, `.tar.gz` or `.tar` file holding the mod's folder, or  
the mod's files. Add it with **Add mod…**, or download it with **Add from URL…**.  
The archive may contain only files and folders. It is unpacked into `<app data>/instance-N/mods/installed/<name>/`.
- **Folder**: use **Add mod…** on the folder (or on its `mod.toml`), or drop the
folder on the window. The app reads the folder in place at every boot, so you
can edit it, restart the emulation and see the change. **Eject** removes it
from the list and leaves the folder untouched.
- **Command line**: `--mod <folder>` (can be repeated) adds a folder for this
launch only, after the slot's mods. A folder whose mod has the same name as one
of the slot's mods boots in its place. `--no-mods` starts without the slot's
mods.

Adding a mod with the same `name` as one already in the slot replaces it. The
new mod takes the old one's place in the list and keeps its on/off state; the
old mod's files are deleted.

## `loader.sh`

Optional. The guest runs it with `/bin/sh` from the mod's folder, as the
systemd unit `cdj3k-mod-<name>.service`. It is stopped after 60 s, along with
every process it started. To keep something running, start it as its own unit
with `systemctl --no-block start`.

`loader.sh` gets these environment variables:


| Variable      | Value                                                                              |
| ------------- | ---------------------------------------------------------------------------------- |
| `MOD_DIR`     | The mod's folder in the guest, `/opt/cdj3k-mods/NN-<name>`                         |
| `MOD_PRELOAD` | The file `mod-preload` writes to                                                   |
| `MOD_BIN`     | The folder holding the helpers below, first on `PATH`                              |
| `DECK_MODEL`  | `cdj3k`, `cdj3kx` or `cdj1500x`                                                    |
| `APP_UNIT`    | The player app's systemd unit: `EP122.service`, `EP145.service` or `EP166.service` |
| `FW_VERSION`  | The slot's firmware release, e.g. `3.22`; empty when unknown                       |

It can call these helpers:

| Helper                  | Effect                                                                                                      |
| ----------------------- | ----------------------------------------------------------------------------------------------------------- |
| `mod-preload <lib.so>…` | Loads more libraries into the player app, like `preload`. A relative path is taken from the current folder. |
| `mod-env KEY=VALUE…`    | Sets more variables for the player app, like `env`, with the same rules.                                    |

## The emulator's shim

The player app's screens and controls go through `/home/root/deck_shim.so`. The
app loads it with `LD_PRELOAD`, set by the `EnvironmentFile=` lines
`/etc/cdj3k/preload.env` and `/run/cdj3k-mods/preload.env` in a drop-in of its
systemd unit. A mod that takes it out of the app leaves the deck without
screens or controls until the mod is disabled.

## Status and logs

After each boot, the list shows what happened to each mod:

| Status       | Meaning                                                                                                  |
| ------------ | -------------------------------------------------------------------------------------------------------- |
| LOADED       | Its `loader.sh` exited with code 0, or it has no `loader.sh` and its `mod.toml` settings were applied    |
| FAILED       | Its `loader.sh` exited with an error or was stopped after 60 s. Its libraries and variables still apply. |
| INCOMPATIBLE | Its `[compat]` table does not include the slot's model and firmware; it did not run                      |
| INVALID      | Its `mod.toml` is missing or cannot be read; it did not run                                              |
| UNKNOWN      | The last boot reported it in a format this version of the app cannot read                                |
| NOT RUN      | It was not part of the last boot                                                                         |


A mod's log shows the output of its `loader.sh` for that boot, followed by the
guest's messages about the mod. Messages from the libraries it loads go to the
player app's own journal (`journalctl -u $APP_UNIT`).

## Examples and template

[`mods/example`](mods/example) is a mod to copy: it sets a variable, runs a
`loader.sh`, and can load a library once you build one and list it in
`preload`. Copy the folder, change `name`, and add it with **Add mod…**.

Its `loader.sh` has commented-out examples:

- a library chosen by model and firmware
- a variable set from `loader.sh`
- the libraries already added by the mods above
- a process that keeps running, as its own unit
- a library loaded into another process
- a library loaded before `deck_shim.so`

## Supported mods

- [cdj3k-mods](https://github.com/nsaintot/cdj3k-mods) (`cdj3k-mods-vx.x.x-modloader.tgz`)
- _add your mod project here_