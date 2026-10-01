# cdj3k-emu - Network Stack

> Reference for how the emulator exposes a NIC to the guest, with a focus
> on Pro DJ Link (DJPL) reach. The modes trade ease against L2
> visibility. Companion to `docs/audio-stack.md` and `docs/alc.md`.

---

## Why this matters

Pro DJ Link is the LAN protocol Pioneer CDJs use to broadcast beat,
master / slave handoff, BPM, ABS_POS, and PLAYER_STATUS over UDP. To
sync two emulator instances - or one emulator and a real CDJ - the
guest's NIC has to sit on the **same L2 segment** as its peers:
broadcasts and arrival-time semantics don't survive NAT.

QEMU's default `-netdev user` (SLIRP) is a userspace NAT stack and
cannot carry DJPL discovery / beat broadcasts between hosts. For real LAN
visibility, macOS uses `vmnet.framework` - opened by QEMU itself, in-process -
or a kernel TAP bridge, Linux a macvtap or a tap on a bridge, and Windows a
TAP-Windows6 adapter in the Network Bridge.

---

## Modes

| Mode             | QEMU netdev                                          | L2 reach                 | Needs root | Cleanup mechanism      |
| ---------------- | ---------------------------------------------------- | ------------------------ | ---------- | ---------------------- |
| **User-mode**    | `user,id=net0,hostfwd=tcp::<2222+id>-:22`            | NAT only                 | no         | n/a                    |
| **vmnet-bridged**| `vmnet-bridged,id=net0,ifname=<iface>`               | full L2 on iface         | no         | QEMU exit              |
| **vmnet-host** (link-local) | `vmnet-host,id=net0,net-uuid=<UUID>`      | shared `bridgeN`, no NIC | no         | QEMU exit              |
| **TAP bridge**   | `tap,id=net0,fd=<N>`                                 | full L2 via TAP          | once per start | lease watcher      |
| **Linux link**   | `tap,id=net0,fd=<N>`                                 | full L2 on iface         | once per start | lease watcher      |
| **Windows link** | `tap,id=net0,ifname=<adapter>`                       | full L2 on iface         | once per start | lease watcher      |

vmnet and the TAP bridge are macOS; the Linux link is Linux; the Windows link
is Windows. Both vmnet modes are unprivileged: vmnet requires root *or* the
`com.apple.developer.networking.vmnet` entitlement, and the app carries it,
authorised by `Contents/embedded.provisionprofile`. Entitlements are
process-wide, so QEMU reaches vmnet from inside `libcdj3k-emu-qemu.dylib` -
the same arrangement HVF already uses. TAP is the one mode that still prompts:
`ifconfig bridge create` is root-only and no entitlement covers it.

**vmnet-host** ("Host-only (link-local)") puts all instances on the same virtual
`bridgeN`, isolated from physical interfaces, via a fixed network UUID (see
`vmnet.rs`, `VMNET_HOST_NETWORK_ID`; `DJPL_VMNET_UUID` overrides it, and djx-emu
must be given the same value). The UUID both joins the segment and suppresses
vmnet's `bootpd`, so guests fall through `udhcpc -T 2` to `avahi-autoipd` and
self-assign 169.254/16 - matching real Pro DJ Link gear on a router-less
network. Measured from a guest on the segment: with the UUID a DHCP DISCOVER
goes unanswered while other traffic still flows; without it `bootpd` offers a
192.168.128.x lease. Two interfaces created under one UUID by separate
processes see each other's broadcasts, which is what lets several instances
share one DJ-Link segment with no daemon between them.

`QemuConfig::build_argv` selects: a tap wins if present (an fd, or on Windows
an adapter name; `net::tap_netdev` renders it per host), otherwise a vmnet
mode, otherwise user-mode. Which of those an interface gets is the host's
`net::attach`.

The device line is identical across modes:

```
-device virtio-net-device,netdev=net0,mac=<MAC>,mrg_rxbuf=off
```

`mrg_rxbuf=off` keeps the guest's virtio_net consumer ring layout
simple - mergeable RX buffers offer no win for the DJPL packet sizes
we see and complicate the trace.

### Mode 1 - User-mode (SLIRP)

Default when neither vmnet nor TAP is selected. QEMU's built-in
userspace stack provides DHCP, DNS, and outbound NAT.

- SSH: host port `2222 + instance_id` forwards to guest `:22`.
- **Cannot** receive DJPL broadcasts from other hosts.
- **Cannot** be discovered by other DJPL endpoints.
- Fine for solo dev, building, kernel work, anything not involving
  beat-sync against a peer.

### Mode 2 - vmnet (bridged and host)

QEMU's own vmnet backends open `vmnet.framework` directly:

```
-netdev vmnet-bridged,id=net0,ifname=en0
-netdev vmnet-host,id=net0,net-uuid=7b3d5e2a-9c14-4f6b-b2e1-0a1c2d3e4f50
```

Bridged puts the guest on the host's physical LAN with a real MAC: it can be
ARP'd from other devices and exchanges DJPL broadcasts natively. Host mode
joins the isolated UUID segment described above.

`isolated` is left at its default (off) in both modes. Switching it on cuts
each guest off from the others on the same vmnet network, which is the
opposite of what DJ-Link discovery needs.

**Sharing is by UUID, not by daemon.** Each QEMU process opens its own vmnet
interface; interfaces created under one `net-uuid` land on one L2 segment.
There is no socket, no lease, no watchdog and no shared daemon to reap - the
interface is released when QEMU exits. Bridged instances need no coordination
at all: they are each on the real LAN.

`VmnetMode` (`crates/cdj3k-emu-runtime/src/net/vmnet.rs`) carries the choice
and renders the `-netdev` argument; interface names are checked
(`platform::net::is_valid_iface`) before they reach a command line.

### Mode 3 - TAP bridge

For OpenVPN-style scenarios where the L2 segment lives on a `tunN` /
`tapN` device. `vmnet.framework` cannot attach to TAP, so we drop
down to a macOS kernel bridge:

```
host_tap (e.g. tap0)  ──┐
                         ├── bridgeN ── qemu_tap (tapM) ── QEMU guest
   other ifaces ────────┘
```

Flow (`TapBridge::setup`, `net/tapbridge.rs`):

1. `cleanup_stale()` reads `<instance>/tapbridge.names` from a previous
   run; if present, `ifconfig destroy` the old bridge and tap via one
   elevated call.
2. Take the slot's lease ([Link lifetime](#link-lifetime)) and write the
   watcher script to `<instance>/tapbridge.sh`.
3. `run_elevated()` launches the script.
4. Watcher picks a free `tapN`, opens it on fd 3 to materialise the
   interface, creates a `bridgeN`, adds the host tap with STP off,
   `chmod 0666 /dev/tapN`, closes fd 3, writes
   `<bridge>:<tap>\n` to the names file.
5. Host process opens `/dev/<tap>` itself, clears `FD_CLOEXEC`, passes the
   fd to QEMU as `tap,fd=N`. The
   interface is **never DOWN** between bridge setup and the first
   packet - the fd is always held by someone.
6. Watcher's Phase 1 polls until the tap shows `RUNNING` again (with
   the new host fd), then `addm` to the bridge with STP off.
7. Phase 2: the watcher holds the link for the lease.

Teardown: `chmod 0600 /dev/<tap>`, `ifconfig <bridge> deletem <tap>`,
`ifconfig <tap> down`, and `ifconfig <bridge> destroy` once no other tap is
on the shared bridge; then remove the names file and the watcher script, and
answer the lease.

### Mode 4 - Linux link

`net/linux_net.rs` asks sysfs what the picked interface is
(`platform::net::linux_kind`) and gives the guest:

| Picked | Link | QEMU gets |
|---|---|---|
| a physical NIC | `cdj3k<slot>`, a macvtap in bridge mode on it, with the slot's MAC | `/dev/tap<ifindex>` |
| a bridge | `cdj3k<slot>`, a tap enslaved to it | `/dev/net/tun` attached to the tap |
| an existing tap | the tap itself | the tap, if this user can open it |

An elevated script (`pkexec`, `<instance>/linuxnet.sh`) deletes any
`cdj3k<slot>` left over, makes the link, hands its device node to the user and
writes the node's path to `<instance>/linuxnet.ready`. It then holds the link
for the slot's lease in the background and deletes it (`ip link del`) when
the lease goes. An existing tap is used as it is: nothing is made, elevated or
deleted. A Wi-Fi station cannot carry either shape, so it is not offered.

A macvtap cannot reach its own parent's host: rekordbox on the same machine
does not see the deck. A bridge built by hand and picked does.

### Mode 5 - Windows link

```
NIC (e.g. "Ethernet") ──┐
                         ├── Network Bridge ── cdj3k-emu-<slot> (TAP-Windows6) ── QEMU guest
   other members ───────┘
```

QEMU opens the slot's TAP-Windows6 adapter by connection name,
`cdj3k-emu-<slot>` (`winbridge::tap_name`). The Windows Network Bridge joins
it to the picked NIC; the guest keeps the MAC QEMU's `-device` gives it, and
nothing is written onto the adapter.

Flow (`WindowsBridge::setup`, `net/windows_net.rs`):

1. The app takes the slot's lease ([Link lifetime](#link-lifetime)) and
   re-runs itself through `run_elevated` as
   `--windows-net-helper <instance dir> <tap> <nic> <inf|-> <pid> <claim> <mac>`
   (`winbridge::Request`; `-` when the install has no INF).
2. The helper (`net/windows_helper.rs`) creates the adapter when there is
   none: a root-enumerated `tap0901` device through SetupAPI, the steps
   `devcon install` takes, since TAP-Windows6 9.27.0 ships no `tapctl`. The
   INF goes to the driver store (`SetupCopyOEMInfW`) and the best compatible
   driver is installed on that device alone (`DIF_INSTALLDEVICE`), so the
   other slots' taps keep theirs. It writes the connection name under the
   network class key, where QEMU looks adapters up, and renames the interface
   alias (`Rename-NetAdapter`), which the registry write alone leaves at
   "Local Area Connection" until the next boot.
   The adapter gets `AllowNonAdmin` "1", without which only administrators
   can open it, and a fixed MAC (`NetworkAddress`, `winbridge::tap_mac`: the
   guest's with the first octet 02), and is restarted when either changed.
3. It joins the tap and the NIC to the host's Network Bridge: `netsh bridge
   create <tap> <nic>` when `netsh bridge list` shows none (the bridge takes
   the tap's MAC, so the host's address on it keeps its DHCP lease from one
   start to the next), recording its GUID in
   `%ProgramData%\cdj3k-emu\bridge-owned`, then `netsh bridge add` for each
   one not yet bridged (`create` reports success with only its first adapter
   joined). Windows has one Network Bridge, so an existing one is joined,
   never replaced.
4. Membership of both is checked again, the helper starts a copy of the app
   as `--windows-net-watch` with the same request, and the verdict goes to
   `<instance>/winnet.result` (`ok`, `tap` or `bridge`, then the message;
   `winbridge::Outcome`).
5. The watcher holds the link for the lease. When the lease goes it takes
   the tap out of the bridge, or destroys the bridge when `bridge-owned` names
   it and no other `cdj3k-emu-*` tap is in it, which gives the NIC its address
   back (`winbridge::teardown`). It answers the lease, then removes the tap
   device once its media goes disconnected: TAP-Windows6 reports connected
   while QEMU holds it open.

Setup and teardown hold the `Global\cdj3k-emu-bridge` mutex. A watcher whose
claim was replaced by a later setup for the same slot leaves the link to it.
A link survives only a host reboot, which ends the watcher; the next bridged
start of the slot reuses it. The uninstaller removes adapters named
`cdj3k-emu-*` and destroys the bridge `bridge-owned` names. `netsh bridge`
needs Windows 11 22H2 with KB5030310; on older builds the bridge is made by
hand in Network Connections.

The adapter list comes from `GetIfTable2`, with addresses from
`GetAdaptersAddresses`: a bridge member has no IP binding of its own and drops
out of `GetAdaptersAddresses`, and shows the bridge's address instead
(`platform::net::windows`). `windows_kind::Adapter::bridgeable` keeps
Ethernet and Wi-Fi adapters that are up and whose name or description matches
no virtual, tunnel, tap or bridge driver. A Wi-Fi adapter is offered; an
access point accepts one MAC per station, so DJ-Link may not see the deck
through it.

The driver comes from the installer: `OemVista.inf`, `tap0901.sys` and
`tap0901.cat` under `<resources>/tap-windows6/`, added to the driver store
through `pnputil`.

---

## MAC addresses

Two MAC sources, one per mode:

| Source                       | When used                          | Form                              |
| ---------------------------- | ---------------------------------- | --------------------------------- |
| Persisted random (settings)  | runtime production launch          | `0a:xx:xx:xx:xx:xx` (LAA / unicast) |
| Deterministic from `id`      | fallback when no persisted MAC     | `0a:00:00:00:00:<id>`             |

The persisted MAC lives in `instance-N/settings.txt` under key `mac`
and is generated on first launch via `uuid::Uuid::new_v4()` with the
first byte forced to `02|LAA`:

> `crates/cdj3k-emu-storage/src/settings/identity.rs:44` - `generate_mac()`

The runtime substitutes a fallback `0a:00:00:00:00:<id&0xff>` if no
persisted MAC is set (`QemuConfig::build_argv`).

---

## Elevation flow

`elevate::run_elevated` is the single elevation primitive; the TAP bridge,
the Linux link and the Windows link are its callers. On macOS it is
AuthorizationServices:

```
AuthorizationCreate(NULL, NULL, kAuthorizationFlagDefaults, &auth)
AuthorizationCopyRights(auth, &{system.privilege.admin},
                         NULL,
                         kAuthorizationFlagInteractionAllowed
                         | kAuthorizationFlagExtendRights, NULL)
AuthorizationExecuteWithPrivileges(auth, /bin/sh,
                                   kAuthorizationFlagDefaults,
                                   ["-c", <script>, NULL], NULL)
AuthorizationFree(auth, kAuthorizationFlagDefaults)
```

`AuthorizationCopyRights` is what triggers the native admin dialog
(TouchID / Apple Watch / password). The shell that runs under
`AuthorizationExecuteWithPrivileges` has no controlling tty - fine
for our scripts, which only spawn backgrounded daemons + watchdogs.

On Linux it is `pkexec /bin/sh -c <cmd>`, answered by the desktop's polkit
agent; it needs a local seat.

On Windows it is UAC: `ShellExecuteExW` with the `runas` verb on
`cmd.exe /C <cmd>`, hidden, waited on; the exit code is the result, and a
declined prompt is `PermissionDenied`.

### Link lifetime

A link the app makes lasts as long as the slot uses it, on every host
(`net/lease.rs`). The elevated half that makes it stays running as its
watcher:

1. Before elevating, the app writes a claim, `<pid>-<ms>`, to
   `<instance>/net.claim` (`Lease::take`), and passes the claim and its pid to
   the watcher.
2. The watcher holds the link while `net.claim` still reads that claim and
   the app process is alive (`kill -0`; on Windows a wait on the process).
3. Dropping the `Lease` removes the claim: the slot switched network, stopped
   or quit. A crash ends the process instead. Either way the watcher takes
   the link down and writes the claim to `<instance>/net.released`.
4. The dropped `Lease` waits up to 15 s for that answer, so a setup that
   follows finds the host settled.

| Host | Watcher | Takes down |
|---|---|---|
| macOS (TAP bridge) | `tapbridge.sh`, backgrounded | the tap, and the shared bridge once it holds no other tap |
| Linux | the background half of `linuxnet.sh` | `cdj3k<slot>` |
| Windows | the app, `--windows-net-watch` | the tap from the bridge, the bridge it made once no slot's tap is left, then the adapter |

A host shutdown ends a watcher without its teardown. On macOS and Linux the
kernel's interfaces go with it; on Windows the adapter and the bridge stay, and
the next bridged start of the slot uses them.

vmnet needs none of this: QEMU owns the interface and the kernel releases it
when QEMU exits.

---

## Discovery & SSH

### User-mode

```
ssh -p $((2222 + INSTANCE_ID)) root@localhost
scp -P $((2222 + INSTANCE_ID)) file root@localhost:/tmp/
```

Default port for instance 0 is `2222`. The guest runs dropbear only when the
setup window's **Developer / Root SSH** option was ticked at provisioning: the
provisioning boot gets `cdj3k.ssh=1`, which the patch dispatcher reads as
`ENABLE_SSH` for `02`-`05` in `initramfs-patch/patch-rootfs.d/`.

### Bridged (vmnet, TAP, Linux or Windows link)

The guest gets a DHCP lease on the host LAN. Find it by MAC:

```
arp -an | grep <mac>          # e.g. 0a:00:00:00:00:01 in dev mode
ssh root@<discovered ip>
```

For known-MAC scripted discovery, the runtime sets `mac` deterministically
when `instance_id` is small or persists it in `settings.txt` so the
mapping is stable across launches.

---

## Pro DJ Link audio-latency compensation

Transport gives DJPL packets a path; **alignment** is what makes the
audible beat coincide between peers. Slave / master shims (clock-shift
on `OptFstUdpServer`; delay-send on `sendto`/`sendmsg`) live in the
audio stack - see:

- `docs/alc.md` - the audio-latency-compensation design end-to-end.
- `docs/audio-stack.md` - pipeline, soft-PLL, latency surface.

Two facts that matter for this doc:

- **Bridged mode is required** for cross-instance / emulator-to-real-CDJ
  sync to be audible-accurate; user-mode NAT not only blocks discovery
  but also re-orders / re-times broadcasts in ways that the master
  shim's per-packet sendto-deadline cannot compensate for.
- The compensation reads `/sys/module/virtio_snd/parameters/audio_latency_ms`,
  which is independent of transport.

---

## Security / defence-in-depth

The runtime never feeds an interface name to an elevated shell
without two layers of filtering:

- **`platform::net::is_valid_iface`**: 1 to 15 bytes of ASCII
  alphanumerics, `.`, `_` and `-`, and not `.` or `..`. Checked by
  `VmnetMode::bridged`, `TapBridge::setup` and `LinuxBridge::setup`;
  `LinuxBridge::setup` also checks the MAC (`is_valid_mac`).
- **`windows_kind::is_valid_adapter_name`** on Windows, where connection names
  hold spaces and any script's letters: 1 to 64 characters of alphanumerics,
  space, `.`, `_` and `-`, no edge spaces. `WindowsBridge::setup` checks it
  and the MAC, and the helper re-checks every part of its request
  (`Request::parse`), paths included (`is_cmd_safe_path`: no `"`, `%`, `&`,
  `|`, `<`, `>`, `^` or line breaks).
- **`elevate::sh_quote`**: wraps the argument in single quotes and escapes
  embedded `'` as `'\''`, for everything interpolated into an elevated
  script.

Per-instance state is rooted in `runtime_paths::instance_dir(id)`: the
lease's claim and answer, the TAP bridge's names file and watcher script, the
Linux link's script and ready file, and the Windows helper's verdict. The host
app process owns this directory; the elevated halves only read from it, except
for the files they report through (the names, ready, verdict and answer
files). The lease relies on that: only the app can change the claim.

---

## Files

| Path                                                       | Role                                              |
| ---------------------------------------------------------- | ------------------------------------------------- |
| `crates/cdj3k-emu-runtime/src/config.rs`                   | `-netdev` / `-device` selection                   |
| `crates/cdj3k-emu-runtime/src/net/`                        | `attach` per host, `NetAttachment`                |
| `crates/cdj3k-emu-runtime/src/net/vmnet.rs`                | `VmnetMode` -> `-netdev` argument, network UUID    |
| `crates/cdj3k-emu-runtime/src/net/lease.rs`                | the slot's lease on its link; every host          |
| `crates/cdj3k-emu-runtime/src/net/tapbridge.rs`            | bridgeN + tapM watcher, stale cleanup (macOS)     |
| `crates/cdj3k-emu-runtime/src/net/linux_net.rs`            | macvtap / tap-on-bridge / existing tap (Linux)    |
| `crates/cdj3k-emu-runtime/src/net/windows_net.rs`          | `WindowsBridge`: claim, elevation, verdict, release (Windows) |
| `crates/cdj3k-emu-runtime/src/net/windows_helper.rs`       | the elevated helper and watcher: SetupAPI tap, `netsh bridge` (Windows) |
| `crates/cdj3k-emu-runtime/src/net/winbridge.rs`            | helper request / result, `netsh bridge` parsing, teardown plan; every host |
| `crates/cdj3k-emu-runtime/src/elevate/`                    | `run_elevated` per host, `sh_quote`               |
| `crates/cdj3k-emu-platform/src/net/`                       | interfaces offered, validators, `linux_kind`, `windows_kind` |
| `crates/cdj3k-emu-storage/src/settings/`                   | persisted `mac`, `net_iface`, MAC generator       |
| `crates/cdj3k-emu-platform/src/runtime_paths/`             | socket / instance-dir layout                      |
| `qemu/build.sh`                                            | QEMU build (unrelated to runtime networking)      |
| `initramfs-patch/patch-rootfs.d/03-dropbear-enable.sh`     | enables in-guest SSH for both modes               |
