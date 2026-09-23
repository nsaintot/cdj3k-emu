# PC Link (USB-B HID/MIDI)

Emulates the deck's rear USB-B cable: carries the emulated deck's USB-HID and
USB-MIDI interfaces to macOS so DJ apps (djay, Traktor, ...) control the emulated
deck as real hardware (where supported). The **PC Link** menu toggle is the cable, on = plugged,
off = unplugged.

## Data path

QEMU exposes nothing to macOS, so each endpoint is synthesised host-side from
the guest's own USB stack, tunnelled over one virtio-serial port
(`cdj3k.usb-link`):

```
app ── /dev/hidg0 (f_hid) ─ dummy_hcd ─ /dev/hidraw0 ┐
app ── f_midi ─ dummy_hcd ─ /dev/snd/midiC2D0 (USB-Audio) ┤
                                          cdj3k-pc-link-bridge ── cdj3k.usb-link
                                                                    └─ host PcLink
                                                                         ├─ IOHIDUserDevice (HID)
                                                                         └─ CoreMIDI driver plugin (MIDI)
```

- `cdj3k-pc-link-bridge` (guest) pumps `/dev/hidraw0` and card 2 MIDI over the
  virtio-serial port. `hidraw`'s leading report-number byte is added/stripped in
  the bridge; the wire payload is the bare report (64 bytes on the CDJ-3000,
  256 in / 1024 out on the CDJ-3000X, as the gadget's descriptor says).
- `PcLink` (macOS) terminates the port: HID reports become an `IOHIDUserDevice`;
  MIDI is handed to the CoreMIDI driver plugin (`tools/midi-driver`).

## Gadget identity

The firmware's own `/home/root/scripts/usb_gadget.sh` defines the gadget, and
patch 28 keeps every identity line of it: `idVendor`, `idProduct`, `bcdDevice`
(from `fw_printenv -n release`), the strings, `serialnumber` (from
`fw_printenv -n serial_number`, which the eMMC's U-Boot env carries) and the
HID `report_length` / `report_desc`. It drops only the UAC functions and
replaces the UDC bind with one that waits for `dummy_udc`.

| | CDJ-3000 (EP122) | CDJ-3000X (EP145) |
|---|---|---|
| VendorID / ProductID | `0x2B73` / `0x002F` | `0x2B73` / `0x004E` |
| Manufacturer | `Pioneer DJ` | `AlphaTheta Corporation` |
| Product | `CDJ-3000` | `CDJ-3000X` |
| HID report (in / out) | 64 / 64 bytes | 256 / 1024 bytes |

The host holds none of this. Every connection opens with a handshake: `PcLink`
sends an empty HELLO frame (resent every 300 ms, since virtio-serial drops what
arrives while the bridge has the port closed) and waits up to 1.5 s for the
bridge's IDENTITY reply (`guest/pc_link_bridge/gadget.c`), read back from
configfs as `key=value` lines. The `IOHIDUserDevice` and the MIDI plugin's
identity are built from it (`pc_link::gadget::GadgetIdentity`), so the macOS
devices match the gadget the app talks to. A re-dial repeats the handshake and
keeps the endpoints it already registered.

## Architecture

Both interfaces share one virtio-serial port, demuxed by frame type, then
diverge: HID terminates in the app, MIDI needs a third process.

```
┌─ GUEST (QEMU) ───────────────────────┐   ┌─ cdj3k-emu (app) ──────────────────┐
│                                      │   │                                    │
│   app (EP122 / EP145)                │   │   Transport                        │
│     │                    │           │   │   reader thread ──┬── on_hid()     │
│     │ /dev/hidg0         │ ALSA seq  │   │                   └── on_midi()    │
│     ▼   (f_hid)          ▼ (f_midi)  │   │        ▲                │          │
│   dummy_hcd ─────────► dummy_hcd     │   │        │                │          │
│     │                    │           │   │   writer thread         │          │
│     ▼                    ▼           │   │        ▲                │          │
│  /dev/hidraw0     /dev/snd/midiC2D0  │   │        │                │          │
│     │                    │           │   │    ┌───┴────┐           │          │
│     └──── pc-link-bridge ┘           │   │    │  mpsc  │           │          │
│                 │                    │   │    └───┬────┘           │          │
│                 ▼                    │   │        │                │          │
│         cdj3k.usb-link ══════════════╪═══╪═► usb-link.sock         │          │
└──────────────────────────────────────┘   └─────────┬───────────────┼──────────┘
                                                     │               │
                         ┌───────────────────────────┘               │
                         │ HID                                 MIDI  │
                         ▼                                           ▼
              ┌───────────────────────┐            ┌────────────────────────────┐
              │  HidBackend           │            │ instance-<id>/             │
              │  IOHIDUserDevice      │            │   midi-driver.sock         │
              │  (needs entitlement)  │            │ app BINDS, plugin DIALS    │
              └──────────┬────────────┘            │ identity, then raw MIDI    │
                         │                         └─────────────┬──────────────┘
                         ▼                                       ▼
                    ┌──────────┐              ┌─ MIDIServer (Apple daemon) ─────┐
                    │  IOKit   │              │   CDJ3KEmuMIDI.plugin           │
                    └────┬─────┘              │     one MIDIDevice per slot     │
                         │                    │       └─ entity                 │
                         │                    │           ├─ src  (→ DAW)       │
                         │                    │           └─ dst  (← DAW)       │
                         │                    └──────────────────┬──────────────┘
                         └───────────────────┬───────────────────┘
                                             ▼
                                    Djay / Ableton / ...
                            (match by VID + PID + LocationID)
```

HID can live in the app because `IOHIDUserDevice` creates the device in the
calling process. MIDI cannot: `MIDIDeviceCreate` is refused outside a driver,
and drivers are loaded by `MIDIServer`. A virtual endpoint (`MIDISourceCreate`)
needs no driver but has no parent entity, so apps that walk
endpoint → entity → device ignore it. The second socket carries the bytes from
the app into the plugin, which runs inside MIDIServer.

### Who starts what

The plugin is a bundle: it loads into MIDIServer, so its thread
lives and dies with MIDIServer (see the
[CoreMIDI docs](https://developer.apple.com/documentation/coremidi)). The app
cannot load or unload it. So the app is the stable listener and the plugin is
the dialer:

| Event | Result |
|---|---|
| PC Link on, no DAW | app binds its socket; nothing connects, nothing published |
| a DAW opens CoreMIDI | MIDIServer starts → `Start()` → pump → dial → identity → device appears |
| second slot switches on | it binds its own socket; the pump dials it too → second device |
| a slot stops | its socket closes; the pump reads EOF → that device is removed |
| every slot stops | dials keep failing every 2 s; nothing published |
| the last DAW quits | MIDIServer exits, taking the plugin's thread with it |

## Toggle = plug / unplug

On:
1. `cfgd` runs `systemctl start cdj3k-pc-link-bridge`. The runtime resends the
   command until `cfgd` echoes it back (the cfg vport drops a lone send).
2. The bridge writes `connect` to `/tmp/usbg1` (a FIFO; the deck_shim open()
   interposer redirects the app's `/proc/udev_usbg1` probe to it). The app's
   `meow::UsbHostPcConnectDetector` reads it once and raises the SOURCE
   **CONTROL MODE (CDJ)** row.
3. PC mode (`connection_with_hostapp::PcModeSwitcher`), the gate the app
   checks before emitting/accepting HID and MIDI:
   - **EP145** enters it itself: picking the SOURCE row runs
     `usecase::pc_control::PcDeckSelector::select`, which calls
     `PcModeSwitcher::setMode(1)` and notifies every `onPcAppliModeChanged`
     listener. A loaded track defers the switch until it is unloaded.
   - **EP122** has no such path, so the bridge forces it (`pcmode.c`, keyed on
     the `APP_NAME` patch 29 sets on the unit).
4. `PcLink` dials, runs the identity handshake, registers the
   `IOHIDUserDevice`, then the plugin creates its CoreMIDI device.

Off reverses all four; the FIFO delivers `disconnect`, the device and HID
unregister. Nothing is visible to macOS while unplugged.

## Pairing (macOS side)

- **HID** uses `IOHIDUserDevice` (a private IOKit SPI).
- **MIDI** uses a CoreMIDI driver plugin (`CDJ3KEmuMIDI.plugin`) that publishes
  a device named for the emulated model, connected to `PcLink` over
  `<runtime>/midi-driver.sock`; the published device name is per-model.
- DJ apps pair the MIDI device with its HID sibling by matching
  **VendorID + ProductID + LocationID**, these must be
  identical on both the plugin's MIDI device and `PcLink`'s HID device. The
  LocationID is per instance (`19070976 + instance_id`); the emulator sends it
  to the plugin in the identity; the plugin does not track which instance it
  reached.

Ordering is set by the app's behaviour: djay opens the HID when it processes a
MIDI device that *appears* (`kMIDIMsgSetupChanged`) and gives up if the HID is
absent at that moment. The plugin therefore creates its CoreMIDI device only
once the link is up, after `PcLink` has registered the HID, and removes it on
link-close. A running DJ app pairs as soon as PC Link is switched on, with no
relaunch.

The plugin ships in the app bundle's `Resources` and installs into
`~/Library/Audio/MIDI Drivers/` on the first PC Link toggle
(`pc_link::midi_driver::ensure_driver_installed`); MIDIServer loads it from
there. (manual install with `make -C tools/midi-driver install`).

## Multi-slot

Identity per slot:

| Field | Value | From |
|---|---|---|
| Product, Manufacturer, VendorID / ProductID | the model's (see [Gadget identity](#gadget-identity)) | the gadget |
| LocationID | `19070976 + instance_id` | `cdj3k_emu_platform::identity` |
| SerialNumber | `DJMP{instance_id:06}EH` | the eMMC's U-Boot env, via the gadget |

LocationID is the discriminator. DJ apps match the product name verbatim and
rekordbox composes `<Product>,<LocationID>,<n>`, so distinct LocationIDs
separate slots with the name identical on all of them.

**HID** lives in the app, one `IOHIDUserDevice` per slot, registered when PC
Link is switched on and released when it is switched off, because
`IOHIDUserDevice` needs an entitlement MIDIServer does not have.

**MIDI** is one driver for every slot. The emulator writes an 112-byte identity on
connect and the raw MIDI stream follows it; the plugin publishes a `MIDIDevice`
carrying the identity's `USBLocationID`, which is what pairs it with that
instance's HID device.

Each instance binds `instance_dir(id)/midi-driver.sock`; the plugin scans
`instance-*` every 2 s and dials each socket it is not already on, holding one
connection, and one `MIDIDevice`, per slot. Nothing is shared between
instances, and each unlinks only its own path. A node that still answers
`connect` is never removed.

The directory holds two lifetimes. Everything else in it belongs to the QEMU
process and is rebuilt by the next one, so `cleanup_qemu_files_for_restart` wipes
it on every respawn. `midi-driver.sock` belongs to `MidiDriverLink` in the app
and lasts as long as PC Link is on, across respawns, like the `PcLink`
endpoints, so the restart wipe skips it, alongside `tapbridge.*`.

The gadget's `serialnumber` is the slot's `serial_number` U-Boot variable,
which the eMMC image is written with (`cdj3k_emu_platform::identity::device_serial`),
so the Mac-side HID device and the plugin identity carry `DJMP{instance_id:06}EH`
from the gadget.

```c
struct pc_link_identity {   /* 112 bytes, little-endian, sent once on connect */
    uint32_t magic;         /* 'CDJ1' = 0x43444a31 */
    uint16_t version;       /* 1, and CDJ3KEmuMIDIVersion in the plugin's plist */
    uint16_t instance;
    uint16_t vid, pid;
    uint32_t location;
    char     product[32];   /* NUL-padded */
    char     serial[32];
    char     manufacturer[32];
};
```

The driver stamps `USBLocationID = identity.location` on each device, its entity
and its endpoints, and sets the slot index as the destination endpoint's
refCon, `Send` is handed that refCon, so a packet finds its instance's
socket.

`ensure_driver_installed` runs on the first PC Link toggle; it should run at app
startup, gated on the bundle hash.

## Where things live

| | |
|---|---|
| Gadget descriptors | `initramfs-patch/patch-rootfs.d/28-usb-gadget.sh` |
| USB-B connect probe redirect | `guest/deck_shim/core/syscalls.c` (`/proc/udev_usbg1` → `/tmp/usbg1`) |
| Bridge unit | `initramfs-patch/patch-rootfs.d/29-pc-link-bridge.sh` |
| Guest bridge | `guest/pc_link_bridge/` |
| Host PcLink (HID/MIDI/transport) | `crates/cdj3k-emu-runtime/src/pc_link/` |
| CoreMIDI driver plugin | `tools/midi-driver/` (installs to `~/Library/Audio/MIDI Drivers/`) |
