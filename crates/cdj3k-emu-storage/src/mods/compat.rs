//! The firmware a mod declares it runs on, in the optional `[compat]` table of
//! `mod.toml`, keyed by model slug:
//!
//! ```toml
//! [compat]
//! cdj3k  = ">=3.13, <=3.20"
//! cdj3kx = "*"
//! ```
//!
//! A range is `*` or comma-separated clauses. A clause is a release with an
//! optional `>=`, `<=`, `>`, `<` or `=` in front; a release without an
//! operator, such as `3.20`, matches only 3.20.
//!
//! A mod without the table, or with an empty one, runs on every firmware.
//! Otherwise a model the table does not list is incompatible, and a slot
//! whose release is unknown matches every range.

use std::cmp::Ordering;

use cdj3k_emu_panel::Model;

/// The `[compat]` table: (slug, range), sorted by slug.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Declared {
    pub ranges: Vec<(String, String)>,
}

/// Whether a mod runs on a slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Compat {
    /// No `[compat]` table, or an empty one.
    Unchecked,
    Compatible,
    /// `wanted` is the range declared for the slot's model, `None` when the
    /// model is not listed; `models` are the slugs the table lists.
    Incompatible {
        wanted: Option<String>,
        models: Vec<String>,
    },
    /// The `mod.toml` is missing or invalid, and why; such a mod never runs.
    Invalid(String),
}

impl Compat {
    pub fn runs(&self) -> bool {
        matches!(self, Compat::Unchecked | Compat::Compatible)
    }
}

impl Declared {
    /// Parse the `[compat]` table.
    pub fn parse(compat: &toml::Value) -> Result<Declared, String> {
        let compat = compat
            .as_table()
            .ok_or_else(|| "`compat` is not a table".to_string())?;
        let mut out = Vec::new();
        for (slug, range) in compat {
            let range = range
                .as_str()
                .ok_or_else(|| format!("compat.{slug} is not a string"))?;
            if !Model::ALL.iter().any(|m| m.slug() == slug) {
                return Err(format!(
                    "compat.{slug}: not a model ({})",
                    Model::ALL.map(|m| m.slug()).join(", ")
                ));
            }
            Range::parse(range).map_err(|e| format!("compat.{slug}: {e}"))?;
            out.push((slug.clone(), range.to_string()));
        }
        Ok(Declared { ranges: out })
    }

    pub fn check(&self, model: Model, release: Option<&str>) -> Compat {
        if self.ranges.is_empty() {
            return Compat::Unchecked;
        }
        let wanted = self
            .ranges
            .iter()
            .find(|(slug, _)| slug == model.slug())
            .map(|(_, range)| range.clone());
        let fits = match (&wanted, release) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(range), Some(release)) => Range::parse(range).is_ok_and(|r| r.contains(release)),
        };
        if fits {
            Compat::Compatible
        } else {
            Compat::Incompatible {
                wanted,
                models: self.ranges.iter().map(|(s, _)| s.clone()).collect(),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Ge,
    Le,
    Gt,
    Lt,
    Eq,
}

struct Range(Vec<(Op, Vec<u32>)>);

impl Range {
    fn parse(s: &str) -> Result<Range, String> {
        let s = s.trim();
        if s == "*" {
            return Ok(Range(Vec::new()));
        }
        let mut clauses = Vec::new();
        for clause in s.split(',') {
            let clause = clause.trim();
            let (op, rest) = [
                (">=", Op::Ge),
                ("<=", Op::Le),
                ("==", Op::Eq),
                (">", Op::Gt),
                ("<", Op::Lt),
                ("=", Op::Eq),
            ]
            .iter()
            .find_map(|(p, op)| clause.strip_prefix(p).map(|rest| (*op, rest)))
            .unwrap_or((Op::Eq, clause));
            let version =
                parse_release(rest.trim()).ok_or_else(|| format!("{clause:?} is not a range"))?;
            clauses.push((op, version));
        }
        Ok(Range(clauses))
    }

    fn contains(&self, release: &str) -> bool {
        let Some(release) = parse_release(release) else {
            return true;
        };
        self.0.iter().all(|(op, v)| {
            let ord = compare(&release, v);
            match op {
                Op::Ge => ord != Ordering::Less,
                Op::Le => ord != Ordering::Greater,
                Op::Gt => ord == Ordering::Greater,
                Op::Lt => ord == Ordering::Less,
                Op::Eq => ord == Ordering::Equal,
            }
        })
    }
}

fn parse_release(s: &str) -> Option<Vec<u32>> {
    if s.is_empty() {
        return None;
    }
    s.split('.').map(|p| p.parse().ok()).collect()
}

/// Compare two releases number by number; a missing number counts as 0.
fn compare(a: &[u32], b: &[u32]) -> Ordering {
    let n = a.len().max(b.len());
    (0..n)
        .map(|i| a.get(i).unwrap_or(&0).cmp(b.get(i).unwrap_or(&0)))
        .find(|o| *o != Ordering::Equal)
        .unwrap_or(Ordering::Equal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Declared, String> {
        let table: toml::Table = text.parse().map_err(|e| format!("{e}"))?;
        match table.get("compat") {
            Some(v) => Declared::parse(v),
            None => Ok(Declared::default()),
        }
    }

    fn declared(text: &str) -> Declared {
        parse(text).unwrap()
    }

    #[test]
    fn no_table_is_unchecked() {
        assert_eq!(
            declared("").check(Model::Cdj3k, Some("3.22")),
            Compat::Unchecked
        );
        assert_eq!(
            declared("name = \"x\"").check(Model::Cdj3k, Some("3.22")),
            Compat::Unchecked
        );
    }

    #[test]
    fn ranges_bound_the_release() {
        let d = declared("[compat]\ncdj3k = \">=3.13, <=3.20\"\ncdj3kx = \"*\"\n");
        assert_eq!(d.check(Model::Cdj3k, Some("3.19")), Compat::Compatible);
        assert_eq!(d.check(Model::Cdj3k, Some("3.20")), Compat::Compatible);
        assert_eq!(
            d.check(Model::Cdj3k, Some("3.22")),
            Compat::Incompatible {
                wanted: Some(">=3.13, <=3.20".into()),
                models: vec!["cdj3k".into(), "cdj3kx".into()],
            }
        );
        assert_eq!(d.check(Model::Cdj3kx, Some("1.40")), Compat::Compatible);
    }

    #[test]
    fn an_unlisted_model_is_incompatible() {
        let d = declared("compat = { cdj3k = \"*\" }");
        assert!(!d.check(Model::Cdj1500x, Some("1.10")).runs());
        assert!(!d.check(Model::Cdj1500x, None).runs());
    }

    #[test]
    fn an_unknown_release_passes() {
        let d = declared("[compat]\ncdj3k = \"3.20\"\n");
        assert_eq!(d.check(Model::Cdj3k, None), Compat::Compatible);
        assert_eq!(d.check(Model::Cdj3k, Some("3.20")), Compat::Compatible);
        assert!(!d.check(Model::Cdj3k, Some("3.19")).runs());
    }

    #[test]
    fn releases_compare_numerically() {
        let d = declared("[compat]\ncdj3k = \">3.9\"\n");
        assert_eq!(d.check(Model::Cdj3k, Some("3.10")), Compat::Compatible);
        let d = declared("[compat]\ncdj3k = \"<3.20\"\n");
        assert!(!d.check(Model::Cdj3k, Some("3.20.1")).runs());
    }

    #[test]
    fn bad_tables_are_refused() {
        assert!(parse("[compat]\ncdj2000 = \"*\"\n").is_err());
        assert!(parse("[compat]\ncdj3k = \"latest\"\n").is_err());
        assert!(parse("[compat]\ncdj3k = 3\n").is_err());
        assert!(parse("compat = \"*\"\n").is_err());
    }
}
