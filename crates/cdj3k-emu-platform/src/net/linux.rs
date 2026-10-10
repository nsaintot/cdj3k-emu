//! Linux bridgeability, decided by the kernel.
//!
//! Linux interface names carry no reliable meaning: the `en*` prefix is a
//! systemd-udev convention and a veth can have any name. The classification
//! therefore comes from sysfs through [`super::linux_kind`], and the two lists
//! it produces are merged.

use std::path::Path;

use super::linux_kind::{Kind, SYS_CLASS_NET};
use super::NetIf;

pub const HOST_ONLY: bool = false;

pub use super::unix::exists;

pub fn enumerate() -> Vec<NetIf> {
    let mut out = super::unix::walk(bridgeable);

    // A tap or a bridge made for a guest has no address of its own, so
    // `getifaddrs` never reports it, yet on Linux these are the interfaces to
    // bridge onto. They are listed from sysfs instead.
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

/// Interfaces a guest can be put on that have no address of their own: the
/// taps and bridges of a DJ-Link segment built by hand.
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
        // Down interfaces are listed, because a tap with nothing attached reads
        // DOWN until the guest opens it, and the guest opens it only once it is
        // picked.
        out.push(name);
    }
    out.sort();
    out
}
