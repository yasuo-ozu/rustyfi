//! Target SATySFi language version.
//!
//! This port implements **0.0.6** semantics only. SATySFi 0.1.x overhauled
//! the surface language (an ML-style module system replacing `@require:` /
//! `@import:` headers with `use <Ident>` headers, among other changes), so a
//! future port of that version will diverge from this one at several points
//! in the pipeline (lexing headers, module resolution in the loader,
//! elaboration in `satysfi-lang`, ...).
//!
//! Rather than let each of those future divergence points grow its own
//! ad-hoc flag, the target version is threaded through the pipeline *now*
//! (see [`crate`]'s consumers: `satysfi-loader`'s `LoadOptions` and
//! `satysfi-cli`'s `--target-version` flag) as a single [`SatysfiVersion`]
//! value, and each divergence point is expressed as a method on it. A future
//! 0.1 implementation flips those methods (and adds new match arms) in one
//! place instead of scattering `if opt_a && opt_b` checks across the crate
//! graph.
//!
//! ## Verification note on the 0.1 header syntax
//!
//! This port's sandbox could not reach the network (GitHub, raw.
//! githubusercontent.com, and zenn.dev all rejected the fetch as an
//! unverified domain), so the exact SATySFi 0.1.0 `use` header grammar is
//! **not independently confirmed** from upstream sources here. The
//! `sniff_version` heuristic below for 0.1 (`use `-prefixed header lines) is
//! therefore marked best-effort; the 0.0.6 side (`@require:` / `@import:` /
//! `@stage:`) *is* verified, directly against this port's own
//! [`crate::cst`]/[`crate::lexer`] implementation of the v0.0.6 grammar.

use std::fmt;
use std::str::FromStr;

/// Target SATySFi language version.
///
/// `#[non_exhaustive]` because more 0.1.z-era (and later) variants are
/// expected as the upstream module system design settles; treat any `match`
/// on this type as needing a wildcard arm.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SatysfiVersion {
    /// SATySFi 0.0.6 (and, as far as this port is concerned, the rest of the
    /// 0.0.x series): `@require:` / `@import:` headers, the module-less
    /// surface syntax this port's `satysfi-syntax` / `satysfi-loader` /
    /// `satysfi-lang` implement.
    V0_0_6,
    /// SATySFi 0.1.x: `use <Ident>` headers and the reworked (F-ing
    /// Modules-based) module system. **Not implemented** by this port; see
    /// [`SatysfiVersion::is_implemented`].
    V0_1,
}

impl SatysfiVersion {
    /// The version this port targets when none is specified.
    pub const DEFAULT: Self = Self::V0_0_6;

    /// Whether this version's surface syntax resolves multi-file programs
    /// via `@require: name` / `@import: name` header lines (as opposed to
    /// 0.1's `use Name` module headers).
    ///
    /// This is the loader's divergence point: `satysfi_loader::load` reads
    /// this to decide which header form it should be parsing / resolving.
    pub fn uses_require_headers(&self) -> bool {
        match self {
            Self::V0_0_6 => true,
            Self::V0_1 => false,
        }
    }

    /// Whether this port actually implements this version end-to-end
    /// (lexer through PDF rendering). Only [`SatysfiVersion::V0_0_6`] is
    /// implemented today.
    pub fn is_implemented(&self) -> bool {
        matches!(self, Self::V0_0_6)
    }

    /// Every version this enum currently distinguishes (implemented or
    /// not), in a stable order, for building help/error text.
    pub fn all() -> &'static [SatysfiVersion] {
        &[Self::V0_0_6, Self::V0_1]
    }

    /// The subset of [`SatysfiVersion::all`] this port can actually load.
    pub fn supported() -> &'static [SatysfiVersion] {
        &[Self::V0_0_6]
    }
}

impl Default for SatysfiVersion {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl fmt::Display for SatysfiVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::V0_0_6 => write!(f, "0.0.6"),
            Self::V0_1 => write!(f, "0.1"),
        }
    }
}

/// Error returned by `SatysfiVersion`'s [`FromStr`] impl for a string that
/// does not name a recognized version.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "unrecognized SATySFi version {input:?}; supported values: \
     0.0.6 (aliases: 0.0, v0.0.6), 0.1 (aliases: 0.1.x, v0.1, v0.1.0; not yet implemented)"
)]
pub struct ParseVersionError {
    /// The string that failed to parse.
    pub input: String,
}

impl FromStr for SatysfiVersion {
    type Err = ParseVersionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim();
        let normalized = normalized
            .strip_prefix('v')
            .or_else(|| normalized.strip_prefix('V'))
            .unwrap_or(normalized);
        match normalized {
            "0.0.6" | "0.0" => Ok(Self::V0_0_6),
            "0.1" | "0.1.x" | "0.1.0" => Ok(Self::V0_1),
            _ => Err(ParseVersionError {
                input: s.to_string(),
            }),
        }
    }
}

/// Best-effort detection of a document's target version from its source
/// text, by inspecting the header-like lines at the top of the file (before
/// any prelude bindings).
///
/// - A `@require:` / `@import:` / `@stage:` header line (0.0.x's header
///   syntax; verified against this port's own lexer/parser) yields
///   `Some(V0_0_6)`.
/// - A `use <Ident>` header line — 0.1's replacement module-header syntax —
///   yields `Some(V0_1)`. **This half of the heuristic is best-effort**: the
///   exact 0.1.0 grammar could not be confirmed against upstream from this
///   sandbox (no network access to GitHub / zenn.dev at the time this was
///   written), so this only recognizes the widely-referenced `use Name`
///   shape and may both over- and under-match real 0.1 documents.
/// - Blank lines and `%`-comments (SATySFi's line-comment syntax, both
///   versions) are skipped while looking for the first header-shaped line.
/// - Returns `None` if no header-shaped line is found before other content
///   (e.g. a bare `let ... in ...` document with no headers at all, which
///   is valid and version-ambiguous in 0.0.x).
pub fn sniff_version(src: &str) -> Option<SatysfiVersion> {
    for raw_line in src.lines() {
        // Strip a trailing `%`-comment (SATySFi's only comment form) before
        // inspecting the line, then trim whitespace.
        let line = match raw_line.find('%') {
            Some(idx) => &raw_line[..idx],
            None => raw_line,
        };
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        if line.starts_with("@require:") || line.starts_with("@import:") || line.starts_with("@stage:")
        {
            return Some(SatysfiVersion::V0_0_6);
        }

        if let Some(rest) = line.strip_prefix("use ") {
            let name = rest.trim();
            let is_ident_like = !name.is_empty()
                && name
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_uppercase())
                    .unwrap_or(false)
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
            if is_ident_like {
                return Some(SatysfiVersion::V0_1);
            }
        }

        // First non-blank, non-comment, non-header line: no more headers can
        // follow (headers are only valid at the top of a file), so stop
        // looking.
        return None;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_accepts_0_0_6_forms() {
        for s in ["0.0.6", "0.0", "v0.0.6", "V0.0.6"] {
            assert_eq!(
                s.parse::<SatysfiVersion>().unwrap_or_else(|e| panic!("{s:?}: {e}")),
                SatysfiVersion::V0_0_6,
                "input {s:?}"
            );
        }
    }

    #[test]
    fn from_str_accepts_0_1_forms() {
        for s in ["0.1", "0.1.x", "0.1.0", "v0.1"] {
            assert_eq!(
                s.parse::<SatysfiVersion>().unwrap_or_else(|e| panic!("{s:?}: {e}")),
                SatysfiVersion::V0_1,
                "input {s:?}"
            );
        }
    }

    #[test]
    fn from_str_rejects_unknown_forms() {
        for s in ["", "1.0", "0.0.7", "garbage", "0.2"] {
            let err = s.parse::<SatysfiVersion>().unwrap_err();
            assert_eq!(err.input, s);
            let msg = err.to_string();
            assert!(msg.contains("0.0.6"), "message should list supported values: {msg}");
            assert!(msg.contains("0.1"), "message should list supported values: {msg}");
        }
    }

    #[test]
    fn default_is_v0_0_6() {
        assert_eq!(SatysfiVersion::DEFAULT, SatysfiVersion::V0_0_6);
        assert_eq!(SatysfiVersion::default(), SatysfiVersion::V0_0_6);
    }

    #[test]
    fn capability_probes() {
        assert!(SatysfiVersion::V0_0_6.uses_require_headers());
        assert!(SatysfiVersion::V0_0_6.is_implemented());
        assert!(!SatysfiVersion::V0_1.uses_require_headers());
        assert!(!SatysfiVersion::V0_1.is_implemented());
    }

    #[test]
    fn display_round_trips_through_from_str() {
        for v in SatysfiVersion::all() {
            let s = v.to_string();
            assert_eq!(&s.parse::<SatysfiVersion>().unwrap(), v, "round-trip of {s:?}");
        }
    }

    #[test]
    fn sniff_none_for_headerless_document() {
        assert_eq!(sniff_version("let x = 1 in x"), None);
        assert_eq!(sniff_version(""), None);
        assert_eq!(sniff_version("   \n% just a comment\n"), None);
    }

    #[test]
    fn sniff_v0_0_6_on_require_and_import_and_stage() {
        assert_eq!(
            sniff_version("@require: stdlib\nlet x = 1 in x"),
            Some(SatysfiVersion::V0_0_6)
        );
        assert_eq!(
            sniff_version("@import: helper\nlet x = 1 in x"),
            Some(SatysfiVersion::V0_0_6)
        );
        assert_eq!(
            sniff_version("@stage: 0\nlet x = 1 in x"),
            Some(SatysfiVersion::V0_0_6)
        );
        // Leading blank lines / comments before the header must not
        // confuse the sniffer.
        assert_eq!(
            sniff_version("% a comment\n\n@require: stdlib\nlet x = 1 in x"),
            Some(SatysfiVersion::V0_0_6)
        );
    }

    #[test]
    fn sniff_v0_0_6_fixtures_are_never_mistaken_for_v0_1() {
        // A representative sample of this port's own 0.0.6 fixtures/tests
        // must not sniff as V0_1.
        for src in [
            "document (|title = {Hello};|) '<+p{Hello, world!}>",
            "@import: helper\nlet x = 1 in x",
            "@require: stdlib\nlet x = 1 in x",
            "let x = 1",
        ] {
            assert_ne!(sniff_version(src), Some(SatysfiVersion::V0_1), "src: {src:?}");
        }
    }

    #[test]
    fn sniff_best_effort_v0_1_use_header() {
        assert_eq!(
            sniff_version("use Foo\nlet x = 1 in x"),
            Some(SatysfiVersion::V0_1)
        );
    }
}
