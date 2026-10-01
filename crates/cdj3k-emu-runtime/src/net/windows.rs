//! Windows: a TAP-Windows6 adapter joined to the chosen NIC by the Network
//! Bridge. See [`super::windows_net`].

use std::io;

use super::windows_net::WindowsBridge;
use super::NetAttachment;

pub use super::windows_helper::run_helper_if_asked;

/// What an attachment keeps for as long as the guest runs.
pub enum NetKeepAlive {
    /// No bridge.
    Nothing,
    Windows(WindowsBridge),
}

/// No host-only network on this host.
pub fn attach_host_only() -> io::Result<NetAttachment> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no host-only network on this host",
    ))
}

/// Attach to the NIC `iface`, presenting the guest with `mac`.
pub fn attach(iface: &str, mac: &str, instance_id: u32) -> io::Result<NetAttachment> {
    let bridge = WindowsBridge::setup(iface, mac, instance_id)?;
    Ok(NetAttachment {
        vmnet: None,
        tap_iface: Some(bridge.iface.clone()),
        tap_fd: None,
        keep: NetKeepAlive::Windows(bridge),
    })
}

/// QEMU opens the tap adapter by its connection name.
pub fn tap_netdev(id: &str, iface: Option<&str>, _fd: Option<i32>) -> Option<String> {
    iface.map(|name| format!("tap,id={id},ifname={name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tap_netdev_is_the_adapter() {
        assert_eq!(
            tap_netdev("net0", Some("cdj3k-emu-0"), None).as_deref(),
            Some("tap,id=net0,ifname=cdj3k-emu-0")
        );
        assert_eq!(tap_netdev("net0", None, None), None);
    }
}
