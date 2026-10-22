//! vmnet.framework networking for Pro DJ Link, via QEMU's native backends.
//!
//! QEMU opens the vmnet interface itself, inside this process:
//!   -netdev vmnet-bridged,id=net0,ifname=en0
//!   -netdev vmnet-host,id=net0,net-uuid=<UUID>
//!
//! vmnet refuses both unless the calling process is root or carries the
//! `com.apple.developer.networking.vmnet` entitlement.  That entitlement is
//! signed onto `cdj3k-emu` and authorised by
//! `Contents/embedded.provisionprofile` (see `packaging/macos/build.sh`).  Entitlements are
//! process-wide, so QEMU reaches vmnet through them from inside
//! libcdj3k-emu-qemu.dylib - the same arrangement HVF uses.
//!
//! Without the profile vmnet returns `VMNET_FAILURE` and QEMU fails to start.
//! Where the entitlement grants only part of vmnet (macOS 15 refuses bridged
//! mode and filters host mode), QEMU reaches it through a root `socket_vmnet`
//! instead ([`super::vmnet_helper`]):
//!   -netdev stream,id=net0,server=off,reconnect-ms=1000,addr.type=unix,addr.path=<sock>

use std::path::PathBuf;

/// vmnet network identifier for host-only mode.
///
/// Maps to vmnet.framework's `vmnet_network_identifier_key`: every interface
/// sharing this UUID joins one isolated L2 network with no DHCP server.
/// Where the entitlement covers host mode, interfaces created under one UUID
/// by separate processes see each other's broadcasts, so several instances
/// (and djx-emu) share one DJ-Link segment. On macOS 15 the root
/// `socket_vmnet` joins the same UUID instead ([`super::vmnet_helper`]).
///
/// Without DHCP, the `udhcpc -T 2` call in the guest's `link-monitor.sh` times
/// out and falls through to `avahi-autoipd`, which assigns a 169.254/16
/// link-local address. Real Pro DJ Link gear does the same on a network
/// without a router.
///
/// `DJPL_VMNET_UUID` overrides it; djx-emu must be given the same value to land
/// on the same segment (the counterpart to `DJPL_BRIDGE` for TAP mode).
const VMNET_HOST_NETWORK_ID: &str = "7b3d5e2a-9c14-4f6b-b2e1-0a1c2d3e4f50";

pub(crate) fn host_network_id() -> String {
    std::env::var("DJPL_VMNET_UUID").unwrap_or_else(|_| VMNET_HOST_NETWORK_ID.to_string())
}

/// Which vmnet backend QEMU should open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VmnetMode {
    /// Isolated DHCP-less segment shared by every instance on the same UUID.
    Host,
    /// Bridged onto a physical interface, putting guests on the real LAN.
    Bridged(String),
    /// A root `socket_vmnet` daemon listening on this Unix socket, which runs
    /// one of the modes above for every instance connected to it. QEMU
    /// reconnects every second while the daemon is gone.
    Socket(PathBuf),
}

impl VmnetMode {
    /// Bridged onto `iface`, or `None` when the name is not valid on a command
    /// line.
    pub fn bridged(iface: &str) -> Option<Self> {
        cdj3k_emu_platform::net::is_valid_iface(iface)
            .then(|| VmnetMode::Bridged(iface.to_string()))
    }

    /// The `-netdev` argument for this mode.
    ///
    /// `isolated` stays at its default (off) in both modes: on, it cuts each
    /// guest off from the others on the same vmnet network.
    pub fn netdev_arg(&self, id: &str) -> String {
        match self {
            VmnetMode::Host => {
                format!("vmnet-host,id={id},net-uuid={}", host_network_id())
            }
            VmnetMode::Bridged(iface) => format!("vmnet-bridged,id={id},ifname={iface}"),
            VmnetMode::Socket(path) => format!(
                "stream,id={id},server=off,reconnect-ms=1000,addr.type=unix,addr.path={}",
                path.display()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridged_names_the_interface_and_host_carries_the_uuid() {
        assert_eq!(
            VmnetMode::Bridged("en0".into()).netdev_arg("net0"),
            "vmnet-bridged,id=net0,ifname=en0"
        );
        let host = VmnetMode::Host.netdev_arg("net0");
        assert!(host.starts_with("vmnet-host,id=net0,net-uuid="));
        assert!(host.contains(&host_network_id()));
    }

    #[test]
    fn a_socket_connects_as_a_stream_client() {
        assert_eq!(
            VmnetMode::Socket(PathBuf::from("/var/run/cdj3k-emu-501/vmnet-br-en0.sock"))
                .netdev_arg("net0"),
            "stream,id=net0,server=off,reconnect-ms=1000,addr.type=unix,addr.path=/var/run/cdj3k-emu-501/vmnet-br-en0.sock"
        );
    }

    #[test]
    fn bridged_rejects_names_outside_the_grammar() {
        assert!(VmnetMode::bridged("en0 ; reboot").is_none());
        assert!(VmnetMode::bridged("").is_none());
    }
}
