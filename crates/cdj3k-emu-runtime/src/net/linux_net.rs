//! Bridged networking on Linux: putting the guest on the real LAN.
//!
//! DJ-Link is link-local — players find each other with broadcasts and claim
//! device numbers by MAC — so a NAT'd guest can only ever talk to the host.
//! Bridging gives the guest its own address on the same segment as everything
//! else, which is what makes a real mixer or a copy of rekordbox see it.
//!
//! # Which shape, and why
//!
//! The kernel's classification of the interface picked decides:
//!
//! * **a tap** — attached as it is. Nothing is created, and nothing is
//!   elevated when the user can open it.
//! * **a bridge** — a tap of ours enslaved to it. The bridge is the user's.
//! * **a physical NIC** — a `macvtap` in bridge mode on it, which gives the
//!   guest its own MAC on the LAN without moving the host's address onto a
//!   bridge.
//!
//! A macvtap cannot reach its own parent's host: rekordbox on this machine
//! does not see the deck. A bridge built by hand and picked does.
//!
//! # Lifetime
//!
//! A link we create outlives the app: it is made once, elevated, owned by the
//! user, and found again on the next launch, so only the first launch after a
//! host boot (or after choosing another interface) asks for a password. Left
//! behind with no guest on it, a macvtap or a tap carries no traffic. It goes
//! when the host reboots, or when a later setup replaces it.

use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{fs, thread};

use cdj3k_emu_platform::net::linux_kind::{Kind, SYS_CLASS_NET};
use cdj3k_emu_platform::runtime_paths;

use crate::elevate::{run_elevated, sh_quote};

/// How the guest is put on the LAN.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// An existing tap the user already owns.
    ExistingTap,
    /// A tap of ours, enslaved to a bridge the user built.
    TapOnBridge,
    /// A macvtap hanging off a physical NIC.
    Macvtap,
}

impl Shape {
    /// What to do with `name`, from the kernel's classification of it.
    pub fn for_iface(name: &str) -> Self {
        Self::of(Kind::of(Path::new(SYS_CLASS_NET), name))
    }

    /// A tap is used as it is and a bridge gets a tap; anything else, a NIC
    /// or an interface that is not there, gets a macvtap, whose creation then
    /// fails with the kernel's own message.
    fn of(kind: Option<Kind>) -> Self {
        match kind {
            Some(Kind::Tap) => Self::ExistingTap,
            Some(Kind::Bridge) => Self::TapOnBridge,
            Some(Kind::Ethernet) | None => Self::Macvtap,
        }
    }
}

/// The guest's link for one instance. The link outlives it (module doc,
/// Lifetime).
pub struct LinuxBridge {
    /// The interface QEMU is attached through — ours, or the user's own tap.
    pub iface: String,
    pub shape: Shape,
    /// Open fd for the tap character device, handed to QEMU as `tap,fd=N` so
    /// the link is never down between setup and the guest's first packet.
    pub fd: RawFd,
    _file: fs::File,
}

impl LinuxBridge {
    /// Put instance `instance_id` on the LAN through `iface`, with `mac`.
    ///
    /// The MAC matters: DJ-Link keys a player's identity on it, so the guest
    /// has to appear with the one the slot was given rather than one the
    /// kernel invents.
    pub fn setup(iface: &str, mac: &str, instance_id: u32) -> io::Result<Self> {
        if !cdj3k_emu_platform::net::is_valid_iface(iface) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid interface name: {iface:?}"),
            ));
        }
        if !cdj3k_emu_platform::net::is_valid_mac(mac) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid MAC: {mac:?}"),
            ));
        }
        let shape = Shape::for_iface(iface);
        if shape == Shape::ExistingTap {
            return Self::open_existing(iface);
        }

        let ours = our_iface_name(instance_id);
        if let Some(bridge) = Self::reuse(shape, &ours, iface, mac) {
            eprintln!("cdj3k-emu: {shape:?} reused on {iface}  iface={ours}  fd={}", bridge.fd);
            return Ok(bridge);
        }

        runtime_paths::ensure_runtime_base_dir()?;
        let dir = runtime_paths::instance_dir(instance_id);
        fs::create_dir_all(&dir)?;
        let base = dir.join("linuxnet");
        let ready = PathBuf::from(format!("{}.ready", base.display()));
        let script = PathBuf::from(format!("{}.sh", base.display()));
        let _ = fs::remove_file(&ready);
        fs::write(
            &script,
            create_script(shape, &ours, iface, mac, &ready, unsafe { libc::getuid() }),
        )?;
        run_elevated(&format!("/bin/sh {}", sh_quote(&script.to_string_lossy())))?;

        // The script writes the device node's path once the link is up and
        // readable by us.
        let node = wait_for_ready(&ready, Duration::from_secs(10))?;
        let bridge = Self::open(shape, &ours, &node)?;
        eprintln!("cdj3k-emu: {shape:?} up on {iface}  iface={ours}  fd={}", bridge.fd);
        Ok(bridge)
    }

    /// The link a previous launch left, if it is still the one asked for: the
    /// same parent, the slot's MAC, up, and openable without elevation.
    fn reuse(shape: Shape, ours: &str, parent: &str, mac: &str) -> Option<Self> {
        let dir = Path::new(SYS_CLASS_NET).join(ours);
        let read = |f: &str| fs::read_to_string(dir.join(f)).ok().map(|s| s.trim().to_string());
        let flags = read("flags").and_then(|f| u32::from_str_radix(f.trim_start_matches("0x"), 16).ok())?;
        const IFF_UP: u32 = 0x1;
        if flags & IFF_UP == 0 {
            return None;
        }
        let node = match shape {
            Shape::Macvtap => {
                if !dir.join(format!("lower_{parent}")).exists()
                    || !read("address")?.eq_ignore_ascii_case(mac)
                {
                    return None;
                }
                format!("/dev/tap{}", read("ifindex")?)
            }
            Shape::TapOnBridge => {
                let master = fs::read_link(dir.join("master")).ok()?;
                if !dir.join("tun_flags").exists()
                    || master.file_name()? != std::ffi::OsStr::new(parent)
                {
                    return None;
                }
                String::from("/dev/net/tun")
            }
            Shape::ExistingTap => return None,
        };
        Self::open(shape, ours, &node).ok()
    }

    /// Open the link's device node for QEMU.
    fn open(shape: Shape, ours: &str, node: &str) -> io::Result<Self> {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(node)
            .map_err(|e| io::Error::new(e.kind(), format!("cannot open {node}: {e}")))?;
        let fd = file.as_raw_fd();
        // A macvtap's node is its interface. `/dev/net/tun` is not: every open
        // of it is unattached until bound by name, and QEMU refuses an
        // unattached fd.
        if shape == Shape::TapOnBridge {
            attach_tun(fd, ours)?;
        }
        // Cleared so the fd survives the fork+exec into QEMU.
        unsafe { libc::fcntl(fd, libc::F_SETFD, 0) };
        Ok(Self {
            iface: ours.to_string(),
            shape,
            fd,
            _file: file,
        })
    }

    /// Attach to a tap the user already made, without creating anything.
    fn open_existing(iface: &str) -> io::Result<Self> {
        let node = "/dev/net/tun";
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(node)
            .map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!("cannot open {node} to attach to {iface}: {e}"),
                )
            })?;
        attach_tun(file.as_raw_fd(), iface)?;
        let fd = file.as_raw_fd();
        unsafe { libc::fcntl(fd, libc::F_SETFD, 0) };
        eprintln!("cdj3k-emu: attached to existing tap {iface}  fd={fd}");
        Ok(Self {
            iface: iface.to_string(),
            shape: Shape::ExistingTap,
            fd,
            _file: file,
        })
    }
}

/// Bind an already-open `/dev/net/tun` fd to an existing tap by name.
fn attach_tun(fd: RawFd, iface: &str) -> io::Result<()> {
    // struct ifreq: 16 bytes of name, then the flags in the union.
    const IFF_TAP: i16 = 0x0002;
    const IFF_NO_PI: i16 = 0x1000;
    const TUNSETIFF: libc::c_ulong = 0x4004_54ca;
    let mut req = [0u8; 40];
    let name = iface.as_bytes();
    if name.len() >= 16 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "interface name too long",
        ));
    }
    req[..name.len()].copy_from_slice(name);
    req[16..18].copy_from_slice(&(IFF_TAP | IFF_NO_PI).to_ne_bytes());
    let rc = unsafe { libc::ioctl(fd, TUNSETIFF, req.as_mut_ptr()) };
    if rc < 0 {
        let e = io::Error::last_os_error();
        return Err(io::Error::new(
            e.kind(),
            format!("TUNSETIFF on {iface} failed: {e} (is it yours, and a tap?)"),
        ));
    }
    Ok(())
}

/// The name of the interface we create for an instance.
///
/// Kept inside IFNAMSIZ (15 usable characters) and stamped with the slot, so
/// two slots on one machine cannot collide and a leftover is identifiable.
fn our_iface_name(instance_id: u32) -> String {
    format!("cdj3k{instance_id}")
}

/// The elevated half: replace any link of ours, make it, hand it to the user.
/// All privileged work happens here.
fn create_script(
    shape: Shape,
    ours: &str,
    parent: &str,
    mac: &str,
    ready: &std::path::Path,
    uid: u32,
) -> String {
    let q_ours = sh_quote(ours);
    let q_parent = sh_quote(parent);
    let q_mac = sh_quote(mac);
    let q_ready = sh_quote(&ready.to_string_lossy());

    // macvtap's device node is /dev/tap<ifindex>; a plain tap is reached
    // through /dev/net/tun and bound by name, so only the first needs chown.
    let create = match shape {
        Shape::Macvtap => format!(
            "ip link add link {q_parent} name {q_ours} address {q_mac} type macvtap mode bridge\n\
             ip link set {q_ours} up\n\
             idx=$(cat /sys/class/net/{q_ours}/ifindex)\n\
             node=/dev/tap$idx\n\
             for i in 1 2 3 4 5 6 7 8 9 10; do [ -e \"$node\" ] && break; sleep 0.2; done\n\
             chown {uid} \"$node\"\n"
        ),
        // The tap keeps the MAC the kernel gave it; only the guest uses the
        // slot's. A bridge counts each port's own address as its own and
        // passes frames for it up to the host, so a tap wearing the guest's
        // MAC would swallow every unicast frame meant for the guest.
        Shape::TapOnBridge => format!(
            "ip tuntap add dev {q_ours} mode tap user {uid}\n\
             ip link set {q_ours} master {q_parent}\n\
             ip link set {q_ours} up\n\
             node=/dev/net/tun\n"
        ),
        // Nothing is created for a tap the user already owns.
        Shape::ExistingTap => String::new(),
    };

    format!(
        "#!/bin/sh\n\
         # cdj3k-emu: makes the guest's link. It stays after the app exits and\n\
         # is reused by the next launch; a later run of this replaces it.\n\
         set -e\n\
         ip link del {q_ours} 2>/dev/null || true\n\
         {create}\
         printf '%s' \"$node\" > {q_ready}\n"
    )
}

fn wait_for_ready(path: &std::path::Path, timeout: Duration) -> io::Result<String> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Ok(s) = fs::read_to_string(path) {
            let s = s.trim();
            if !s.is_empty() {
                return Ok(s.to_string());
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "the elevated helper did not bring the interface up in time",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kernels_classification_picks_the_shape() {
        assert_eq!(Shape::of(Some(Kind::Tap)), Shape::ExistingTap);
        assert_eq!(Shape::of(Some(Kind::Bridge)), Shape::TapOnBridge);
        assert_eq!(Shape::of(Some(Kind::Ethernet)), Shape::Macvtap);
        assert_eq!(Shape::of(None), Shape::Macvtap);
    }

    /// A link of ours from an earlier run is replaced, never stacked on, and
    /// the new one is left in place for the next launch.
    #[test]
    fn the_script_replaces_and_keeps_the_link() {
        for shape in [Shape::Macvtap, Shape::TapOnBridge] {
            let s = create_script(
                shape,
                "cdj3k1",
                "eno2",
                "0e:12:b7:bc:af:b1",
                std::path::Path::new("/run/ready"),
                1000,
            );
            assert_eq!(s.matches("ip link del 'cdj3k1'").count(), 1, "{shape:?}:\n{s}");
            let del = s.find("ip link del").unwrap();
            let add = s.find("ip link add").or_else(|| s.find("ip tuntap add")).unwrap();
            assert!(del < add, "{shape:?}: the old link must go first:\n{s}");
            assert!(s.trim_end().ends_with("> '/run/ready'"), "{shape:?}:\n{s}");
        }
    }

    /// On a bridge the slot's MAC belongs to the guest alone. Given to the tap
    /// as well, it becomes the bridge's own address, and unicast for the
    /// guest (a DHCP offer, the LINK handshake) is delivered to the host.
    #[test]
    fn a_bridge_tap_does_not_wear_the_guests_mac() {
        let s = create_script(
            Shape::TapOnBridge,
            "cdj3k1",
            "br0",
            "0e:12:b7:bc:af:b1",
            std::path::Path::new("/run/ready"),
            1000,
        );
        assert!(!s.contains("0e:12:b7:bc:af:b1"), "{s}");
        assert!(s.contains("master 'br0'"), "{s}");
    }

    /// The guest keeps the MAC its slot was given: DJ-Link identity is keyed
    /// on it, so a kernel-invented one would make the deck a different player
    /// every launch.
    #[test]
    fn the_slots_mac_reaches_the_link() {
        let s = create_script(
            Shape::Macvtap,
            "cdj3k1",
            "eno2",
            "0e:12:b7:bc:af:b1",
            std::path::Path::new("/run/ready"),
            1000,
        );
        assert!(s.contains("address '0e:12:b7:bc:af:b1'"), "{s}");
    }
}
