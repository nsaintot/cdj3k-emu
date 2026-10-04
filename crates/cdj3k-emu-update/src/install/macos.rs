//! macOS: Sparkle installs the bundle ([`crate::native`]), so this crate
//! installs nothing here.

use std::path::{Path, PathBuf};

use super::wrong_kind;
use crate::{Error, Kind};

pub(super) struct Staged;

pub(super) fn prepare(_package: &Path, kind: Kind) -> Result<Staged, Error> {
    Err(wrong_kind(kind))
}

pub(super) fn apply(_staged: &Staged, _relaunch: bool) -> Result<(), Error> {
    Err(Error::new("Sparkle installs updates on macOS"))
}

pub(super) fn hand_over(_package: &Path, kind: Kind) -> Result<PathBuf, Error> {
    Err(wrong_kind(kind))
}
