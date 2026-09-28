//! Hosts with no bridging backend.

use std::io;

use super::NetAttachment;

/// Nothing to keep alive: no attachment is ever made here.
pub enum NetKeepAlive {
    Nothing,
}

/// No host-only network on this host.
pub fn attach_host_only() -> io::Result<NetAttachment> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no host-only network on this host",
    ))
}

/// Hosts with no bridging backend: the guest stays on the default NAT link.
pub fn attach(iface: &str, _mac: &str, _instance_id: u32) -> io::Result<NetAttachment> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!("this host cannot bridge onto {iface:?}"),
    ))
}
