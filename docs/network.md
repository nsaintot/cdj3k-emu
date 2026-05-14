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
cannot carry DJPL discovery / beat broadcasts between hosts. To get
real LAN visibility on macOS we use either `vmnet.framework` (via
`socket_vmnet`) or a kernel TAP bridge.

---

## Three modes

| Mode             | QEMU netdev                                                                                       | L2 reach           | Needs root | Cleanup mechanism      |
| ---------------- | ------------------------------------------------------------------------------------------------- | ------------------ | ---------- | ---------------------- |
| **User-mode**    | `user,id=net0,hostfwd=tcp::<2222+id>-:22`                                                         | NAT only           | no         | n/a                    |
| **vmnet-bridged**| `stream,id=net0,server=off,addr.type=unix,addr.path=<sock>`                                       | full L2 on iface   | yes        | watchdog unlinks sock  |
| **TAP bridge**   | `tap,id=net0,fd=<N>`                                                                              | full L2 via TAP    | yes        | heartbeat-file watcher |

Selection logic lives in `crates/cdj3k-emu-runtime/src/config.rs:274-298`:
TAP fd wins if present, otherwise the vmnet socket path wins, otherwise
user-mode falls through.

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

- SSH: host port `2222 + instance_id` forwards to guest `:22`. The
  runtime forwards SSH only; the dev `boot.sh` additionally forwards
  UDP `8801` (cfgd/subucom debug listener).
- **Cannot** receive DJPL broadcasts from other hosts.
- **Cannot** be discovered by other DJPL endpoints.
- Fine for solo dev, building, kernel work, anything not involving
  beat-sync against a peer.

### Mode 2 - vmnet-bridged (`socket_vmnet`)

Bundled / brew-installed `socket_vmnet` daemon (`lima-vm/socket_vmnet`)
binds `vmnet.framework` in bridged mode against a chosen host
interface, exposes it as a Unix socket, and bridges it onto the host's
physical LAN. QEMU connects with `-netdev stream` over that socket.
The guest receives a real MAC on the host LAN, can be ARP'd from
other devices, and exchanges DJPL broadcasts natively.

Binary search order (`vmnet.rs:16-20`, `find_binary()`):

```
<app bundle>/socket_vmnet                  (preferred - bundled)
/opt/homebrew/bin/socket_vmnet              (Apple Silicon brew)
/usr/local/bin/socket_vmnet                 (Intel brew)
/opt/homebrew/opt/socket_vmnet/bin/socket_vmnet
```

**Daemon is shared across instances.** The socket path is keyed by the
host interface name (`runtime_paths::vmnet_sock(iface)`). On `start_bridged`
(`vmnet.rs:55-80`):

1. If `connect()` to the socket succeeds → attach, `owns_daemon=false`.
2. Stale socket file with no listener → unlink, then proceed.
3. Otherwise → `launch_elevated()`, `owns_daemon=true`.

`Drop` unlinks the socket only if `owns_daemon` (`vmnet.rs:94-100`).
That's how three concurrent instances share one daemon and one
elevation dialog: the second and third just attach.

**Root-side watchdog** (`vmnet.rs:194-211`): the elevated shell spawns
a subshell that polls every second:

```sh
while kill -0 <cdj3k-emu pid> && [ -S <sock> ]; do sleep 1; done
kill <SV_PID>; sleep 0.3; kill -9 <SV_PID>; rm -f <sock>
```

Either the host app vanishes (crash / SIGKILL / clean exit) or the
socket gets unlinked (the user-side `stop()` signal) and the daemon
is reaped. The user-level app cannot kill a root process directly,
but it *can* unlink a socket in a user-owned directory - that's the
shutdown channel.

### Mode 3 - TAP bridge

For OpenVPN-style scenarios where the L2 segment lives on a `tunN` /
`tapN` device. `vmnet.framework` cannot attach to TAP, so we drop
down to a macOS kernel bridge:

```
host_tap (e.g. tap0)  ──┐
                         ├── bridgeN ── qemu_tap (tapM) ── QEMU guest
   other ifaces ────────┘
```

Flow (`tapbridge.rs:57-118`):

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
5. Host process opens `/dev/<tap>` itself, clears `FD_CLOEXEC`
   (`tapbridge.rs:110`), passes the fd to QEMU as `tap,fd=N`. The
   interface is **never DOWN** between bridge setup and the first
   packet - the fd is always held by someone.
6. Watcher's Phase 1 polls until the tap shows `RUNNING` again (with
   the new host fd), then `addm` to the bridge with STP off.
7. Phase 2: heartbeat loop. `Drop` removes the heartbeat;
   `kill -0 <app_pid>` also gates the loop for unclean exits.

Teardown: `chmod 0600 /dev/<tap>`, `ifconfig <tap> down`,
`ifconfig <bridge> destroy`, remove the names and heartbeat files,
remove the watcher script itself.

---

## MAC addresses

Two MAC sources, one per mode:

| Source                       | When used                          | Form                              |
| ---------------------------- | ---------------------------------- | --------------------------------- |
| Persisted random (settings)  | runtime production launch          | `0a:xx:xx:xx:xx:xx` (LAA / unicast) |
| Deterministic from `id`      | `boot.sh` dev launches in vmnet mode | `0a:00:00:00:00:<id>`             |

The persisted MAC lives in `instance-N/settings.txt` under key `mac`
and is generated on first launch via `uuid::Uuid::new_v4()` with the
first byte forced to `02|LAA`:

> `crates/cdj3k-emu-storage/src/settings.rs:237-248` - `generate_mac()`

The runtime substitutes a fallback `0a:00:00:00:00:<id&0xff>` if no
persisted MAC is set (`config.rs:270-273`). The dev `boot.sh` always
uses the deterministic form so you can `arp -an | grep
0a:00:00:00:00:01` to find an instance quickly.

---

## Elevation flow

`run_elevated()` in `vmnet.rs:244-308` is the single elevation
primitive (both vmnet and tapbridge call it):

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

### Why a watcher and not direct lifetime ownership

A user-level process can `kill()` only processes it owns. socket_vmnet
runs as root after elevation; if the host app crashes, the kernel
cannot send it a teardown signal. The pattern across both modes:

- **vmnet**: watchdog polls cdj3k-emu PID + socket file presence
  (`launch_elevated` in `vmnet.rs:178-211`).
- **tapbridge**: watcher polls cdj3k-emu PID + heartbeat file
  (`build_watcher_script` in `tapbridge.rs:136-215`).

Both signals are race-free: the host app holds an open fd to the
heartbeat / socket inode for its lifetime, and `kill -0 <pid>` is
atomic against process exit.

---

## Discovery & SSH

### User-mode

```
ssh -p $((2222 + INSTANCE_ID)) root@localhost
scp -P $((2222 + INSTANCE_ID)) file root@localhost:/tmp/
```

Default port for instance 0 is `2222`. Inside the guest, dropbear is
enabled at boot via `initramfs-patch/patch-rootfs.d/03-dropbear-enable.sh`.

### Bridged (vmnet or TAP)

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

- **`is_valid_iface()`** in `vmnet.rs:44-48`:

  ```
  non-empty && len <= 16 && ASCII alnum / '_' / '-' only
  ```

  Matches the BSD ifname grammar. Used by both `SocketVmnet::start_bridged`
  (`vmnet.rs:56-61`) and `TapBridge::setup` (`tapbridge.rs:58-63`).

- **`sh_quote()`** in `vmnet.rs:311-313`: wraps the argument in single
  quotes and escapes embedded `'` as `'\''`. Sufficient on its own
  for `/bin/sh -c` and shell scripts - `is_valid_iface` is the
  belt-and-braces check so that even an `sh_quote` regression cannot
  hand the elevated shell a metacharacter-bearing iface name.

Per-instance state is rooted in `runtime_paths::instance_dir(id)` -
heartbeat, names file, watcher script, vmnet socket path. The host
app process owns this directory; root scripts only read from it, and
the unlink-to-shutdown signal exploits exactly that asymmetry.

---

## Files

| Path                                                       | Role                                              |
| ---------------------------------------------------------- | ------------------------------------------------- |
| `crates/cdj3k-emu-runtime/src/config.rs`                   | `-netdev` / `-device` selection (`L274-298`)      |
| `crates/cdj3k-emu-runtime/src/vmnet.rs`                    | socket_vmnet elevation + root watchdog            |
| `crates/cdj3k-emu-runtime/src/tapbridge.rs`                | bridgeN + tapM watcher, stale cleanup             |
| `crates/cdj3k-emu-storage/src/settings.rs`                 | persisted `mac`, `net_iface`, MAC generator       |
| `crates/cdj3k-emu-platform/src/runtime_paths.rs`           | socket / instance-dir layout                      |
| `boot.sh`                                                  | dev launcher; deterministic MAC, UDP 8801 forward |
| `qemu/build.sh`                                            | QEMU build (unrelated to runtime networking)      |
| `initramfs-patch/patch-rootfs.d/03-dropbear-enable.sh`     | enables in-guest SSH for both modes               |
