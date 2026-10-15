# Control API

The control API drives a running slot from another program: it presses
buttons, touches the screen, turns the rotary, reads the lamps, captures the
LCDs, runs shell commands in the guest and sends events. It exists for testing
mods and the emulator.

## Developer Mode

| Turn it on | Lasts |
|---|---|
| **Emulation › Developer Mode** | Saved in the slot's settings |
| `--dev` or `CDJ3K_DEV=1` | This launch only |

While Developer Mode is on:

| What | Where |
|---|---|
| The control API | TCP port 4400 + slot, on every interface (slot 3: 4403) |
| QEMU's gdb stub | TCP port 1235 + slot, on every interface |
| `cdj3k-devd` in the guest | Runs `sh` and `watch log` commands, on the `cdj3k.dev` virtio-serial port |

The menu item starts and stops the control API at once. Turning it off
releases everything the API held. The gdb stub and devd follow at the next start of
the emulation.

devd comes from the app's `tools/devd`, and the boot adds it to the guest the
way it adds mods (see [Mods](mods.md)), so a slot needs no reinstall. A boot
outside Developer Mode carries neither devd nor its port.

## Headless slots

`--headless` (or `CDJ3K_HEADLESS=1`) runs a slot without showing its window,
for scripted tests or a second deck on the link.

```text
cdj3k-emu --instance 2 --headless --dev
```

- It boots the slot's model, or `--model`. With neither, it exits with status
  2 and leaves no slot behind.
- On macOS the process has no Dock icon and no menu bar.
- It does not check for updates.
- Stop it with `quit` on the control API, or `quit` on its window socket
  (`crates/cdj3k-emu-platform/src/window_socket.rs`).
- `shot` still captures the window as it would be drawn.

`--ephemeral` (or `CDJ3K_EPHEMERAL=1`) runs a throwaway copy of slot N as
slot N+4, so the session never changes slot N. For slot 1 that is slot 5, with
the control API on port 4405.

| Part | In the copy |
|---|---|
| Kernel, initramfs, eMMC | Hard links to slot N's files. The guest writes the eMMC through an overlay, `emmc-overlay.qcow2` in the copy's runtime folder, which lasts across restarts; `reset` starts a new one |
| `settings.txt` | Slot N's, without its MAC (the copy gets its own) and without its USB device |
| `mods/list.txt`, `mods/installed/` | Copies of slot N's |

The copy takes the lock on slot N's eMMC, so slot N and its copy never run at
once. The app deletes the copy when the window closes. A copy that a crashed
session left is deleted by the next start of the app. The E2E runner drives
these copies ([E2E scenarios](e2e.md)).

## Protocol

A client connects to the port and sends one request per line. The window
answers each request with one JSON line, in order. A connection may stay open
for many requests.

- A successful reply holds `"ok": true` and the fields the table lists.
- A failed reply is `{"ok": false, "error": "<why>"}`.
- When a connection closes, the window releases every input it held until
  release, and stops its watches. Timed holds still end at their time.
- A connection with watches also receives event lines, which hold an `event`
  field and no `ok` field. An event that follows a reply on the wire happened
  after it.

```text
$ printf 'hello\n' | nc -w 1 127.0.0.1 4403
{"app":"cdj3k-emu","booted":true,"model":"cdj3k","ok":true,"panel":true,"protocol":1,"running":true,"slot":3,"version":"0.2.0"}
```

### Requests

| Request | Does | Reply fields |
|---|---|---|
| `hello` | Describe the slot | `protocol`, `app`, `version`, `slot`, `model`, `release`, `panel`, `running`, `booted`, `ephemeral`, `accel` |
| `buttons` | List the model's buttons | `buttons` |
| `lamps` | Read every lamp the model has | `lamps` |
| `state` | `hello`, plus the lamps and the API's inputs | `lamps`, `inputs` |
| `press <button> [ms]` | Hold a button for `ms` (default 200; `0` until release) | `bit` |
| `hold <button>` | Hold a button until release | `bit` |
| `clear <bit> [ms]` | Force a frame bit low, such as one the idle frame sets (default until release) | `bit` |
| `touch <x> <y> [ms]` | Touch the main LCD at fractions of its width and height (default 200 ms; `0` until release) | |
| `release <button>\|touch\|all` | Let go; `all` releases what this connection holds until release | |
| `rotary <n>` | Turn the rotary selector by `n` detents, from -100 to 100. A positive count moves down a list | |
| `tempo <pos>` | Move the tempo fader: 0 full minus, 0.5 centre, 1 full plus | |
| `jog <degrees> [ms] [top\|side]` | Turn the platter by `degrees` (positive is clockwise) over `ms` (default 500), by its top plate (default) or its side; the reply comes when the turn ends | |
| `jog spin <rps> [top\|side]` | Let go of the platter at `rps` turns per second (-20 to 20); it coasts to rest | |
| `tilt` | Tilt the deck, or bring it back | |
| `encoder beatloop <n>` | Turn the BEAT LOOP encoder by `n` detents, from -100 to 100 | |
| `knob vinyl\|jog_adjust <pos>` | Set VINYL SPEED ADJUST or JOG ADJUST, from 0 to 1 | |
| `direction fwd\|rev\|slip_rev` | Set the direction lever | |
| `lcd [main\|jog]` | Capture a screen from the guest's framebuffer | `width`, `height`, `png` (base64) |
| `shot` | Capture the window. It fails while the window is minimized | `width`, `height`, `png` (base64) |
| `wait boot [ms]` | Wait until the slot is booted (default 10 s) | |
| `wait lamp <lamp> <state> [ms]` | Wait for a lamp to be `on`, `off` or a value (default 10 s) | |
| `sh <command>` | Run `/bin/sh -c <command>` in the guest as root, for at most 30 s | `status`, `output`, `truncated` |
| `mods [list]` | List the slot's mods | `mods`, `mods_on`, `booted` |
| `mods push <path>` | Add the mod folder or archive at `path`, or replace the mod of the same name, then restart a running emulation | `name`, `version`, `replaced`, `restarted`, `mods_on` |
| `mods enable\|disable <name>` | Turn a mod on or off for the next boot | |
| `mods remove <name>` | Take a mod off the list; an installed mod's files are deleted, a folder is left as it is | |
| `mods log <name>` | The mod's journal from the last boot | `log` |
| `media [status]` | The USB port's state | `attached` (`virtual`, `physical` or `none`), `image` |
| `media attach <path>` | Attach a USB image file, as **Mount Image…** does | `attached`, `image` |
| `media eject` | Eject the USB device, virtual or physical | `attached`, `image` |
| `setting` | Read the slot's settings | One field per setting |
| `setting <name> <value>` | Change a setting, as its menu item does | `restarts` |
| `watch <topic> [filters]` | Send events about a topic on this connection | `watch` (its number) |
| `unwatch <n>\|all` | Stop a watch of this connection | |
| `restart` | Restart the emulation, as Restart Emulation does | |
| `reset` | Restart an ephemeral slot from an empty overlay | |
| `quit` | Close the window, as Quit does | |

A wait lasts at most 600 s. A wait that runs out fails with `the wait timed
out`.

The slot counts as booted once the player app drives the panel, that is,
once the guest has sent an LED frame that is not all zero since QEMU last
started. After a `restart`, the slot is not booted until the new run's player
app is up.

A turn by the top plate touches the platter, as for a scratch. A turn by the
side does not, as for a pitch bend. `jog 0 <ms>` holds the top plate still. The platter
stops when a turn ends. A turn is refused while another one runs. While the
window is hidden or headless, a turn advances in steps of about 100 ms.

`sh` merges stdout and stderr and keeps the first 1 MiB of output. A command
that exits with a status other than 0 fails, with its `status` and `output`
in the reply. A command still running after 30 s is killed with its process
group, and the reply holds what it printed.

A request for a control the model does not have fails, such as `encoder` on
a CDJ-3000 or `direction` on a CDJ-1500X.

### Media

`media attach` and `media eject` reply once the host attached or ejected the
device, at most 30 s later. The player app sees the device a few seconds
after that; wait for its screen or its lamps before using it. The path is read
on the machine that runs the window.

### Settings

| Name | Values | Restarts |
|---|---|---|
| `audio` | `on`, `off` | Yes |
| `alc` | `on`, `off` | No |
| `haptics` | `on`, `off` | No |
| `pc_link` | `on`, `off`; fails on a host without PC Link | No |
| `service_mode` | `on`, `off` | Yes |
| `mods` | `on`, `off` | Yes, while the emulation runs |
| `network` | `nat`, `host` or the name of a host interface, such as `en0` | Yes |

A change is saved for the slot. When the reply holds `"restarts": true`, the
emulation restarts and the slot is not booted until the new run is up; send
`wait boot` before the next request.

`accel` is the name of the hardware accelerator QEMU uses (`hvf`, `kvm` or
`whpx`), or `null` under TCG.

### Mods

`mods` edits the same list as the Mods view, and a change applies at the next
boot. `mods push` restarts the emulation itself. A path is read on the machine
that runs the window, and a folder is used in place, as **Add mod…** does. To
work on a mod, push its folder once, then edit it and push again, or send
`restart`.

| Field | Holds |
|---|---|
| `mods` | One entry per mod in boot order: `name`, `version`, `enabled`, `source` (`installed` or `folder`) and `from` (the archive's origin or the folder's path) |
| `mods_on` | Whether the slot's mods boot: Enable Mods is on and the slot was installed with the mods runner |
| `booted` | The mods the running emulation booted with |

### Watches

| Topic | Filters | Events |
|---|---|---|
| `lamps` | Lamp names; none for every lamp | `lamp`: `lamp`, `value`. One per lamp when the watch starts, then one per change |
| `lcd` | `main` (default) or `jog`, then an optional region `x0,y0,x1,y1` | `lcd`: `screen`, `changed`, `bbox`. Checked every 100 ms; sent when pixels in the region changed |
| `emulation` | None | `emulation`: `model`, `running`, `booted`. One when the watch starts, then one per change |
| `log` | `kernel`, `unit <name>` or `mod <name>` (none for the whole journal), then `grep <text>` | `log`: `line`. New journal lines only; `grep` keeps the lines that contain the text, in any case. The last event holds `ended` instead of `line` when the guest command stops |

Every event holds `event` and `watch`, the number the `watch` reply gave.

```text
watch lamps PLAY CUE
watch lcd main 0,0,1,0.13
watch log mod stems grep error
watch log unit EP122.service
```

### Names

| Kind | Form | Examples |
|---|---|---|
| Button | The name `buttons` lists, in any case | `PLAY`, `hot_a`, `ROTARY_PRESS` |
| Frame bit | `<byte>:<mask>` in the model's own MISO frame | `12:0x01` |
| Lamp | The name `lamps` lists, in any case | `PLAY`, `HOT_A`, `JOG_RING` |

### Lamp values

| Lamp drive | Value |
|---|---|
| On or off | `true`, `false` |
| Step | `"off"`, `"medium"`, `"full"` |
| Level, such as the jog meter | a number |
| Colour | `[r, g, b]` |

`wait lamp` counts any value other than off, `false`, 0 or black as `on`. Any
other state waits for that exact value: `full`, `medium`, `true`, `false`, a number,
or a colour written `r,g,b`, such as `wait lamp SLOT_2 136,240,255`.

### `state.inputs`

| Field | Holds |
|---|---|
| `held` | Buttons the API holds |
| `cleared` | Frame bits the API forces low |
| `touch` | The API's touch in the model's touch units, or `null` |
| `tempo`, `rotary`, `tilted` | The panel's tempo fader, rotary counter and tilt |
| `miso`, `mosi` | The last frame sent to the guest and the last LED frame received, in hex |

## `cdj3k-emu ctl`

The app binary is also a client. Each command prints one JSON line, except
`sh`, which prints the command's output, and `watch`, which prints one line
per event. It exits with 0 on success, 1 on a failure and 2 on a usage error.

```text
cdj3k-emu ctl [--slot N] [--host HOST] <command>
```

| Command | Does |
|---|---|
| `<request>` | Send a request from the table above, such as `ctl press PLAY` |
| `lcd main\|jog <out.png>` | Save a screen |
| `shot <out.png>` | Save the window |
| `match main\|jog <ref.png> [options]` | Capture the screen until it matches `ref.png`; fail at the timeout |
| `diff <ref.png> <actual.png> [options]` | Compare two files; no slot needed |
| `changes main\|jog <x0,y0,x1,y1> [--timeout MS]` | Wait until a region of the screen changes; fail at the timeout (default 10 s) |
| `still main\|jog <x0,y0,x1,y1> [--for MS]` | Fail when a region of the screen changes within `--for` (default 10 s) |
| `sh <command>` | Run a guest command and print its output; fail when it fails |
| `watch <topic> [filters] [--for MS] [--count N]` | Print events until `--for` passes or `--count` events arrived |
| `run <file>` | Run one command per line over one connection; `#` starts a comment |
| `sleep <ms>` | Pause, for `run` files |
| `e2e [options] <file\|dir>...` | Run scenarios and write a report ([E2E scenarios](e2e.md)) |

On macOS the binary is `CDJ3K Emulator.app/Contents/MacOS/cdj3k-emu`. The
Windows build has no console, so `ctl` prints nothing there; use the protocol
directly.

### Diff options

| Option | Default | Means |
|---|---|---|
| `--threshold N` | 0 | The largest channel difference that counts as equal |
| `--max N` | 0 | The changed pixels allowed |
| `--mask x0,y0,x1,y1` | | A region to leave out, in fractions of the image; repeatable |
| `--region x0,y0,x1,y1` | | Compare only inside this region |
| `--out <diff.png>` | | Write a picture of the difference: changed pixels red, masked regions blue, the rest dimmed |
| `--timeout MS` | 10000 | How long `match` keeps trying |
| `--settle MS` | 3000 | In record mode, how long the compared part of the screen must stay unchanged before `match` saves it |

A reference such as `ref.png` also takes the regions listed in `ref.mask`,
one per line as `x0 y0 x1 y1`. Use masks for parts of a screen that always
move, such as the waveform, the playhead and the time counters.

The report fields are `width`, `height`, `compared` (pixels outside the
masks), `changed` and `bbox` (`[x, y, width, height]` around the changes, or
`null`).

### Example

```text
# browse.ctl: open the USB library and check the screen
wait boot 60000
press SOURCE
touch 0.3 0.19
sleep 1000
match main refs/browse.png --timeout 5000 --out out/browse-diff.png
sh test -f /tmp/my-mod.ready
```

```text
cdj3k-emu ctl --slot 3 run browse.ctl
```
