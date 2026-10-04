//! The updater: what the latest release offers this installation, and the
//! steps that replace it.
//!
//! Every release carries one index, `cdj3k-emu-update.json`, and its Ed25519
//! signature (`.github/scripts/update-index.sh`, signed in `cd.yml`), read
//! from `releases/latest/download/`: the newest published release that is not
//! a pre-release.
//!
//! * [`check`] reads the index, verifies it against the update key compiled
//!   in here, and picks the package for this OS, architecture and package
//!   format ([`installed_kind`]).
//! * [`download`] fetches that package and checks its size and SHA-256
//!   against the signed index.
//! * [`prepare`] lays the new build down beside the running one while the
//!   emulation keeps running; for a deb or an rpm, [`hand_over`] moves the
//!   package to the downloads folder for the package manager instead.
//! * [`close_other_slots`] has every other window stop and close, and
//!   [`apply`] puts the prepared build in place - at a restart, which starts
//!   the app again on it, or at a quit.

mod fetch;
mod index;
mod install;
mod installed;
pub mod native;
mod signature;
mod slots;
mod version;

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

pub use index::{Index, Kind, Package};
pub use install::{apply, hand_over, prepare, Prepared};
pub use installed::installed_kind;
pub use slots::{close_other_slots, other_slots_open};
pub use version::Version;

const LATEST: &str = "https://github.com/nsaintot/cdj3k-emu/releases/latest/download";
const INDEX_NAME: &str = "cdj3k-emu-update.json";

/// Where the latest release's index is.
pub fn index_url() -> String {
    format!("{LATEST}/{INDEX_NAME}")
}

/// Where the latest release's Sparkle feed is ([`native`]).
pub fn appcast_url() -> String {
    format!("{LATEST}/appcast.xml")
}

/// The page that lists every release, for an installation the updater does
/// not manage.
pub const RELEASES_PAGE: &str = "https://github.com/nsaintot/cdj3k-emu/releases";

/// What the latest release means for this installation.
#[derive(Clone, Debug)]
pub enum Status {
    /// Nothing newer than this build. `notes` and `release_notes` are the
    /// latest release's, as in [`Offer`].
    Current {
        latest: String,
        notes: String,
        release_notes: String,
    },
    /// A newer release with a package for this installation.
    Available(Offer),
    /// A newer release, but no package of this installation's format for this
    /// platform.
    NoPackage { latest: String },
    /// A build not installed from a release package - a developer's build -
    /// which the updater leaves alone.
    Unmanaged,
}

/// A newer release, and the package of it that replaces this installation.
#[derive(Clone, Debug)]
pub struct Offer {
    pub version: String,
    /// The release's page.
    pub notes: String,
    /// The release's notes, as Markdown.
    pub release_notes: String,
    pub kind: Kind,
    pub package: Package,
}

/// Ask the latest release what it offers this installation.
pub fn check() -> Result<Status, Error> {
    let Some(kind) = installed_kind() else {
        return Ok(Status::Unmanaged);
    };
    let url = index_url();
    let data = fetch::get(&url).map_err(|e| {
        if e.not_found {
            Error::new(
                "the latest release carries no update index; it was published \
                 before the updater - download it from the releases page",
            )
        } else {
            e
        }
    })?;
    let sig = fetch::get(&format!("{url}.sig"))?;
    let sig =
        String::from_utf8(sig).map_err(|_| Error::new("the update index signature is not text"))?;
    signature::verify(&data, &sig)?;
    let index = Index::parse(&data)?;
    evaluate(
        &index,
        cdj3k_emu_platform::app_meta::VERSION,
        std::env::consts::OS,
        std::env::consts::ARCH,
        kind,
    )
}

/// [`check`]'s decision, for an index already verified.
fn evaluate(
    index: &Index,
    running: &str,
    os: &str,
    arch: &str,
    kind: Kind,
) -> Result<Status, Error> {
    let latest = Version::parse(&index.version).ok_or_else(|| {
        Error::new(format!(
            "the update index names version {:?}",
            index.version
        ))
    })?;
    let running = Version::parse(running)
        .ok_or_else(|| Error::new(format!("this build's version {running:?} does not parse")))?;
    if latest <= running {
        return Ok(Status::Current {
            latest: latest.to_string(),
            notes: index.notes.clone(),
            release_notes: index.release_notes.clone(),
        });
    }
    Ok(match index.package_for(os, arch, kind) {
        Some(package) => Status::Available(Offer {
            version: latest.to_string(),
            notes: index.notes.clone(),
            release_notes: index.release_notes.clone(),
            kind,
            package: package.clone(),
        }),
        None => Status::NoPackage {
            latest: latest.to_string(),
        },
    })
}

/// Where a download for `version` goes: a directory of its own under the
/// app's per-user runtime root, which an elevated installer can read.
pub fn download_dir(version: &str) -> PathBuf {
    cdj3k_emu_platform::runtime_paths::runtime_base_dir().join(format!("update-{version}"))
}

/// Fetch `offer`'s package, checked against the signed index. `progress`
/// hears the bytes so far and the total; setting `cancel` stops it.
pub fn download(
    offer: &Offer,
    progress: &mut dyn FnMut(u64, u64),
    cancel: &AtomicBool,
) -> Result<PathBuf, Error> {
    cdj3k_emu_platform::runtime_paths::ensure_runtime_base_dir()?;
    fetch::download(
        &offer.package,
        &download_dir(&offer.version),
        progress,
        cancel,
    )
}

/// Remove what [`download`] left for `version`.
pub fn discard_download(version: &str) {
    let _ = std::fs::remove_dir_all(download_dir(version));
}

/// Why an update step did not go through, in words for the update window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    message: String,
    cancelled: bool,
    not_found: bool,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            cancelled: false,
            not_found: false,
        }
    }

    /// The user stopped it: a cancelled download, a declined password prompt.
    pub fn cancelled() -> Self {
        Self {
            message: "cancelled".into(),
            cancelled: true,
            not_found: false,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::new(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/index.json");

    fn status(running: &str, os: &str, arch: &str, kind: Kind) -> Status {
        evaluate(&Index::parse(FIXTURE).unwrap(), running, os, arch, kind).unwrap()
    }

    #[test]
    fn offers_a_newer_release_with_this_installations_package() {
        match status("0.3.0", "windows", "aarch64", Kind::Inno) {
            Status::Available(o) => {
                assert_eq!(o.version, "0.4.0");
                assert!(o.package.url.ends_with("windows-arm64.exe"));
            }
            s => panic!("{s:?}"),
        }
        // A pre-release of the same version is older than it.
        assert!(matches!(
            status("0.4.0-rc1", "macos", "aarch64", Kind::Dmg),
            Status::Available(_)
        ));
    }

    #[test]
    fn the_same_or_a_newer_build_is_current() {
        for running in ["0.4.0", "0.4.1", "0.5.0-rc1"] {
            assert!(
                matches!(
                    status(running, "linux", "x86_64", Kind::Deb),
                    Status::Current { .. }
                ),
                "{running}"
            );
        }
    }

    #[test]
    fn a_release_without_this_format_says_so() {
        assert!(matches!(
            status("0.3.0", "linux", "x86_64", Kind::Rpm),
            Status::NoPackage { .. }
        ));
    }
}
