//! Linux: a bridge, a macvtap or an existing tap, chosen from what the host
//! has. See [`super::linux_net`].

use std::io;

use super::NetAttachment;
use super::linux_net::LinuxBridge;

/// What an attachment keeps for as long as the guest runs.
pub enum NetKeepAlive {
    /// No bridge.
    Nothing,
    Linux(LinuxBridge),
}

/// No host-only network on this host.
pub fn attach_host_only() -> io::Result<NetAttachment> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no host-only network on this host",
    ))
}

/// Attach to `iface`, presenting the guest with `mac`.
///
/// `mac` is the slot's own: DJ-Link keys a player's identity on it, so it has
/// to survive onto whatever link the guest ends up behind. Anything this host
/// creates needs the MAC written onto it, since QEMU does not set one there.
pub fn attach(iface: &str, mac: &str, instance_id: u32) -> io::Result<NetAttachment> {
    let bridge = LinuxBridge::setup(iface, mac, instance_id)?;
    Ok(NetAttachment {
        vmnet: None,
        tap_iface: Some(bridge.iface.clone()),
        tap_fd: Some(bridge.fd),
        keep: NetKeepAlive::Linux(bridge),
    })
}
