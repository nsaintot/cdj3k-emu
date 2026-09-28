//! Hosts with no virtual-endpoint backend.

use std::path::Path;

#[derive(Debug)]
pub enum PcLinkError {
    Unsupported,
}

impl std::fmt::Display for PcLinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "pc-link bridge requires macOS")
    }
}
impl std::error::Error for PcLinkError {}

impl PcLinkError {
    pub fn is_retryable(&self) -> bool {
        false
    }
}

/// False: this build has no virtual-endpoint backend, so the menu offers no
/// PC-link switch.
pub const SUPPORTED: bool = false;

pub struct PcLink;

impl PcLink {
    pub fn start(_sock_dir: &Path, _instance_id: u32) -> Result<Self, PcLinkError> {
        Err(PcLinkError::Unsupported)
    }

    pub fn is_alive(&self) -> bool {
        false
    }

    pub fn drain_while_down(&mut self) -> usize {
        0
    }

    pub fn reconnect(&mut self) -> Result<(), PcLinkError> {
        Err(PcLinkError::Unsupported)
    }
}
