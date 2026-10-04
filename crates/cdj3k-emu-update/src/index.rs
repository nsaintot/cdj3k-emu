//! `cdj3k-emu-update.json`, the index every release carries: its version and
//! one entry per package. `.github/scripts/update-index.sh` writes it.

use serde::Deserialize;

use crate::Error;

/// The schema this build reads. A release raises it only for a change an
/// older reader would get wrong.
pub const SCHEMA: u32 = 1;

#[derive(Clone, Debug, Deserialize)]
pub struct Index {
    pub schema: u32,
    pub version: String,
    /// The release's page.
    pub notes: String,
    /// The release's notes, as Markdown; empty when the index has none.
    #[serde(default)]
    pub release_notes: String,
    pub packages: Vec<Package>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Package {
    /// `std::env::consts::OS`: `macos`, `linux` or `windows`.
    pub os: String,
    /// `std::env::consts::ARCH`, or `universal` for a macOS bundle holding
    /// both slices.
    pub arch: String,
    pub kind: Kind,
    pub url: String,
    pub size: u64,
    /// Lowercase hex.
    pub sha256: String,
}

/// The package formats a release ships, each installed its own way.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// The macOS bundle, in a disk image.
    Dmg,
    Deb,
    Rpm,
    AppImage,
    /// The Windows Inno Setup installer.
    Inno,
    /// A format a later release added.
    #[serde(other)]
    Other,
}

impl Kind {
    /// Whether this crate installs the format itself. A deb or an rpm is the
    /// system package manager's to install, and a macOS bundle Sparkle's
    /// ([`crate::native`]).
    pub fn installs_itself(self) -> bool {
        matches!(self, Self::AppImage | Self::Inno)
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim() {
            "dmg" => Self::Dmg,
            "deb" => Self::Deb,
            "rpm" => Self::Rpm,
            "appimage" => Self::AppImage,
            "inno" => Self::Inno,
            _ => return None,
        })
    }
}

impl Index {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let index: Self = serde_json::from_slice(bytes)
            .map_err(|e| Error::new(format!("the update index is malformed: {e}")))?;
        if index.schema > SCHEMA {
            return Err(Error::new(format!(
                "the update index is schema {}, newer than this build reads; \
                 download the new release from its page",
                index.schema
            )));
        }
        Ok(index)
    }

    /// The package that replaces an installation of `kind` on `os`/`arch`:
    /// one built for the architecture, or failing that a universal one.
    pub fn package_for(&self, os: &str, arch: &str, kind: Kind) -> Option<&Package> {
        let of_kind = || {
            self.packages
                .iter()
                .filter(move |p| p.os == os && p.kind == kind)
        };
        of_kind()
            .find(|p| p.arch == arch)
            .or_else(|| of_kind().find(|p| p.arch == "universal"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/index.json");

    #[test]
    fn picks_the_package_for_each_installation() {
        let index = Index::parse(FIXTURE).unwrap();
        assert_eq!(index.version, "0.4.0");
        let pick = |os, arch, kind| index.package_for(os, arch, kind).map(|p| p.url.as_str());
        assert!(pick("macos", "aarch64", Kind::Dmg)
            .unwrap()
            .ends_with("macos-universal.dmg"));
        assert!(pick("macos", "x86_64", Kind::Dmg)
            .unwrap()
            .ends_with("macos-universal.dmg"));
        assert!(pick("windows", "aarch64", Kind::Inno)
            .unwrap()
            .ends_with("windows-arm64.exe"));
        assert!(pick("windows", "x86_64", Kind::Inno)
            .unwrap()
            .ends_with("windows-x64.exe"));
        assert!(pick("linux", "x86_64", Kind::Deb)
            .unwrap()
            .ends_with("x86_64.deb"));
        assert!(pick("linux", "aarch64", Kind::AppImage).is_some());
        // The fixture has no aarch64 deb and no rpm at all.
        assert!(pick("linux", "aarch64", Kind::Deb).is_none());
        assert!(pick("linux", "x86_64", Kind::Rpm).is_none());
    }

    #[test]
    fn an_unknown_format_is_kept_out_of_the_way() {
        let json = br#"{"schema":1,"version":"9.0.0","notes":"",
            "packages":[{"os":"linux","arch":"x86_64","kind":"flatpak",
            "url":"u","size":1,"sha256":"00"}]}"#;
        let index = Index::parse(json).unwrap();
        assert_eq!(index.packages[0].kind, Kind::Other);
        assert!(index.package_for("linux", "x86_64", Kind::Deb).is_none());
    }

    #[test]
    fn a_newer_schema_is_refused() {
        let json = br#"{"schema":2,"version":"9.0.0","notes":"","packages":[]}"#;
        assert!(Index::parse(json).is_err());
    }
}
