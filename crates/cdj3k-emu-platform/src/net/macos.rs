//! macOS bridgeability, decided by name.
//!
//! There is no sysfs, so an allowlist of prefixes decides: `en*` (BSD NICs)
//! and `tap*` (a hand-built segment). `utun*`, `awdl*`, `llw*` and `bridge*`
//! are excluded.

use super::NetIf;

pub const HOST_ONLY: bool = true;

pub use super::unix::exists;

pub fn enumerate() -> Vec<NetIf> {
    super::unix::walk(bridgeable)
}

fn bridgeable(name: &str) -> bool {
    name.starts_with("en") || name.starts_with("tap")
}
