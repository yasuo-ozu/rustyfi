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
    /// SATySFi 0.1.x: the `dev-0-1-0` language generation — an ML-style
    /// module system (F-ing Modules-based), row-polymorphic records and
    /// optional-argument encodings, and a reworked surface grammar —
    /// shared near-identically by `saphe-split` (confirmed by direct diff;
    /// see `docs/plans/satysfi-0-1-0-support.md` §1.1). This says nothing
    /// about *packaging*: `V0_1` documents may resolve dependencies via
    /// either today's `@require:`/`@import:` headers (`dev-0-1-0`'s own
    /// model, and this port's `LoadMode::Legacy` — Slice 1's target) or the
    /// `use`/manifest/lockfile model (`saphe-split`'s `LoadMode::Envelopes`,
    /// a later milestone) — see `satysfi_loader::LoadMode`. **Not yet fully
    /// implemented** by this port; see [`SatysfiVersion::is_implemented`].
    V0_1,
}

impl SatysfiVersion {
    /// The version this port targets when none is specified.
    pub const DEFAULT: Self = Self::V0_0_6;

    /// Whether this version has an ML-style module system (`module M =
    /// struct ... end` bindings that erase to stamped-flat names, `val`
    /// bindings inside them, later: signatures/functors). `false` for
    /// `V0_0_6` (which has only its own non-parameterized, single-level
    /// `module`/`sig` surface — see `docs/plans/class-signature-lang-gaps.md`
    /// — not a real module *system*); `true` for `V0_1`.
    pub fn has_module_system(&self) -> bool {
        matches!(self, Self::V0_1)
    }

    /// Whether this version's type system has row-polymorphic records and
    /// optional-argument rows (`?(l = e)` bundles, `?'r` row variables).
    /// `false` for `V0_0_6` (closed `Kind::Record` rows only); `true` for
    /// `V0_1`.
    pub fn has_row_polymorphism(&self) -> bool {
        matches!(self, Self::V0_1)
    }

    /// Whether `page-break`/`page-break-multicolumn`/`page-break-two-column`
    /// take the `page` ADT (`A4Paper`/`UserDefinedPaper`) as their paper-size
    /// argument, as opposed to `V0_1`'s plain `length * length`. Deliberately
    /// phrased as an assertion about `V0_0_6`'s surface (not "is it 0.0.6"),
    /// so a future third generation that also drops the ADT reads correctly
    /// without touching call sites — see L7 in the main plan.
    pub fn has_page_adt(&self) -> bool {
        matches!(self, Self::V0_0_6)
    }

    /// Whether the `math` type is split into `math-text` (unparsed `${...}`
    /// source) / `math-boxes` (evaluated tree) with a `read-math` primitive
    /// bridging them, as opposed to `V0_0_6`'s single unsplit `math` type.
    /// `false` for `V0_0_6`; `true` for `V0_1`.
    pub fn math_is_split(&self) -> bool {
        matches!(self, Self::V0_1)
    }

    /// Whether this port actually implements this version end-to-end
    /// (lexer through PDF rendering). Both generations, Slice 1 scope for
    /// `V0_1` — see `docs/plans/satysfi-0-1-0-support.md` §3.
    pub fn is_implemented(&self) -> bool {
        matches!(self, Self::V0_0_6 | Self::V0_1)
    }

    /// Every version this enum currently distinguishes (implemented or
    /// not), in a stable order, for building help/error text.
    pub fn all() -> &'static [SatysfiVersion] {
        &[Self::V0_0_6, Self::V0_1]
    }

    /// The subset of [`SatysfiVersion::all`] this port can actually load.
    pub fn supported() -> &'static [SatysfiVersion] {
        &[Self::V0_0_6, Self::V0_1]
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
/// any prelude bindings), and, failing that, the first content line.
///
/// - A `@stage:` header line is a real, direct signal: 0.1's lexer rejects it
///   outright, so seeing it at all yields `Some(V0_0_6)`.
/// - `@require:` / `@import:` header lines are *transparent*: byte-identical
///   in both v0.0.6 and dev-0-1-0, so their presence pins neither axis — they
///   are skipped just like a blank/comment line.
/// - A `use`-shaped header line — 0.1/Saphe's module-header syntax, see
///   [`is_use_header`] — yields `Some(V0_1)`. **This half of the heuristic is
///   best-effort**: the exact grammar could not be confirmed against
///   upstream from this sandbox (no network access to GitHub / zenn.dev at
///   the time this was written).
/// - Blank lines and `%`-comments (SATySFi's line-comment syntax, both
///   versions) are skipped while looking for the first header-shaped line.
/// - Once a non-blank, non-comment, non-header line is reached (headers are
///   only valid at the top of a file), that single line is inspected for a
///   content-level signal (see [`sniff_content_line`]) and the result —
///   including `None` — is returned regardless.
/// - Returns `None` if no signal is found at all (e.g. a bare `let ... in
///   ...` document with no headers, which is valid and version-ambiguous in
///   0.0.x).
pub fn sniff_version(src: &str) -> Option<SatysfiVersion> {
    for raw_line in src.lines() {
        let line = match raw_line.find('%') {
            Some(idx) => &raw_line[..idx],
            None => raw_line,
        };
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        // `@stage:` is a real signal: 0.1's lexer rejects it outright (the
        // header-lexing rule dropped the `"stage" -> HEADER_STAGE*` arm
        // between v0.0.6 and dev-0-1-0 — see this plan's header intro), so
        // seeing it at all means 0.0.6.
        if line.starts_with("@stage:") {
            return Some(SatysfiVersion::V0_0_6);
        }

        // `@require:`/`@import:` are *transparent*: byte-identical in both
        // v0.0.6 and dev-0-1-0 (same citation), so their presence pins
        // neither axis. Skip past them exactly like a blank/comment line —
        // do NOT return here (that was the S1 bug: this used to return
        // `Some(V0_0_6)` on the very first `@require:`, misclassifying every
        // 0.1-syntax-body-with-legacy-headers document, which is Slice 1's
        // own target shape).
        if line.starts_with("@require:") || line.starts_with("@import:") {
            continue;
        }

        if is_use_header(line) {
            return Some(SatysfiVersion::V0_1);
        }

        // First non-blank, non-comment, non-header line: headers are only
        // valid at the top of a file, so inspect this one line for a
        // content-level signal, then stop looking regardless of the result.
        return sniff_content_line(line);
    }
    None
}

/// Recognize a `use`-shaped 0.1/Saphe header line: bare `use Ident[.Ident]*`,
/// `use package ...`, `use open ...`, or `use #[attr] ...`. Best-effort
/// (Saphe's exact grammar is `saphe-split`-only and not yet ported), but
/// deliberately broader than "bare `use Ident`" per S1 — narrow enough that
/// no 0.0.6 keyword or identifier can start a line with `use ` (0.0.6 has no
/// `use` keyword at all), so widening this can only ever gain true positives,
/// never introduce a false positive against the 0.0.6 corpus.
fn is_use_header(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("use") else {
        return false;
    };
    let Some(rest) = rest.strip_prefix(|c: char| c.is_whitespace()) else {
        return false; // `used`, `user`, ... — not the `use` keyword
    };
    let rest = rest.trim_start();
    if rest.is_empty() {
        return false;
    }
    if rest.starts_with('#') {
        return true; // `use #[attr] ...`
    }
    let first_word = rest.split_whitespace().next().unwrap_or("");
    if first_word == "package" || first_word == "open" {
        return true; // `use package ...` / `use open ...`
    }
    // bare `use Ident[.Ident]*` (possibly followed by `as .../of ...`, which
    // this only requires the *first* word to look like a module path for).
    first_word
        .chars()
        .next()
        .map(|c| c.is_ascii_uppercase())
        .unwrap_or(false)
        && first_word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

/// Inspect the first non-header content line for a version signal.
/// `module <Upper> ... = struct` is deliberately **not** checked here — see
/// §1.4 step 3 of the plan: 0.0.6's `TopBinding::Module` makes the sig
/// annotation optional and all 29 shipped 0.0.6 packages open with a
/// `module` head after their headers, so a `module` head is no signal in
/// either direction and correctly falls through to `None` below.
fn sniff_content_line(line: &str) -> Option<SatysfiVersion> {
    if starts_with_word(line, "val") {
        return Some(SatysfiVersion::V0_1);
    }
    for kw in ["let-rec", "let-inline", "let-block", "let-math", "let-mutable"] {
        if starts_with_word(line, kw) {
            return Some(SatysfiVersion::V0_0_6);
        }
    }
    None
}

/// True if `line` starts with `word` followed by a word boundary (end of
/// string or non-identifier character) — so `"val"` matches `"val f = .."`
/// but not an identifier like `"values"` or `"val-like"`.
fn starts_with_word(line: &str, word: &str) -> bool {
    match line.strip_prefix(word) {
        Some(rest) => rest
            .chars()
            .next()
            .map(|c| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(true),
        None => false,
    }
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
        assert!(SatysfiVersion::V0_0_6.is_implemented());
        assert!(SatysfiVersion::V0_1.is_implemented());

        assert!(!SatysfiVersion::V0_0_6.has_module_system());
        assert!(SatysfiVersion::V0_1.has_module_system());

        assert!(!SatysfiVersion::V0_0_6.has_row_polymorphism());
        assert!(SatysfiVersion::V0_1.has_row_polymorphism());

        assert!(SatysfiVersion::V0_0_6.has_page_adt());
        assert!(!SatysfiVersion::V0_1.has_page_adt());

        assert!(!SatysfiVersion::V0_0_6.math_is_split());
        assert!(SatysfiVersion::V0_1.math_is_split());
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
    fn sniff_require_import_are_transparent_stage_still_pins() {
        // `@require:`/`@import:` no longer pin a version by themselves — with
        // no other signal on the first content line (a bare, non-hyphenated
        // `let`), the result is `None` (falls to `SatysfiVersion::DEFAULT`
        // downstream in `resolve_version`, not sniffed here). This is the S1
        // fix itself: pre-fix, all three of the first assertions below
        // returned `Some(V0_0_6)` directly at the header line.
        assert_eq!(sniff_version("@require: stdlib\nlet x = 1 in x"), None);
        assert_eq!(sniff_version("@import: helper\nlet x = 1 in x"), None);
        // Leading blank lines / comments before a transparent header must
        // still not confuse the sniffer into inventing a signal.
        assert_eq!(
            sniff_version("% a comment\n\n@require: stdlib\nlet x = 1 in x"),
            None
        );
        // `@stage:` is the one header that IS still a real, direct signal
        // (0.1's lexer rejects it outright).
        assert_eq!(
            sniff_version("@stage: 0\nlet x = 1 in x"),
            Some(SatysfiVersion::V0_0_6)
        );
    }

    #[test]
    fn sniff_require_then_module_is_none() {
        // The S1 bug case: a legacy-header-then-module-body file (Slice 1's
        // own target shape — a V0_1-syntax library reached through the
        // unmodified `@require:` loader) must sniff `None`, not `V0_0_6`
        // (the old bug) and not `V0_1` (no positive signal for it either —
        // `module` is deliberately not a signal).
        assert_eq!(
            sniff_version("@require: pervasives\nmodule V01Mini = struct\nval x = 1\nend"),
            None
        );
        assert_eq!(
            sniff_version("@import: helper\n@require: pervasives\nmodule M = struct\nend"),
            None
        );
    }

    #[test]
    fn sniff_val_head_is_v0_1() {
        assert_eq!(
            sniff_version("@require: pervasives\nval x = 1"),
            Some(SatysfiVersion::V0_1)
        );
        // No headers at all — `val` at the very first content line still
        // signals V0_1.
        assert_eq!(sniff_version("val f x = x"), Some(SatysfiVersion::V0_1));
    }

    #[test]
    fn sniff_hyphenated_let_head_is_v0_0_6() {
        for src in [
            "let-rec f x = x",
            "let-inline ctx \\emph x = x",
            "let-block ctx +p x = x",
            "let-math \\frac x y = x",
            "let-mutable r <- 0",
        ] {
            assert_eq!(sniff_version(src), Some(SatysfiVersion::V0_0_6), "src: {src:?}");
        }
    }

    #[test]
    fn sniff_use_shapes_broader_than_bare_ident() {
        for src in ["use package foo", "use open Foo", "use #[attr] Foo"] {
            assert_eq!(sniff_version(src), Some(SatysfiVersion::V0_1), "src: {src:?}");
        }
    }

    #[test]
    fn sniff_lib_satysfi_corpus_never_v0_1() {
        // Every vendored 0.0.6 package must sniff `None` or `Some(V0_0_6)`,
        // never `Some(V0_1)` — the non-regression guarantee §3's Acceptance
        // depends on.
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../lib-satysfi/dist/packages");
        let mut checked = 0usize;
        for entry in std::fs::read_dir(root).expect("lib-satysfi/dist/packages must exist") {
            let path = entry.expect("readable dir entry").path();
            // The vendored 29-package corpus uses both `.satyh` (27 of them,
            // plus this port's own `stdja-mini.satyh`) and `.satyg` (`list`,
            // `option` — 2 of them) extensions; a `.satyh`-only filter would
            // undercount the real corpus to 28 and never reach the `>= 29`
            // floor below.
            if !matches!(path.extension().and_then(|e| e.to_str()), Some("satyh") | Some("satyg")) {
                continue;
            }
            let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
            let sniffed = sniff_version(&src);
            assert_ne!(
                sniffed,
                Some(SatysfiVersion::V0_1),
                "{path:?} sniffed as V0_1 (got {sniffed:?})"
            );
            checked += 1;
        }
        assert!(checked >= 29, "expected to check the full 29-package corpus, got {checked}");
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
