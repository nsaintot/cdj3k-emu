//! Putting a verified package in place, in two steps.
//!
//! [`prepare`] does the slow part while the emulation keeps running: the new
//! build is unpacked, checked and laid down beside the running one, where
//! nothing runs it. [`apply`] is what is left for the restart, or for a
//! quit: for an AppImage a single rename.
//!
//! | Kind       | Prepared                                      | Applied                       | Started again by  |
//! |------------|-----------------------------------------------|-------------------------------|-------------------|
//! | `appimage` | the image copied beside `$APPIMAGE`           | a rename over `$APPIMAGE`     | this process      |
//! | `inno`     | nothing: Windows cannot replace a loaded file | Setup, `/SILENT /UPDATE=1`    | Setup, unelevated |
//!
//! A deb or an rpm is left to the system's package manager: [`hand_over`]
//! puts the verified package in the user's downloads folder for it. A macOS
//! bundle is Sparkle's ([`crate::native`]).
//!
//! The swap waits for the restart, so a slot the running build opens from the
//! Instances menu runs the same build.
//!
//! A restart starts the app once, as a plain launch would, with
//! `--after-update` so it waits for this process to let its slot go rather
//! than finding the slot taken.

use std::path::{Path, PathBuf};

use crate::{Error, Kind};

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod imp;
#[cfg(windows)]
#[path = "windows.rs"]
mod imp;
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
#[path = "unsupported.rs"]
mod imp;

/// A new build laid down beside the running one, waiting for [`apply`].
pub struct Prepared(imp::Staged);

/// Lay `package`, of `kind`, down beside the running build. The emulation can
/// keep running; nothing that runs is touched.
pub fn prepare(package: &Path, kind: Kind) -> Result<Prepared, Error> {
    imp::prepare(package, kind).map(Prepared)
}

/// Put `prepared` in place of the running build and, with `relaunch`, have
/// the app start again on it once this process exits; a quit passes false.
/// Every other window must already have closed. On success, exit now.
pub fn apply(prepared: &Prepared, relaunch: bool) -> Result<(), Error> {
    imp::apply(&prepared.0, relaunch)
}

/// Move a verified `package` the package manager installs into the user's
/// downloads folder. Returns where it now is.
pub fn hand_over(package: &Path, kind: Kind) -> Result<PathBuf, Error> {
    imp::hand_over(package, kind)
}

fn wrong_kind(kind: Kind) -> Error {
    Error::new(format!("this host cannot install a {kind:?} package"))
}
