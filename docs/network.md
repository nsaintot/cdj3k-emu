# cdj3k-emu - Network Stack

> Reference for how the emulator exposes a NIC to the guest, with a focus
> on Pro DJ Link (DJPL) reach. Three modes, trading ease against L2
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
or a kernel TAP bridge, and Linux a macvtap or a tap on a bridge.

---

## Modes

| Mode             | QEMU netdev                                          | L2 reach                 | Needs root | Cleanup mechanism      |
| ---------------- | ---------------------------------------------------- | ------------------------ | ---------- | ---------------------- |
| **User-mode**    | `user,id=net0,hostfwd=tcp::<2222+id>-:22`            | NAT only                 | no         | n/a                    |
| **vmnet-bridged**| `vmnet-bridged,id=net0,ifname=<iface>`               | full L2 on iface         | no         | QEMU exit              |
| **vmnet-host** (link-local) | `vmnet-host,id=net0,net-uuid=<UUID>`      | shared `bridgeN`, no NIC | no         | QEMU exit              |
| **TAP bridge**   | `tap,id=net0,fd=<N>`                                 | full L2 via TAP          | yes        | heartbeat-file watcher |
| **Linux link**   | `tap,id=net0,fd=<N>`                                 | full L2 on iface         | once per host boot | none: the link persists |

vmnet and the TAP bridge are macOS; the Linux link is Linux. Both vmnet modes
are unprivileged: vmnet requires root *or* the
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

`QemuConfig::build_argv` selects: a tap fd wins if present, otherwise a vmnet
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
2. Write watcher script to `<instance>/tapbridge.sh`, touch
   `<instance>/tapbridge.alive` (heartbeat).
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
7. Phase 2: heartbeat loop. `Drop` removes the heartbeat;
   `kill -0 <app_pid>` also gates the loop for unclean exits.

Teardown: `chmod 0600 /dev/<tap>`, `ifconfig <tap> down`,
`ifconfig <bridge> destroy`, remove the names and heartbeat files,
remove the watcher script itself.

### Mode 4 - Linux link

`net/linux_net.rs` asks sysfs what the picked interface is
(`platform::net::linux_kind`) and gives the guest:

| Picked | Link | QEMU gets |
|---|---|---|
| a physical NIC | `cdj3k<slot>`, a macvtap in bridge mode on it, with the slot's MAC | `/dev/tap<ifindex>` |
| a bridge | `cdj3k<slot>`, a tap enslaved to it | `/dev/net/tun` attached to the tap |
| an existing tap | the tap itself | the tap, if this user can open it |

The link is made once by an elevated script (`pkexec`, `linuxnet.sh`), owned
by the user, and reused on the next launch when sysfs shows it up, on the same
parent and with the slot's MAC. It goes when the host reboots or a later setup
replaces it. A Wi-Fi station cannot carry either shape, so it is not offered.

A macvtap cannot reach its own parent's host: rekordbox on the same machine
does not see the deck. A bridge built by hand and picked does.

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

`elevate::run_elevated` is the single elevation primitive; the TAP bridge and
the Linux link are its callers. On macOS it is AuthorizationServices:

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

### Why a watcher and not direct lifetime ownership

A user-level process can `kill()` only processes it owns. The bridge and
TAP are created by a root shell after elevation, so if the host app crashes
the kernel cannot send them a teardown signal. Hence the watcher: it polls
the cdj3k-emu PID and a heartbeat file (`build_watcher_script` in
`tapbridge.rs`) and tears the interfaces down when either goes away.

vmnet needs none of this - QEMU owns the interface and the kernel releases
it when QEMU exits.

The signal is race-free: the host app holds an open fd to the
heartbeat / socket inode for its lifetime, and `kill -0 <pid>` is
atomic against process exit.

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

### Bridged (vmnet, TAP or Linux link)

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
- **`elevate::sh_quote`**: wraps the argument in single quotes and escapes
  embedded `'` as `'\''`, for everything interpolated into an elevated
  script.

Per-instance state is rooted in `runtime_paths::instance_dir(id)`: the
TAP bridge's heartbeat, names file and watcher script, and the Linux link's
script and ready file. The host app process owns this directory; root scripts
only read from it, and the TAP bridge's unlink-to-shutdown signal relies on
that.

---

## Files

| Path                                                       | Role                                              |
| ---------------------------------------------------------- | ------------------------------------------------- |
| `crates/cdj3k-emu-runtime/src/config.rs`                   | `-netdev` / `-device` selection                   |
| `crates/cdj3k-emu-runtime/src/net/`                        | `attach` per host, `NetAttachment`                |
| `crates/cdj3k-emu-runtime/src/net/vmnet.rs`                | `VmnetMode` -> `-netdev` argument, network UUID    |
| `crates/cdj3k-emu-runtime/src/net/tapbridge.rs`            | bridgeN + tapM watcher, stale cleanup (macOS)     |
| `crates/cdj3k-emu-runtime/src/net/linux_net.rs`            | macvtap / tap-on-bridge / existing tap (Linux)    |
| `crates/cdj3k-emu-runtime/src/elevate/`                    | `run_elevated` per host, `sh_quote`               |
| `crates/cdj3k-emu-platform/src/net/`                       | interfaces offered, validators, `linux_kind`      |
| `crates/cdj3k-emu-storage/src/settings/`                   | persisted `mac`, `net_iface`, MAC generator       |
| `crates/cdj3k-emu-platform/src/runtime_paths/`             | socket / instance-dir layout                      |
| `qemu/build.sh`                                            | QEMU build (unrelated to runtime networking)      |
| `initramfs-patch/patch-rootfs.d/03-dropbear-enable.sh`     | enables in-guest SSH for both modes               |
