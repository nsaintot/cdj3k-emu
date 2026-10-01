//! macOS: vmnet, or an OpenVPN-style tap bridged at the OS level.
//!
//! vmnet cannot bridge to a tap interface, so a tap goes through
//! [`super::tapbridge`] instead.

use std::io;

use super::NetAttachment;
use super::tapbridge::TapBridge;
use super::vmnet::VmnetMode;

/// What an attachment keeps for as long as the guest runs; dropping the tap
/// bridge tears it down.
pub enum NetKeepAlive {
    /// vmnet, which QEMU opens and closes itself.
    Nothing,
    Tap(TapBridge),
}

/// vmnet's host-only network: the guests and this Mac, nothing else.
pub fn attach_host_only() -> io::Result<NetAttachment> {
    Ok(NetAttachment {
        vmnet: Some(VmnetMode::Host),
        tap_iface: None,
        tap_fd: None,
        keep: NetKeepAlive::Nothing,
    })
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
