//! Which Windows adapters can carry a guest.
//!
//! Windows reports every adapter the same way, physical or not, so the
//! decision comes from the interface type, the operational state and the
//! driver's description. The rules are plain data over [`Adapter`], so they
//! run on any host.

/// `IF_TYPE_ETHERNET_CSMACD`.
pub const IF_TYPE_ETHERNET: u32 = 6;
/// `IF_TYPE_IEEE80211`.
pub const IF_TYPE_WIFI: u32 = 71;

/// One entry of the host's adapter list.
#[derive(Clone, Debug, PartialEq)]
pub struct Adapter {
    /// The connection name shown in Network Connections ("Ethernet 2").
    pub friendly_name: String,
    /// The driver's description ("Intel(R) Ethernet Connection I219-V").
    pub description: String,
    pub if_type: u32,
    pub up: bool,
    /// First unicast IPv4 address and its prefix length.
    pub ipv4: Option<(String, u8)>,
}

/// Description fragments of adapters that are not a LAN port: virtual
/// switches, tunnels, VPNs, the tap driver itself and the bridge.
const NOT_A_LAN_PORT: &[&str] = &[
    "tap-windows",
    "tap0901",
    "hyper-v",
    "vethernet",
    "virtual",
    "vmware",
    "vpn",
    "wintun",
    "wireguard",
    "bluetooth",
    "wan miniport",
    "loopback",
    "kernel debug",
    "mac bridge",
    "network bridge",
    "multiplexor",
    "pseudo",
];

impl Adapter {
    /// A physical Ethernet or Wi-Fi adapter that is up, with or without an
    /// address: a bridge member's IPv4 binding is the bridge's.
    pub fn bridgeable(&self) -> bool {
        if !self.up {
            return false;
        }
        if self.if_type != IF_TYPE_ETHERNET && self.if_type != IF_TYPE_WIFI {
            return false;
        }
        let description = self.description.to_lowercase();
        let name = self.friendly_name.to_lowercase();
        !NOT_A_LAN_PORT
            .iter()
            .any(|frag| description.contains(frag) || name.contains(frag))
    }

    pub fn is_wifi(&self) -> bool {
        self.if_type == IF_TYPE_WIFI
    }

    /// The tap driver's own adapter.
    pub fn is_tap(&self) -> bool {
        self.description.to_lowercase().contains("tap-windows")
    }

    /// The Network Bridge's own adapter: "Microsoft Network Adapter
    /// Multiplexor Driver" on Windows 11, "Microsoft MAC Bridge Virtual NIC"
    /// before it.
    pub fn is_bridge(&self) -> bool {
        let description = self.description.to_lowercase();
        description.contains("mac bridge") || description.contains("multiplexor")
    }
}

/// A connection name safe to write into a batch file and to hand to `netsh`:
/// letters of any script, digits, space, `.`, `_`, `-`, at most 64
/// characters, and no edge spaces.
pub fn is_valid_adapter_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= 64
        && name == name.trim()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(name: &str, desc: &str, if_type: u32) -> Adapter {
        Adapter {
            friendly_name: name.into(),
            description: desc.into(),
            if_type,
            up: true,
            ipv4: Some(("192.168.1.42".into(), 24)),
        }
    }

    #[test]
    fn physical_ethernet_and_wifi_are_kept() {
        assert!(a(
            "Ethernet",
            "Intel(R) Ethernet Connection I219-V",
            IF_TYPE_ETHERNET
        )
        .bridgeable());
        assert!(a("Wi-Fi", "Intel(R) Wi-Fi 6 AX201 160MHz", IF_TYPE_WIFI).bridgeable());
        assert!(a("Wi-Fi", "x", IF_TYPE_WIFI).is_wifi());
    }

    #[test]
    fn virtual_adapters_are_dropped() {
        for (name, desc) in [
            (
                "vEthernet (Default Switch)",
                "Hyper-V Virtual Ethernet Adapter",
            ),
            ("Ethernet 3", "TAP-Windows Adapter V9"),
            ("cdj3k-emu-0", "TAP-Windows Adapter V9 #2"),
            ("Network Bridge", "Microsoft MAC Bridge Virtual NIC"),
            (
                "Network Bridge",
                "Microsoft Network Adapter Multiplexor Driver",
            ),
            (
                "VMware Network Adapter VMnet1",
                "VMware Virtual Ethernet Adapter for VMnet1",
            ),
            ("Ethernet 4", "VirtualBox Host-Only Ethernet Adapter"),
            (
                "Local Area Connection* 1",
                "Microsoft Wi-Fi Direct Virtual Adapter",
            ),
            (
                "Bluetooth Network Connection",
                "Bluetooth Device (Personal Area Network)",
            ),
        ] {
            assert!(!a(name, desc, IF_TYPE_ETHERNET).bridgeable(), "{name}");
        }
    }

    #[test]
    fn both_bridge_adapter_names_are_recognised() {
        assert!(a(
            "Network Bridge",
            "Microsoft Network Adapter Multiplexor Driver",
            6
        )
        .is_bridge());
        assert!(a("Network Bridge", "Microsoft MAC Bridge Virtual NIC", 6).is_bridge());
        assert!(!a("Ethernet", "Parallels VirtIO Ethernet Adapter", 6).is_bridge());
    }

    #[test]
    fn a_bridge_member_without_an_address_is_kept() {
        let mut member = a("Ethernet", "Realtek PCIe GbE", IF_TYPE_ETHERNET);
        member.ipv4 = None;
        assert!(member.bridgeable());
    }

    #[test]
    fn other_types_and_dead_links_are_dropped() {
        assert!(!a("Loopback", "Software Loopback Interface 1", 24).bridgeable());
        assert!(!a("PPP", "WAN", 23).bridgeable());
        let mut down = a("Ethernet", "Realtek PCIe GbE", IF_TYPE_ETHERNET);
        down.up = false;
        assert!(!down.bridgeable());
    }

    #[test]
    fn tap_and_bridge_are_recognised() {
        assert!(a("x", "TAP-Windows Adapter V9", IF_TYPE_ETHERNET).is_tap());
        assert!(a(
            "Network Bridge",
            "Microsoft MAC Bridge Virtual NIC",
            IF_TYPE_ETHERNET
        )
        .is_bridge());
        assert!(!a("Ethernet", "Realtek PCIe GbE", IF_TYPE_ETHERNET).is_tap());
    }

    #[test]
    fn adapter_names_carry_no_shell_characters() {
        for good in [
            "Ethernet",
            "Ethernet 2",
            "Wi-Fi",
            "Connexion au réseau local",
            "cdj3k-emu-0",
        ] {
            assert!(is_valid_adapter_name(good), "{good:?}");
        }
        for bad in [
            "",
            " Ethernet",
            "Ethernet ",
            "a\"b",
            "a&b",
            "a|b",
            "a%PATH%",
            "a^b",
            "eth0; id",
            "eth0 $(id)",
            "../../etc",
            "a\nb",
            &"x".repeat(65),
        ] {
            assert!(!is_valid_adapter_name(bad), "{bad:?}");
        }
    }
}
