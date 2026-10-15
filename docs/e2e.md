# E2E scenarios

`cdj3k-emu ctl e2e` runs scenario files against one or more slots through the
[control API](control-api.md) and writes a report. The scenarios live in
`tests/e2e/scenarios/`.

```text
cd tests/e2e/scenarios
cdj3k-emu ctl e2e --slot 1 --slot 2 --launch .
```

## Requirements

- Slots with firmware installed.
- `tests/e2e/fixtures/e2e-usb.img`, built by `tests/e2e/fixtures/make-usb.sh`.
- Two slots for the scenarios with `#@ slots 2`.

The runner drives the ephemeral session of each slot (`--ephemeral`, slot N+4;
see [Control API](control-api.md)), in Developer Mode.

## Options

| Option | Default | Effect |
|---|---|---|
| `--slot N` | `1` | A slot to run on, through its ephemeral session on slot N+4; repeat for multi-slot scenarios. A scenario with `#@ slots 2` uses the first two |
| `--launch` | off | Start the ephemeral session of each slot that has none open, headless and in Developer Mode, and close it at the end |
| `--record` | off | A `match` whose reference does not exist saves the screen as that reference |
| `--report DIR` | `e2e-report` | Where the report goes |
| `--boot-wait MS` | 600000 | How long a slot may take to boot after its reset |
| `--host HOST` | `127.0.0.1` | Given before `e2e`: the machine that runs the slots |

The exit status is 0 when no scenario failed, 1 otherwise, and 2 on a usage
error.

## What a run does

For each scenario, in file-name order:

1. Sets the scenario's settings on each of its slots.
2. Sends `reset`, which puts the eMMC back in its installed state, and waits
   for the boot.
3. Attaches a copy of the scenario's USB image, or ejects the slot's USB
   device.
4. Runs the steps. The first failing step ends the scenario.
5. Releases all inputs and watches, and ejects the copy.

At the end, the runner releases what it holds, and the sessions that
`--launch` started close and are deleted.

## Scenario files

A scenario is a `.ctl` file of `ctl run` commands with a header of `#@`
directives. `ctl run` treats `#@` lines as comments.

```text
# A loaded track starts playing, and PLAY pauses it.
#@ media ../fixtures/e2e-usb.img
#@ setting audio off
#@ setting network nat
#@ include lib/open-usb.inc
#@ include lib/load-a440.inc
changes main 0.34,0.69,0.57,0.8 --timeout 3000
press PLAY
```

| Directive | Effect |
|---|---|
| `#@ name <text>` | The scenario's name in the report; the file name by default |
| `#@ models <slug>...` | Run only on these models, such as `cdj3k cdj3kx`; skipped on others |
| `#@ slots <n>` | Drive `n` slots, 1 to 4; skipped when the run has fewer |
| `#@ setting <name> <value>` | Set a setting before the reset, as the `setting` request does; repeatable |
| `#@ media <path>` | Give each slot a copy of this USB image after the reset |
| `#@ reset off` | Keep the slots as the previous scenario left them |
| `#@ needs hw-accel` | Run only when the slots' QEMU has a hardware accelerator (HVF, KVM or WHPX); skipped under TCG |
| `#@ include <path>` | Insert the steps of another file here. An included file holds steps only; its `.inc` extension keeps it out of the run |

Paths are relative to the scenario's folder. Steps run in that folder.

The scenarios set `audio off`, `network nat` and `mods off`, except where
they test audio, the network or mods.

### Steps

Every `ctl` command works as a step: requests, `match`, `changes`, `still`,
`sh`, `watch`, `sleep`. One more exists in scenarios:

| Step | Effect |
|---|---|
| `slot <k>` | Send the next steps to the scenario's slot `k` (1 is the first) |

Words in a step take these values:

| Word | Value |
|---|---|
| `{model}` | The current slot's model slug, such as `cdj3k` |
| `{release}` | The current slot's firmware release, such as `3.22` |
| `{slot}` | The current slot's number, as given to `--slot` |
| `{media}` | The current slot's copy of the scenario's USB image |

## References

A `match` compares the screen with a reference PNG under
`refs/<model>-<release>/`. Compare only the part of the screen that the step
is about, with `--region`, and mask clocks, waveforms and player numbers in
the reference's `.mask` file.

Record mode (`--record`) saves a missing reference once the compared region has
stayed unchanged for `--settle` (3 s by default).

- Check each recorded file.
- Record a new path one reference at a time.
- Give a `match` that waits for a notice to go, such as "rekordbox Database
  not found!", a longer `--settle`.

## Report

The report folder holds:

| File | Holds |
|---|---|
| `report.json` | Each scenario's status, time, failing line and error; the slots, with model and launch state; the totals |
| `junit.xml` | One test case per scenario |
| `slot-N.log` | The output of a slot that `--launch` started |
| `NN-<name>/steps.log` | Each step and its reply |
| `NN-<name>/slotK-main.png`, `slotK-jog.png` | The screens when the scenario failed |
| `NN-<name>/slotK-journal.txt` | The guest's journal when the scenario failed |
| `NN-<name>/<ref>-actual-N.png`, `<ref>-diff-N.png` | The capture of a failed `match`, and the difference |

## Firmware

`tests/e2e/fetch-firmware.sh <model> [release]` downloads a firmware from
AlphaTheta's list, checks it against `tests/e2e/firmware.sha256` and prints
the path of its `.UPD`. `--pin` adds the hash of a release that has none.
