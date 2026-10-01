//! Hosts with no interface enumeration.
//!
//! The menu shows no interfaces and the guest stays on QEMU's user-mode NAT,
//! which needs no interface.

use super::NetIf;

pub const HOST_ONLY: bool = false;

pub fn enumerate() -> Vec<NetIf> {
    Vec::new()
}

pub fn exists(_name: &str) -> bool {
    false
}
