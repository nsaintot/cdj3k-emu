//! Interfaces a guest can be put on, as a host sees them.
//!
//! The walk over the host's address list is the same everywhere it exists;
//! what differs is which interfaces it keeps. macOS has only names to go on,
//! Linux asks the kernel. So the walk lives in [`unix`] and takes the host's
//! filter as a parameter, and each host supplies its own.
//!
//! Windows lists adapters with `GetIfTable2`, filtered by [`windows_kind`].
//!
//! A host with none of these enumerates nothing (`unsupported.rs`).

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[path = "unix.rs"]
mod unix;

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod imp;
#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod imp;
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
#[path = "unsupported.rs"]
mod imp;

pub mod linux_kind;
pub mod windows_kind;

/// Whether the host offers a host-only network (vmnet's), which a QEMU built
/// without it has no netdev for.
pub use imp::HOST_ONLY;

/// A single discovered network interface.
#[derive(Clone, Debug, PartialEq)]
pub struct NetIf {
    /// e.g. "en0"
    pub name: String,
    /// IPv4 address, e.g. "192.168.1.42"
    pub addr: String,
    /// CIDR prefix length, e.g. 24
    pub prefix_len: u8,
}

impl NetIf {
    /// Menu label: "192.168.1.42/24 (en0)", or the bare name for an interface
    /// with no IPv4 address.
    pub fn label(&self) -> String {
        if self.addr.is_empty() {
            return self.name.clone();
        }
        format!("{}/{} ({})", self.addr, self.prefix_len, self.name)
    }
}

/// An interface name safe to put on a QEMU command line or in a root shell:
/// at most 15 bytes (`IFNAMSIZ` less its NUL), letters, digits, `.`, `_`, `-`.
pub fn is_valid_iface(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 16
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
}

/// Six colon-separated hex octets.
pub fn is_valid_mac(mac: &str) -> bool {
    let mut octets = 0;
    for part in mac.split(':') {
        if part.len() != 2 || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
            return false;
        }
        octets += 1;
    }
    octets == 6
}

/// Every interface this host can carry a guest, in the order the host
/// reports them.
///
/// An entry with an empty [`NetIf::addr`] is one that has no address of its
/// own — a tap or a bridge waiting for a guest — which some hosts can offer
/// and others cannot.
pub fn enumerate_interfaces() -> Vec<NetIf> {
    imp::enumerate()
}

/// Whether the host has an interface named `name`, offered or not.
pub fn interface_exists(name: &str) -> bool {
    imp::exists(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both reach a root shell, so neither may carry anything but its own
    /// characters.
    #[test]
    fn nothing_shell_shaped_passes_validation() {
        for bad in [
            "eth0; rm -rf /",
            "eth0 $(id)",
            "eth0`id`",
            "eth0'",
            "en 0",
            "",
            "..",
            "averyveryverylongname",
        ] {
            assert!(!is_valid_iface(bad), "{bad:?} was accepted");
        }
        for good in ["en0", "eno2", "br-lan", "bridge99", "eth0.100"] {
            assert!(is_valid_iface(good), "{good:?} was refused");
        }

        for bad in [
            "",
            "0a:11:22:33:44",
            "0a:11:22:33:44:55:66",
            "zz:11:22:33:44:55",
            "02-11-22-33-44-55",
            "0a1122334455",
        ] {
            assert!(!is_valid_mac(bad), "{bad:?} was accepted");
        }
        assert!(is_valid_mac("0e:12:b7:bc:af:b1"));
    }
}
