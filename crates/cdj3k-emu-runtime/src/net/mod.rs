//! Putting the guest on a chosen interface, whatever the host calls that.
//!
//! The first spawn in `launch.rs` and the worker's reconcile both ask here,
//! so a selection made at boot and one made while running behave the same.

/// macOS-only: the user-mode TAP bridge is built out of `ifconfig bridge` and
/// an elevated watcher script.
#[cfg(target_os = "macos")]
pub mod tapbridge;
pub mod vmnet;

/// The slot's hold on its bridged link, the same on every host.
pub mod lease;

#[cfg(target_os = "linux")]
pub mod linux_net;

/// The text side of Windows bridging, checked on every host.
pub mod winbridge;
#[cfg(target_os = "windows")]
pub mod windows_helper;
#[cfg(target_os = "windows")]
pub mod windows_net;

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

pub use imp::{NetKeepAlive, attach, attach_host_only, run_helper_if_asked, tap_netdev};

use vmnet::VmnetMode;

/// What QEMU has to be told, and what has to stay alive while it runs.
pub struct NetAttachment {
    pub vmnet: Option<VmnetMode>,
    pub tap_iface: Option<String>,
    pub tap_fd: Option<i32>,
    /// Whatever the attachment needs kept for as long as the guest runs.
    pub keep: NetKeepAlive,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An interface name carrying shell characters is refused on every host.
    #[test]
    fn a_hostile_name_is_refused_before_anything_is_created() {
        for bad in ["eth0; id", "eth0 $(id)", "", "../../etc"] {
            assert!(
                attach(bad, "0e:12:b7:bc:af:b1", 1).is_err(),
                "{bad:?} was accepted"
            );
        }
    }

    /// The tap netdev names what the host hands QEMU: a descriptor, or on
    /// Windows the adapter.
    #[test]
    fn the_tap_netdev_follows_the_host() {
        let arg = tap_netdev("net0", Some("cdj3k-emu-0"), Some(7));
        #[cfg(target_os = "windows")]
        assert_eq!(arg.as_deref(), Some("tap,id=net0,ifname=cdj3k-emu-0"));
        #[cfg(not(target_os = "windows"))]
        assert_eq!(arg.as_deref(), Some("tap,id=net0,fd=7"));
        assert_eq!(tap_netdev("net0", None, None), None);
    }

    /// A MAC carrying shell characters is refused.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_hostile_mac_is_refused_too() {
        assert!(attach("eno2", "0a:11:22:33:44:55; id", 1).is_err());
    }
}
