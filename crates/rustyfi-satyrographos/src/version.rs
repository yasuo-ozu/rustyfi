//! A semver-triple value type ([`Version`]) and dependency-requirement
//! language ([`Constraint`]) for the phase-7c solver (`solve.rs`).
//!
//! This replaces the ad-hoc string `registry::version_cmp` with a
//! real, totally-ordered value type, and adds the upstream-faithful
//! constraint syntax the solver recurses on: **caret-only** requirements
//! (`^X.Y.Z`), matching `saphe-split:src-util/semanticVersion.ml` /
//! `packageConstraintSolver.ml`.
//!
//! No `semver` crate: the surface here is deliberately small (parse, order,
//! caret-match, compat-bucket) and self-contained, so the crate keeps its
//! `std`-only, hand-rolled resolver footprint.

use std::cmp::Ordering;
use std::fmt;

use crate::error::Error;

/// A semver triple plus an optional pre-release tag: `major.minor.patch` or
/// `major.minor.patch-pre`.
///
/// Ordering compares `(major, minor, patch)` numerically first; when those
/// are equal, a version *with* a pre-release sorts strictly below the same
/// version with none (`1.2.0-rc < 1.2.0`), and two pre-releases at the same
/// triple fall back to a plain string compare of their tags — SATySFi
/// package pre-releases are not standardised beyond "some tag", so a total,
/// deterministic order is what matters here (not full SemVer 2.0 pre-release
/// precedence).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Option<String>,
}

impl Version {
    /// A release version (no pre-release tag), without going through
    /// [`Version::parse`].
    pub fn new(major: u64, minor: u64, patch: u64) -> Self {
        Version {
            major,
            minor,
            patch,
            pre: None,
        }
    }

    /// Parse `"X.Y.Z"` or `"X.Y.Z-pre"` (each of `X`, `Y`, `Z` a bare
    /// non-negative integer, no leading `+`/build metadata — SATySFi package
    /// versions do not use it). [`Error::InvalidVersion`] on any other shape.
    pub fn parse(s: &str) -> Result<Version, Error> {
        let (core, pre) = match s.split_once('-') {
            Some((core, pre)) => (core, Some(pre.to_string())),
            None => (s, None),
        };
        let mut parts = core.split('.');
        let (Some(maj), Some(min), Some(pat), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(invalid(s, "expected major.minor.patch"));
        };
        let major = parse_component(s, maj)?;
        let minor = parse_component(s, min)?;
        let patch = parse_component(s, pat)?;
        if let Some(pre) = &pre {
            if pre.is_empty() {
                return Err(invalid(s, "empty pre-release tag after '-'"));
            }
        }
        Ok(Version {
            major,
            minor,
            patch,
            pre,
        })
    }
}

fn parse_component(whole: &str, s: &str) -> Result<u64, Error> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid(whole, "version components must be non-negative integers"));
    }
    s.parse::<u64>()
        .map_err(|_| invalid(whole, "version component out of range"))
}

fn invalid(text: &str, message: &str) -> Error {
    Error::InvalidVersion {
        text: text.to_string(),
        message: message.to_string(),
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                // A release sorts ABOVE its own pre-release.
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

/// A dependency-version requirement. `Caret` is the upstream-faithful default
/// (the *only* syntax `saphe-split` accepts); `Exact` and `Any` are kept so
/// the pre-existing `Satyristes` exact pin (`"1.2.3"`) and the
/// `Satyristes` wildcard (`"*"`) map onto the same type without a format
/// change to either front end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Constraint {
    /// `^X.Y.Z[-pre]` — npm/cargo-style caret compatibility.
    Caret(Version),
    /// An exact pin: matches only that one version.
    Exact(Version),
    /// `*` — matches every version.
    Any,
}

impl Constraint {
    /// Parse `"^x.y.z"` (caret), `"x.y.z"` (exact), or `"*"` (any).
    pub fn parse(s: &str) -> Result<Constraint, Error> {
        let s = s.trim();
        if s == "*" || s.is_empty() {
            return Ok(Constraint::Any);
        }
        if let Some(rest) = s.strip_prefix('^') {
            return Ok(Constraint::Caret(Version::parse(rest)?));
        }
        Ok(Constraint::Exact(Version::parse(s)?))
    }

    /// Whether `v` satisfies this requirement (must match
    /// `semanticVersion.ml`'s `is_compatible`):
    ///
    /// - `Any` always matches.
    /// - `Exact(req)` matches only `v == req`.
    /// - `Caret(req)`:
    ///   - a pre-release `v` never satisfies a requirement whose own `req` is
    ///     not itself a pre-release (pre-releases only satisfy a requirement
    ///     that explicitly names a pre);
    ///   - `req.major == 0`: `v` must share `major == 0` **and** `minor ==
    ///     req.minor`, with `v >= req` (0.x releases are not
    ///     cross-minor-compatible);
    ///   - `req.major >= 1`: `v` must share `major`, with `v >= req`.
    pub fn matches(&self, v: &Version) -> bool {
        match self {
            Constraint::Any => true,
            Constraint::Exact(req) => v == req,
            Constraint::Caret(req) => {
                if v.pre.is_some() && req.pre.is_none() {
                    return false;
                }
                if req.major == 0 {
                    v.major == 0 && v.minor == req.minor && v >= req
                } else {
                    v.major == req.major && v >= req
                }
            }
        }
    }

    /// The upstream compat bucket a version belongs to: `(major, minor if
    /// major == 0 else 0)`. Two versions in different buckets are different
    /// *roles* to the solver — never interchangeable, matching upstream's
    /// `RegisteredRole{package_id, compatibility}`.
    pub fn bucket(v: &Version) -> (u64, u64) {
        if v.major == 0 {
            (0, v.minor)
        } else {
            (v.major, 0)
        }
    }

    /// The compat bucket this constraint *pins*, if any (`Caret`/`Exact` pin
    /// exactly one bucket via their reference version; `Any` pins none). Used
    /// by the solver to tell a genuine "no matching version published" apart
    /// from "two requirers disagree on an incompatible bucket."
    pub fn pinned_bucket(&self) -> Option<(u64, u64)> {
        match self {
            Constraint::Any => None,
            Constraint::Exact(v) | Constraint::Caret(v) => Some(Constraint::bucket(v)),
        }
    }
}

impl fmt::Display for Constraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Constraint::Any => write!(f, "*"),
            Constraint::Exact(v) => write!(f, "{v}"),
            Constraint::Caret(v) => write!(f, "^{v}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_triple() {
        let v = Version::parse("1.2.3").unwrap();
        assert_eq!(v, Version::new(1, 2, 3));
        assert_eq!(v.to_string(), "1.2.3");
    }

    #[test]
    fn parse_with_prerelease() {
        let v = Version::parse("1.2.3-rc1").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
        assert_eq!(v.pre.as_deref(), Some("rc1"));
        assert_eq!(v.to_string(), "1.2.3-rc1");
    }

    #[test]
    fn parse_rejects_bad_shapes() {
        assert!(Version::parse("1.2").is_err());
        assert!(Version::parse("1.2.3.4").is_err());
        assert!(Version::parse("1.2.x").is_err());
        assert!(Version::parse("v1.2.3").is_err());
        assert!(Version::parse("1.2.3-").is_err());
        assert!(Version::parse("").is_err());
    }

    #[test]
    fn ord_numeric_not_lexical() {
        assert!(Version::new(1, 10, 0) > Version::new(1, 9, 0));
        assert!(Version::new(2, 0, 0) > Version::new(1, 99, 99));
        assert_eq!(Version::new(1, 0, 0), Version::new(1, 0, 0));
    }

    #[test]
    fn prerelease_sorts_below_release() {
        let rc = Version::parse("1.2.0-rc").unwrap();
        let rel = Version::parse("1.2.0").unwrap();
        assert!(rc < rel);
        assert!(rel > rc);
    }

    #[test]
    fn two_prereleases_compare_by_tag() {
        let a = Version::parse("1.2.0-alpha").unwrap();
        let b = Version::parse("1.2.0-beta").unwrap();
        assert!(a < b);
    }

    #[test]
    fn constraint_parse_forms() {
        assert_eq!(Constraint::parse("*").unwrap(), Constraint::Any);
        assert_eq!(
            Constraint::parse("1.2.3").unwrap(),
            Constraint::Exact(Version::new(1, 2, 3))
        );
        assert_eq!(
            Constraint::parse("^1.2.3").unwrap(),
            Constraint::Caret(Version::new(1, 2, 3))
        );
        assert!(Constraint::parse("^1.2").is_err());
        assert!(Constraint::parse(">=1.2.3").is_err());
        assert!(Constraint::parse("~1.2.3").is_err());
    }

    #[test]
    fn caret_zero_major_is_minor_locked() {
        let req = Constraint::Caret(Version::new(0, 3, 1));
        assert!(req.matches(&Version::new(0, 3, 1)));
        assert!(req.matches(&Version::new(0, 3, 9)), "patch bump within 0.3 OK");
        assert!(!req.matches(&Version::new(0, 3, 0)), "below the required patch");
        assert!(!req.matches(&Version::new(0, 4, 0)), "0.x minor bump is NOT compatible");
        assert!(!req.matches(&Version::new(1, 3, 1)), "major bump is NOT compatible");
    }

    #[test]
    fn caret_ge_one_major_is_major_locked() {
        let req = Constraint::Caret(Version::new(1, 2, 3));
        assert!(req.matches(&Version::new(1, 2, 3)));
        assert!(req.matches(&Version::new(1, 9, 9)), "minor/patch bump within major 1 OK");
        assert!(!req.matches(&Version::new(1, 2, 2)), "below the required (minor,patch)");
        assert!(!req.matches(&Version::new(2, 0, 0)), "major bump is NOT compatible");
    }

    #[test]
    fn caret_prerelease_only_matches_prerelease_requirement() {
        let stable_req = Constraint::Caret(Version::new(1, 0, 0));
        let pre_v = Version::parse("1.0.5-rc1").unwrap();
        assert!(
            !stable_req.matches(&pre_v),
            "a pre-release must not satisfy a non-pre requirement"
        );

        let pre_req = Constraint::Caret(Version::parse("1.0.0-rc1").unwrap());
        assert!(pre_req.matches(&Version::parse("1.0.5-rc1").unwrap()));
    }

    #[test]
    fn exact_and_any() {
        let exact = Constraint::Exact(Version::new(1, 2, 3));
        assert!(exact.matches(&Version::new(1, 2, 3)));
        assert!(!exact.matches(&Version::new(1, 2, 4)));

        assert!(Constraint::Any.matches(&Version::new(0, 0, 0)));
        assert!(Constraint::Any.matches(&Version::new(99, 99, 99)));
    }

    #[test]
    fn bucket_zero_major_is_per_minor() {
        assert_eq!(Constraint::bucket(&Version::new(0, 3, 1)), (0, 3));
        assert_eq!(Constraint::bucket(&Version::new(0, 3, 9)), (0, 3));
        assert_ne!(
            Constraint::bucket(&Version::new(0, 3, 1)),
            Constraint::bucket(&Version::new(0, 4, 0))
        );
    }

    #[test]
    fn bucket_ge_one_major_is_per_major() {
        assert_eq!(Constraint::bucket(&Version::new(1, 2, 3)), (1, 0));
        assert_eq!(Constraint::bucket(&Version::new(1, 9, 9)), (1, 0));
        assert_ne!(
            Constraint::bucket(&Version::new(1, 0, 0)),
            Constraint::bucket(&Version::new(2, 0, 0))
        );
    }

    #[test]
    fn pinned_bucket() {
        assert_eq!(Constraint::Any.pinned_bucket(), None);
        assert_eq!(
            Constraint::Caret(Version::new(1, 2, 3)).pinned_bucket(),
            Some((1, 0))
        );
        assert_eq!(
            Constraint::Exact(Version::new(0, 3, 1)).pinned_bucket(),
            Some((0, 3))
        );
    }
}
