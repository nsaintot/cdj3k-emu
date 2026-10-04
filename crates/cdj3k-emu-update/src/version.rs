//! Release versions: `MAJOR.MINOR.PATCH[-PRE]`, the shape the release tag is
//! checked against in `cd.yml`, ordered by semver precedence.

use std::cmp::Ordering;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    core: [u64; 3],
    /// The dot-separated identifiers after the hyphen; empty for a release.
    pre: Vec<String>,
}

impl Version {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('v');
        let s = s.split_once('+').map_or(s, |(v, _build)| v);
        let (core, pre) = match s.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (s, None),
        };
        let mut parts = core.split('.');
        let mut field = || -> Option<u64> {
            let p = parts.next()?;
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            p.parse().ok()
        };
        let parsed = [field()?, field()?, field()?];
        if parts.next().is_some() {
            return None;
        }
        let pre = match pre {
            None => Vec::new(),
            Some(pre) => {
                let ids: Vec<String> = pre.split('.').map(str::to_owned).collect();
                let valid = |id: &String| {
                    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                };
                if !ids.iter().all(valid) {
                    return None;
                }
                ids
            }
        };
        Some(Self { core: parsed, pre })
    }

    /// Whether this is a pre-release (`0.3.0-rc1`).
    pub fn is_pre(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core.cmp(&other.core).then_with(|| {
            // A release outranks any of its pre-releases.
            match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => cmp_pre(&self.pre, &other.pre),
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Identifier by identifier: numbers numerically and below words, words in
/// ASCII order, and a shorter list first when one is a prefix of the other.
fn cmp_pre(a: &[String], b: &[String]) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        let o = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            (Err(_), Err(_)) => x.cmp(y),
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    a.len().cmp(&b.len())
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [major, minor, patch] = self.core;
        write!(f, "{major}.{minor}.{patch}")?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre.join("."))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap_or_else(|| panic!("{s} should parse"))
    }

    #[test]
    fn parses_the_tag_shapes() {
        assert_eq!(v("0.3.0").to_string(), "0.3.0");
        assert_eq!(v("v0.3.0").to_string(), "0.3.0");
        assert_eq!(v("0.3.0-rc1").to_string(), "0.3.0-rc1");
        assert_eq!(v("1.2.3-beta.2+build.7").to_string(), "1.2.3-beta.2");
        for bad in [
            "",
            "0.3",
            "0.3.0.1",
            "0.3.x",
            "0..1",
            "0.3.0-",
            "0.3.0-rc..1",
            " -1.0.0",
        ] {
            assert!(Version::parse(bad).is_none(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn orders_by_semver_precedence() {
        let ordered = [
            "0.2.0",
            "0.3.0-alpha",
            "0.3.0-alpha.1",
            "0.3.0-alpha.beta",
            "0.3.0-beta.2",
            "0.3.0-beta.11",
            "0.3.0-rc1",
            "0.3.0",
            "0.3.1",
            "0.10.0",
            "1.0.0",
        ];
        for pair in ordered.windows(2) {
            assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
        }
        assert!(v("0.3.0-rc1").is_pre());
        assert!(!v("0.3.0").is_pre());
    }
}
