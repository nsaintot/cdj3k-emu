//! Linux bridgeability, decided by the kernel.
//!
//! Linux names nothing usefully — an `en*` prefix is a systemd-udev habit and
//! a veth can be called anything — so the classification comes from sysfs via
//! [`super::linux_kind`], and the two lists it produces are then merged.

use std::path::Path;

use super::linux_kind::{Kind, SYS_CLASS_NET};
use super::NetIf;

pub const HOST_ONLY: bool = false;

pub use super::unix::exists;

pub fn enumerate() -> Vec<NetIf> {
    let mut out = super::unix::walk(bridgeable);

    // A tap or a bridge made for a guest to sit on carries no address of its
    // own, so `getifaddrs` never reports it — and on Linux those are exactly
    // the interfaces worth bridging onto. They are listed from sysfs instead.
    for name in bridgeable_without_address() {
        if !out.iter().any(|i| i.name == name) {
            out.push(NetIf {
                name,
                addr: String::new(),
                prefix_len: 0,
            });
        }
    }

    out
}

fn bridgeable(name: &str) -> bool {
    Kind::of(Path::new(SYS_CLASS_NET), name).is_some()
}

/// Interfaces a guest can be put on that have no address of their own: taps
/// and bridges, which is what someone building a DJ-Link segment by hand ends
/// up with.
fn bridgeable_without_address() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(SYS_CLASS_NET) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !matches!(
            Kind::of(Path::new(SYS_CLASS_NET), &name),
            Some(Kind::Tap | Kind::Bridge)
        ) {
            continue;
        }
        // Down interfaces are listed: a tap with nothing attached reads DOWN
        // until the guest opens it, which is precisely when it is picked.
        out.push(name);
    }
    out.sort();
    out
}
