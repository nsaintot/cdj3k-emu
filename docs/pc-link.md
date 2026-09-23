# PC Link (USB-B HID/MIDI)

Emulates the deck's rear USB-B cable: carries the CDJ-3000's USB-HID and
USB-MIDI interfaces to macOS so DJ apps (djay, Traktor, ...) control the emulated
deck as real hardware (where supported). The **PC Link** menu toggle is the cable, on = plugged,
off = unplugged.

## Data path

QEMU exposes nothing to macOS, so each endpoint is synthesised host-side from
the guest's own USB stack, tunnelled over one virtio-serial port
(`cdj3k.usb-link`):

```
EP122 ── /dev/hidg0 (f_hid) ─ dummy_hcd ─ /dev/hidraw0 ┐
EP122 ── f_midi ─ dummy_hcd ─ /dev/snd/midiC2D0 (USB-Audio) ┤
                                          cdj3k-pc-link-bridge ── cdj3k.usb-link
                                                                    └─ host PcLink
                                                                         ├─ IOHIDUserDevice (HID)
                                                                         └─ CoreMIDI driver plugin (MIDI)
```

- `cdj3k-pc-link-bridge` (guest) pumps `/dev/hidraw0` and card 2 MIDI over the
  virtio-serial port. `hidraw`'s leading report-number byte is added/stripped in
  the bridge; the wire payload is the bare 64-byte report.
- `PcLink` (macOS) terminates the port: HID reports become an `IOHIDUserDevice`;
  MIDI is handed to the CoreMIDI driver plugin (`tools/midi-driver`).

## Architecture

Both interfaces share one virtio-serial port, demuxed by frame type, then
diverge: HID terminates in the app, MIDI needs a third process.

```
┌─ GUEST (QEMU) ───────────────────────┐   ┌─ cdj3k-emu (app) ──────────────────┐
│                                      │   │                                    │
│   EP122                              │   │   Transport                        │
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
2. The bridge writes `connect` to `/tmp/usbg1` (a FIFO; the ep122_shim open()
   interposer redirects EP122's `/proc/udev_usbg1` probe to it). EP122's
   detector reads it once and raises the SOURCE **CONTROL MODE (CDJ)** row.
3. The bridge forces `connection_with_hostapp::PcModeSwitcher` on, the
   gate EP122 checks before emitting/accepting HID and MIDI.
4. `PcLink` registers the `IOHIDUserDevice`, then the plugin creates its
   CoreMIDI device.

Off reverses all four; the FIFO delivers `disconnect`, the device and HID
unregister. Nothing is visible to macOS while unplugged.

## Pairing (macOS side)

- **HID** uses `IOHIDUserDevice` (a private IOKit SPI).
- **MIDI** uses a CoreMIDI driver plugin (`CDJ3KEmuMIDI.plugin`) that publishes
  a device named for the emulated model, connected to `PcLink` over
  `<runtime>/midi-driver.sock`; the published device name is per-model.
- DJ apps pair the MIDI device with its HID sibling by matching
  **VendorID `0x2B73` + ProductID `0x2F` + LocationID**, these must be
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

Identity per slot, from `cdj3k_emu_platform::identity`:

| Field | Value |
|---|---|
| Product | `CDJ-3000` |
| VendorID / ProductID | `0x2B73` / `0x2F` |
| LocationID | `19070976 + instance_id` |
| SerialNumber | `DJMP{instance_id:06}EH` |

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

Patch 28 writes a fixed `serialnumber` on the guest gadget; it has no access to
`instance_id`. The Mac-side HID device and the identity carry
`DJMP{instance_id:06}EH`.

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
| USB-B connect probe redirect | `guest/ep122_shim/syscalls.c` (`/proc/udev_usbg1` → `/tmp/usbg1`) |
| Bridge unit | `initramfs-patch/patch-rootfs.d/29-pc-link-bridge.sh` |
| Guest bridge | `guest/pc_link_bridge/` |
| Host PcLink (HID/MIDI/transport) | `crates/cdj3k-emu-runtime/src/pc_link/` |
| CoreMIDI driver plugin | `tools/midi-driver/` (installs to `~/Library/Audio/MIDI Drivers/`) |
