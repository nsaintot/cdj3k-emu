# cdj3k-emu - Network Stack

> This page describes how the emulator exposes a NIC to the guest, with a
> focus on Pro DJ Link (DJPL) reach. The modes trade ease of setup against L2
> visibility. See also `docs/audio-stack.md` and `docs/alc.md`.

---

## Why this matters

Pro DJ Link is the LAN protocol Pioneer CDJs use to broadcast beats, the
master and slave handoff, BPM, ABS_POS and PLAYER_STATUS over UDP. To sync two
emulator instances, or one emulator and a real CDJ, the guest's NIC has to sit
on the **same L2 segment** as its peers, because broadcasts and their arrival
times do not survive NAT.

QEMU's default `-netdev user` (SLIRP) is a userspace NAT stack and cannot
carry DJPL discovery or beat broadcasts between hosts. For real LAN visibility:

- macOS uses `vmnet.framework`, which QEMU opens in its own process, or a
  kernel TAP bridge.
- Linux uses a macvtap or a tap on a bridge.
- Windows uses a TAP-Windows6 adapter in the Network Bridge.

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

vmnet and the TAP bridge exist only on macOS, the Linux link only on Linux,
and the Windows link only on Windows. Both vmnet modes are unprivileged: vmnet
requires root or the `com.apple.developer.networking.vmnet` entitlement, and
the app carries that entitlement, authorised by
`Contents/embedded.provisionprofile`. Entitlements apply to the whole process,
so QEMU reaches vmnet from inside `libcdj3k-emu-qemu.dylib`, as HVF does. The
TAP bridge is the only macOS mode that asks for the admin password, because
`ifconfig bridge create` needs root and no entitlement covers it.

**vmnet-host** ("Host-only (link-local)") puts all instances on the same
virtual `bridgeN`, isolated from physical interfaces, through a fixed network
UUID (`VMNET_HOST_NETWORK_ID` in `vmnet.rs`). `DJPL_VMNET_UUID` overrides it,
and djx-emu must be given the same value.

- The UUID joins the segment and also turns off vmnet's `bootpd`. Guests fall
  through `udhcpc -T 2` to `avahi-autoipd` and assign themselves a 169.254/16
  address, as real Pro DJ Link gear does on a network without a router.
- With the UUID, a DHCP DISCOVER goes unanswered while other traffic still
  flows; without it, `bootpd` offers a 192.168.128.x lease.
- Interfaces created under one UUID by separate processes see each other's
  broadcasts, so several instances share one DJ-Link segment with no daemon
  between them.

`QemuConfig::build_argv` picks a tap when there is one (an fd, or an adapter
name on Windows; `net::tap_netdev` writes it for each host), otherwise a vmnet
mode, otherwise user mode. The host's `net::attach` decides which of these an
interface gets.

Every mode uses the same device. The bridged modes also set its MAC:

```
-device virtio-net-device,netdev=net0,mac=<MAC>,mrg_rxbuf=off
```

`mrg_rxbuf=off` keeps the guest's virtio_net receive ring simple. Mergeable RX
buffers bring no gain at DJPL packet sizes and make packet traces harder to
read.

### Mode 1 - User-mode (SLIRP)

This is the default ("Default (NAT)" in the Network menu). QEMU's built-in
userspace stack provides DHCP, DNS and outbound NAT.

- SSH: host port `2222 + instance_id` forwards to guest `:22`.
- It cannot receive DJPL broadcasts from other hosts.
- Other DJPL endpoints cannot discover it.
- It suits solo development, building and kernel work: anything that does
  not beat-sync against a peer.

### Mode 2 - vmnet (bridged and host)

QEMU's own vmnet backends open `vmnet.framework` directly:

```
-netdev vmnet-bridged,id=net0,ifname=en0
-netdev vmnet-host,id=net0,net-uuid=7b3d5e2a-9c14-4f6b-b2e1-0a1c2d3e4f50
```

Bridged mode puts the guest on the host's physical LAN with its own MAC:
other devices can reach it through ARP, and it exchanges DJPL broadcasts
directly. Host mode joins the isolated UUID segment described above.

`isolated` is left at its default (off) in both modes. Switching it on cuts
each guest off from the others on the same vmnet network, which is the
opposite of what DJ-Link discovery needs.

**Instances share a segment through its UUID.** Each QEMU process opens its
own vmnet interface, and interfaces created under one `net-uuid` land on one L2
segment. There is no socket, lease, watchdog or shared daemon: the interface is
released when QEMU exits. Bridged instances need no coordination, since each
one is on the real LAN.

`VmnetMode` (`crates/cdj3k-emu-runtime/src/net/vmnet.rs`) carries the choice
and renders the `-netdev` argument; interface names are checked
(`platform::net::is_valid_iface`) before they reach a command line.

### Mode 3 - TAP bridge

This mode is for OpenVPN-style setups where the L2 segment lives on a `tunN`
or `tapN` device. `vmnet.framework` cannot attach to a TAP device, so the app
uses a macOS kernel bridge:

```
host_tap (e.g. tap0)  ──┐
                         ├── bridgeN ── qemu_tap (tapM) ── QEMU guest
   other ifaces ────────┘
```

Flow (`TapBridge::setup`, `net/tapbridge.rs`):

1. `cleanup_stale()` reads `<instance>/tapbridge.names` from a previous run.
   If the file exists, it destroys the old bridge and tap (`ifconfig destroy`)
   in one elevated call.
2. Take the slot's lease ([Link lifetime](#link-lifetime)) and write the
   watcher script to `<instance>/tapbridge.sh`.
3. `run_elevated()` launches the script.
4. The watcher picks a free `tapN` and opens it on fd 3 to create the
   interface. It creates a `bridgeN`, adds the host tap with STP off, makes
   `/dev/tapN` writable by everyone (`chmod 0666`), closes fd 3 and writes
   `<bridge>:<tap>\n` to the names file.
5. The host process opens `/dev/<tap>` itself, clears `FD_CLOEXEC` and passes
   the fd to QEMU as `tap,fd=N`. Some process always holds the fd, so the
   interface never goes down between the bridge setup and the first packet.
6. In phase 1 the watcher polls until the tap shows `RUNNING` again with the
   new host fd, then adds it to the bridge (`addm`) with STP off.
7. In phase 2 the watcher holds the link for the lease.

Teardown runs `chmod 0600 /dev/<tap>`, `ifconfig <bridge> deletem <tap>` and
`ifconfig <tap> down`, then `ifconfig <bridge> destroy` once no other tap is on
the shared bridge. It then removes the names file and the watcher script and
answers the lease.

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
deleted. A Wi-Fi station can carry neither a macvtap nor a tap on a bridge,
so it is not offered.

A macvtap cannot reach the host of its parent interface, so rekordbox on the
same machine does not see the deck. Picking a bridge built by hand avoids
this.

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
   can open it, and a fixed MAC (`NetworkAddress`, `winbridge::tap_mac`),
   which is the guest's MAC with the first octet set to 02. The adapter is
   restarted when either value changes.
3. It joins the tap and the NIC to the host's Network Bridge: `netsh bridge
   create <tap> <nic>` when `netsh bridge list` shows none (the bridge takes
   the tap's MAC, so the host's address on it keeps its DHCP lease from one
   start to the next), recording its GUID in
   `%ProgramData%\cdj3k-emu\bridge-owned`, then `netsh bridge add` for each
   one not yet bridged (`create` reports success with only its first adapter
   joined). Windows has one Network Bridge, so an existing one is joined,
   never replaced.
4. The helper checks again that both are bridge members, starts a copy of
   the app as `--windows-net-watch` with the same request, and writes the
   verdict to `<instance>/winnet.result` (`ok`, `tap` or `bridge`, then the message;
   `winbridge::Outcome`).
5. The watcher holds the link for the lease. When the lease goes it takes
   the tap out of the bridge, or destroys the bridge when `bridge-owned` names
   it and no other `cdj3k-emu-*` tap is in it, which gives the NIC its address
   back (`winbridge::teardown`). It answers the lease, then removes the tap
   device once its media goes disconnected: TAP-Windows6 reports connected
   while QEMU holds it open.

Setup and teardown hold the `Global\cdj3k-emu-bridge` mutex. A watcher whose
claim was replaced by a later setup for the same slot leaves the link to that
setup. A link outlives its slot only across a host reboot, which ends the
watcher; the next bridged start of the slot reuses that link. The uninstaller removes adapters named
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

The bridged modes give the guest a MAC from one of two sources. User mode
sets none.

| Source                       | When used                          | Form                              |
| ---------------------------- | ---------------------------------- | --------------------------------- |
| Persisted random (settings)  | normal launch                      | random, with the locally administered bit set and the multicast bit clear |
| Deterministic from `id`      | no persisted MAC                   | `0a:00:00:00:00:<id>`             |

The persisted MAC lives in `instance-N/settings.txt` under the key `mac`.
`generate_mac()` (`crates/cdj3k-emu-storage/src/settings/identity.rs`) makes
it on first launch from `uuid::Uuid::new_v4()`.

When no MAC is persisted, `QemuConfig::build_argv` uses
`0a:00:00:00:00:<id & 0xff>`.

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

`AuthorizationCopyRights` shows the native admin dialog (Touch ID, Apple
Watch or password). The shell that runs under
`AuthorizationExecuteWithPrivileges` has no controlling tty. The scripts do
not need one: they only start background daemons and watchdogs.

On Linux it is `pkexec /bin/sh -c <cmd>`, answered by the desktop's polkit
agent; it needs a local seat.

On Windows it runs `cmd.exe /C <cmd>` hidden through `ShellExecuteExW` with
the `runas` verb, which shows the UAC prompt, and waits for it. The exit code is the
result, and a declined prompt returns `PermissionDenied`.

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
kernel's interfaces disappear with it; on Windows the adapter and the bridge
stay, and the next bridged start of the slot uses them.

vmnet needs none of this: QEMU owns the interface and the kernel releases it
when QEMU exits.

---

## Discovery and SSH

### User-mode

```
ssh -p $((2222 + INSTANCE_ID)) root@localhost
scp -P $((2222 + INSTANCE_ID)) file root@localhost:/tmp/
```

The default slot, slot 1, listens on port `2223`. The guest runs dropbear only when the
setup window's **Developer / Root SSH** option was ticked at provisioning: the
provisioning boot gets `cdj3k.ssh=1`, which the patch dispatcher reads as
`ENABLE_SSH` for `02`-`05` in `initramfs-patch/patch-rootfs.d/`.

### Bridged (vmnet, TAP, Linux or Windows link)

The guest gets a DHCP lease on the host LAN. Find it by MAC:

```
arp -an | grep <mac>          # the slot's `mac` from settings.txt
ssh root@<discovered ip>
```

A script can find a deck by the `mac` in its `settings.txt`: the runtime
persists the MAC, so it stays the same across launches.

---

## Pro DJ Link audio-latency compensation

The network carries the DJPL packets; **alignment** makes the audible beat
coincide between peers. The slave shim shifts the clock of `OptFstUdpServer`
and the master shim delays `sendto` and `sendmsg`. Both belong to the audio
stack:

- `docs/alc.md` describes audio-latency compensation from end to end.
- `docs/audio-stack.md` describes the pipeline, the soft PLL and the latency
  readout.

Two facts matter for networking:

- **Bridged mode is required** for audibly accurate sync between instances,
  or between an emulator and a real CDJ. User-mode NAT blocks discovery, and
  it also reorders and retimes broadcasts, which the master shim's per-packet
  send deadline cannot correct.
- The compensation reads `/sys/module/virtio_snd/parameters/audio_latency_ms`,
  which does not depend on the network mode.

---

## Security / defence-in-depth

The runtime filters every interface name twice before it reaches an
elevated shell:

- **`platform::net::is_valid_iface`** accepts 1 to 15 bytes of ASCII
  alphanumerics, `.`, `_` and `-`, except `.` and `..`. It is checked by
  `VmnetMode::bridged`, `TapBridge::setup` and `LinuxBridge::setup`;
  `LinuxBridge::setup` also checks the MAC (`is_valid_mac`).
- **`windows_kind::is_valid_adapter_name`** applies on Windows, where
  connection names can hold spaces and letters from any script. It accepts 1
  to 64 characters of alphanumerics, space, `.`, `_` and `-`, with no leading
  or trailing space. `WindowsBridge::setup` checks it and the MAC, and the
  helper checks every part of its request again (`Request::parse`),
  including paths (`is_cmd_safe_path` rejects `"`, `%`, `&`, `|`, `<`, `>`,
  `^` and line breaks).
- **`elevate::sh_quote`** wraps the argument in single quotes and escapes an
  embedded `'` as `'\''`. Every value written into an elevated script goes
  through it.

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
| `crates/cdj3k-emu-runtime/src/net/vmnet.rs`                | `VmnetMode` and its `-netdev` argument, network UUID |
| `crates/cdj3k-emu-runtime/src/net/lease.rs`                | the slot's lease on its link; every host          |
| `crates/cdj3k-emu-runtime/src/net/tapbridge.rs`            | the bridgeN and tapM watcher, stale cleanup (macOS) |
| `crates/cdj3k-emu-runtime/src/net/linux_net.rs`            | macvtap / tap-on-bridge / existing tap (Linux)    |
| `crates/cdj3k-emu-runtime/src/net/windows_net.rs`          | `WindowsBridge`: claim, elevation, verdict, release (Windows) |
| `crates/cdj3k-emu-runtime/src/net/windows_helper.rs`       | the elevated helper and watcher: SetupAPI tap, `netsh bridge` (Windows) |
| `crates/cdj3k-emu-runtime/src/net/winbridge.rs`            | helper request / result, `netsh bridge` parsing, teardown plan; every host |
| `crates/cdj3k-emu-runtime/src/elevate/`                    | `run_elevated` per host, `sh_quote`               |
| `crates/cdj3k-emu-platform/src/net/`                       | interfaces offered, validators, `linux_kind`, `windows_kind` |
| `crates/cdj3k-emu-storage/src/settings/`                   | persisted `mac`, `net_iface`, MAC generator       |
| `crates/cdj3k-emu-platform/src/runtime_paths/`             | socket / instance-dir layout                      |
| `qemu/build.sh`                                            | QEMU build (unrelated to runtime networking)      |
| `initramfs-patch/patch-rootfs.d/03-dropbear-enable.sh`     | enables SSH in the guest when the slot was provisioned with Root SSH, in every network mode |
