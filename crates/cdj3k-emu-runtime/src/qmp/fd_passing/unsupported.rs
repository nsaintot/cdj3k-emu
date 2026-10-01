//! A host with no descriptor passing: what QEMU needs is opened by path
//! ([`crate::disk::PASSES_FDS`]), so nothing is ever handed over.

use std::convert::Infallible;
use std::path::Path;

use crate::qmp::QmpError;

/// Uninhabited: there is no descriptor to hand over.
pub type Fd<'a> = &'a Infallible;

pub fn add_fd(_socket: &Path, fd: Fd<'_>) -> Result<u64, QmpError> {
    match *fd {}
}
