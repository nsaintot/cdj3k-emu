//! macOS: vmnet, or an OpenVPN-style tap bridged at the OS level.
//!
//! vmnet cannot bridge to a tap interface, so a tap goes through
//! [`super::tapbridge`] instead. Where vmnet treats this process as a limited
//! client, QEMU reaches vmnet through [`super::vmnet_helper`].

use std::io;

use super::tapbridge::TapBridge;
use super::vmnet::VmnetMode;
use super::vmnet_helper::{self, SocketVmnet};
use super::NetAttachment;

/// What an attachment keeps for as long as the guest runs; dropping the tap
/// bridge tears it down, and dropping the helper releases its lease.
pub enum NetKeepAlive {
    /// In-process vmnet, which QEMU opens and closes itself.
    Nothing,
    Tap(TapBridge),
    VmnetHelper(SocketVmnet),
}

/// Attaches to vmnet's host-only network, which connects only the guests and
/// this Mac.
pub fn attach_host_only() -> io::Result<NetAttachment> {
    if vmnet_helper::use_helper(None) {
        return Ok(through_helper(SocketVmnet::start_host()?));
    }
    Ok(NetAttachment {
        vmnet: Some(VmnetMode::Host),
        tap_iface: None,
        tap_fd: None,
        keep: NetKeepAlive::Nothing,
    })
}

fn through_helper(helper: SocketVmnet) -> NetAttachment {
    NetAttachment {
        vmnet: Some(VmnetMode::Socket(helper.socket_path().to_path_buf())),
        tap_iface: None,
        tap_fd: None,
        keep: NetKeepAlive::VmnetHelper(helper),
    }
}

/// Attach to `iface`, presenting the guest with `mac`.
///
/// `mac` is the slot's own: DJ-Link keys a player's identity on it, so it has
/// to survive onto whatever link the guest ends up behind. Both paths here
/// take it from QEMU's own command line, so it is not used.
pub fn attach(iface: &str, mac: &str, instance_id: u32) -> io::Result<NetAttachment> {
    let _ = mac;
    if iface.starts_with("tap") {
        let tb = TapBridge::setup(iface, instance_id)?;
        Ok(NetAttachment {
            vmnet: None,
            tap_iface: Some(tb.qemu_tap.clone()),
            tap_fd: Some(tb.qemu_tap_fd),
            keep: NetKeepAlive::Tap(tb),
        })
    } else {
        match VmnetMode::bridged(iface) {
            Some(_) if vmnet_helper::use_helper(Some(iface)) => {
                Ok(through_helper(SocketVmnet::start_bridged(iface)?))
            }
            Some(mode) => Ok(NetAttachment {
                vmnet: Some(mode),
                tap_iface: None,
                tap_fd: None,
                keep: NetKeepAlive::Nothing,
            }),
            None => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("this host cannot bridge onto {iface:?}"),
            )),
        }
    }
}

/// QEMU is handed the tap as an inherited descriptor.
pub fn tap_netdev(id: &str, _iface: Option<&str>, fd: Option<i32>) -> Option<String> {
    fd.map(|fd| format!("tap,id={id},fd={fd}"))
}

/// There is no privileged helper entry point on this host.
pub fn run_helper_if_asked(_args: &[String]) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tap_netdev_is_the_descriptor() {
        assert_eq!(
            tap_netdev("net0", Some("tap0"), Some(7)).as_deref(),
            Some("tap,id=net0,fd=7")
        );
        assert_eq!(tap_netdev("net0", None, None), None);
    }
}
