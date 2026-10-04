//! No installer on this host.

use std::path::{Path, PathBuf};

use super::wrong_kind;
use crate::{Error, Kind};

pub(super) struct Staged;

pub(super) fn prepare(_package: &Path, kind: Kind) -> Result<Staged, Error> {
    Err(wrong_kind(kind))
}

pub(super) fn apply(_staged: &Staged, _relaunch: bool) -> Result<(), Error> {
    Err(Error::new("this host has no installer"))
}

pub(super) fn hand_over(_package: &Path, kind: Kind) -> Result<PathBuf, Error> {
    Err(wrong_kind(kind))
}
