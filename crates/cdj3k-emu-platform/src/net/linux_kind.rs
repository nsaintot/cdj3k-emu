//! What Linux can put a guest on, decided by sysfs rather than by name.
//!
//! Compiled on every host so the classification is tested on every host.

/// Where the kernel lists network interfaces.
pub const SYS_CLASS_NET: &str = "/sys/class/net";

/// What a Linux interface can become.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A physical wired NIC, which gets a macvtap.
    Ethernet,
    /// A bridge, which gets a tap enslaved to it.
    Bridge,
    /// A tap, used as it is.
    Tap,
}

impl Kind {
    /// Classify `<root>/<name>` from the kernel's own entries.
    ///
    /// Taken from a root rather than a literal so the layout can be built in a
    /// test without a Linux kernel behind it.
    pub fn of(root: &std::path::Path, name: &str) -> Option<Self> {
        let dir = root.join(name);
        if dir.join("tun_flags").exists() {
            return Some(Self::Tap);
        }
        if dir.join("bridge").is_dir() {
            return Some(Self::Bridge);
        }
        // A Wi-Fi station may only send frames under its own MAC. The access
        // point drops a macvtap's, and the kernel will not enslave a station
        // to a bridge, so neither shape can carry the guest.
        if dir.join("wireless").exists() || dir.join("phy80211").exists() {
            return None;
        }
        // A backing device is what separates a NIC from the virtual links
        // that also report as Ethernet: veths, and macvtaps including our own.
        if dir.join("device").exists() {
            return Some(Self::Ethernet);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::Kind;

    /// Each kind laid out the way the kernel lays it out in sysfs, under
    /// names chosen to mislead: nothing here may be decided by the name.
    #[test]
    fn the_kernel_decides_not_the_name() {
        let root = std::env::temp_dir().join(format!("cdj3k-netkind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mk = |name: &str, entries: &[&str]| {
            let dir = root.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            for e in entries {
                if let Some(sub) = e.strip_suffix('/') {
                    std::fs::create_dir_all(dir.join(sub)).unwrap();
                } else {
                    std::fs::write(dir.join(e), "").unwrap();
                }
            }
        };
        mk("eth0", &["device"]);
        mk("wlo1", &["device", "wireless/", "phy80211"]);
        mk("lan", &["bridge/"]);
        mk("vpn0", &["tun_flags"]);
        mk("en_veth", &[]);
        mk("cdj3k1", &[]);

        assert_eq!(Kind::of(&root, "eth0"), Some(Kind::Ethernet));
        assert_eq!(Kind::of(&root, "lan"), Some(Kind::Bridge));
        assert_eq!(Kind::of(&root, "vpn0"), Some(Kind::Tap));
        assert_eq!(Kind::of(&root, "wlo1"), None, "a Wi-Fi station cannot carry the guest");
        assert_eq!(Kind::of(&root, "en_veth"), None, "no backing device, not a NIC");
        assert_eq!(Kind::of(&root, "cdj3k1"), None, "our own macvtap is not a parent");
        assert_eq!(Kind::of(&root, "absent"), None);

        let _ = std::fs::remove_dir_all(&root);
    }
}
