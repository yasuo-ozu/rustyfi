//! Tier 2 — the buffer that lexes but does not **parse** — held to the same
//! five properties as tier 1, over both corpora and both generations.
//!
//! # The defect this file is the regression test for
//!
//! `format_cst` used to answer `Option<String>`, and a file that did not parse
//! came back as `Some(source)` from the slice-0 identity builder. That is
//! byte-for-byte what an *already formatted* file looks like, so `rustyfi fmt
//! --check` reported it clean and exited `0`: one unparseable line silenced the
//! formatter for a whole file, in CI, with a success message. `CstOutcome`
//! exists to make the two distinguishable and `CstOutcome::FellBack` is the
//! arm that says which one this was.
//!
//! # Why the properties are asserted here rather than assumed
//!
//! Tier 2's output is `crate::format`'s, and `tests/format.rs` already sweeps
//! that function over the same corpora. But it sweeps it over files that
//! **parse**, which is not the input tier 2 exists for, and it compares against
//! its own idea of the areas rather than against `format_cst`'s. What is new
//! here is the composition: `format_cst_outcome` picks the tier, derives
//! `FormatOptions` from `CstOptions`, and re-verifies. A bug in any of those
//! three joints produces output that `tests/format.rs` never looks at.
//!
//! # How an unparseable corpus file is obtained
//!
//! By appending `let unfinished =` to a real one. `File` is `headers + prelude
//! + (in body)? + EOI` (`cst.rs:29-38`), so the suffix is a top-level `let`
//! with no right-hand side in a library and trailing tokens after the body in a
//! document — a parse failure either way, in program area, that cannot change
//! how anything before it lexes. The alternative (hand-written fixtures) would
//! test the tier on ten-line files and say nothing about a 600-line one.

use std::path::{Path, PathBuf};

use rustyfi_lsp::{format_cst_outcome, CstOptions, CstOutcome, RustyfiVersion};
use rustyfi_syntax::{Atom, ParseFailureKind, Token};

// ---------------------------------------------------------------------------
// corpus discovery — the same two roots `tests/format_cst_slice1.rs` uses
// ---------------------------------------------------------------------------

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two levels below the workspace root")
        .to_path_buf()
}

fn corpus(dirs: &[&str]) -> Vec<PathBuf> {
    let root = repo_root();
    let mut out = Vec::new();
    for d in dirs {
        collect(&root.join(d), &mut out);
    }
    out.sort();
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if matches!(
            p.extension().and_then(|s| s.to_str()),
            Some("saty" | "satyh" | "satyg")
        ) {
            out.push(p);
        }
    }
}

// ---------------------------------------------------------------------------
// the properties — lifted verbatim from `tests/format_cst_slice1.rs`, which
// lifted the text/math half from `tests/format.rs`. Duplicated rather than
// shared because an integration test cannot import another integration test
// without re-running its `#[test]`s.
// ---------------------------------------------------------------------------

fn tokens(src: &str, version: RustyfiVersion) -> Option<Vec<Token>> {
    rustyfi_syntax::lex_with_version(src, version)
        .ok()
        .map(|atoms| atoms.into_iter().map(|a| a.slot).collect())
}

fn assert_same_tokens(original: &str, formatted: &str, version: RustyfiVersion, what: &str) {
    let before = tokens(original, version).expect("the input lexes");
    let after = tokens(formatted, version)
        .unwrap_or_else(|| panic!("{what}: the formatted text no longer lexes"));
    assert_eq!(before, after, "{what}: the token stream changed");
}

/// Which tokens the lexer can only have produced while reading inline text,
/// block text or math. Includes the *openers*, because `lexer.rs:562-566`
/// folds `{`'s skipped whitespace into `Token::BHorzGrp`'s own span.
fn is_text_or_math(t: &Token) -> bool {
    matches!(
        t,
        Token::Char(_)
            | Token::CodeText(_)
            | Token::Space
            | Token::Break
            | Token::Item(_)
            | Token::Sep
            | Token::MathChar(_)
            | Token::Superscript
            | Token::Subscript
            | Token::Primes(_)
            | Token::HorzCmd(_)
            | Token::HorzCmdWithMod(..)
            | Token::HorzMacro(_)
            | Token::VertCmd(_)
            | Token::VertCmdWithMod(..)
            | Token::VertMacro(_)
            | Token::MathCmd(_)
            | Token::MathCmdWithMod(..)
            | Token::VarInHorz(..)
            | Token::VarInVert(..)
            | Token::VarInMath(..)
            | Token::BHorzGrp
            | Token::EHorzGrp
            | Token::BVertGrp
            | Token::EVertGrp
            | Token::BMathGrp
            | Token::EMathGrp
    )
}

fn text_math_regions(atoms: &[Atom]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut previous_was_text = false;
    for a in atoms {
        let is_text = is_text_or_math(&a.slot);
        match (is_text, previous_was_text) {
            (true, true) => out.last_mut().expect("a run is open").1 = a.span.end.byte,
            (true, false) => out.push((a.span.start.byte, a.span.end.byte)),
            _ => {}
        }
        previous_was_text = is_text;
    }
    out
}

fn assert_text_areas_untouched(
    original: &str,
    formatted: &str,
    version: RustyfiVersion,
    what: &str,
) -> usize {
    let before = rustyfi_syntax::lex_with_version(original, version).expect("the input lexes");
    let after =
        rustyfi_syntax::lex_with_version(formatted, version).expect("the formatted text lexes");
    let (rb, ra) = (text_math_regions(&before), text_math_regions(&after));
    assert_eq!(
        rb.len(),
        ra.len(),
        "{what}: the number of text/math regions changed ({} -> {})",
        rb.len(),
        ra.len()
    );
    for (i, ((bs, be), (as_, ae))) in rb.iter().zip(&ra).enumerate() {
        assert_eq!(
            &original[*bs..*be],
            &formatted[*as_..*ae],
            "{what}: text/math region {i} was rewritten"
        );
    }
    rb.len()
}

fn content_lines(s: &str) -> Vec<&str> {
    s.lines().filter(|l| !l.trim().is_empty()).collect()
}

fn squeeze(line: &str) -> String {
    line.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The five properties over one tier-2 buffer. Returns the number of
/// text/math regions compared, so the sweep can prove it looked at something.
///
/// Property 5 (`cst_walk_desync`) is vacuous here and deliberately absent: it
/// measures the CST walk against the atom stream, and tier 2 runs no walk. Its
/// stand-in is the tier assertion itself — this buffer reached tier 2 and not
/// tier 1, which is exactly what "there was no walk" means.
fn check_tier2(src: &str, version: RustyfiVersion, what: &str) -> usize {
    let opts = CstOptions::default();
    let out = format_cst_outcome(src, version, &opts);
    let CstOutcome::FellBack { text, error, .. } = &out else {
        panic!(
            "{what}: expected tier 2 (lexes, does not parse), got {:?}",
            std::mem::discriminant(&out)
        );
    };
    assert_ne!(
        error.kind,
        ParseFailureKind::Lex,
        "{what}: a lex failure must be tier 0, not tier 2"
    );

    // 1, 2.
    assert_same_tokens(src, text, version, what);
    let regions = assert_text_areas_untouched(src, text, version, what);

    // 3. Idempotence. The second pass is tier 2 as well — the appended suffix
    // is still there — so this is `crate::format`'s own fixpoint reached
    // through `format_cst_outcome`'s option derivation.
    let twice = format_cst_outcome(text, version, &opts);
    assert_eq!(
        twice.text(),
        Some(text.as_str()),
        "{what}: tier 2 is not idempotent"
    );

    // 4. No non-blank content change.
    let (a, b) = (content_lines(src), content_lines(text));
    assert_eq!(
        a.len(),
        b.len(),
        "{what}: tier 2 changed the number of content-bearing lines ({} -> {})",
        a.len(),
        b.len()
    );
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        assert_eq!(
            squeeze(x),
            squeeze(y),
            "{what}: content line {i} changed by more than its whitespace"
        );
    }
    regions
}

/// The suffix that makes any corpus file fail to parse without changing how
/// one byte of it lexes.
const BREAK: &str = "\nlet unfinished =\n";

fn sweep(dirs: &[&str], version: RustyfiVersion, label: &str) {
    let files = corpus(dirs);
    assert!(
        files.len() > 20,
        "expected the bundled corpus, found {} files — is the checkout complete?",
        files.len()
    );
    let (mut checked, mut regions) = (0usize, 0usize);
    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        let broken = format!("{src}{BREAK}");
        let what = path.display().to_string();
        regions += check_tier2(&broken, version, &what);
        checked += 1;
    }
    eprintln!("tier 2, {label}: {checked} files checked, {regions} text/math regions compared");
    assert!(checked > 20, "only {checked} files were checked");
}

#[test]
fn tier_two_holds_the_five_properties_over_the_v006_corpus() {
    sweep(
        &["lib-rustyfi/dist/packages", "layout-tests/corpus"],
        RustyfiVersion::V0_0,
        "0.0.6 corpus",
    );
}

#[test]
fn tier_two_holds_the_five_properties_over_the_v01_corpus() {
    sweep(
        &["lib-rustyfi/dist-v01/packages"],
        RustyfiVersion::V0_1,
        "0.1 corpus",
    );
}

/// Tier 2 is not the identity — it really does normalise.
///
/// The sweep above would pass with `fall_back` returning `source` verbatim,
/// because every property it checks is reflexive. This is the non-vacuity
/// half, and it is asserted per file rather than in aggregate: EVERY corpus
/// file gains the appended suffix's own trailing blank line and must lose it.
#[test]
fn tier_two_changes_something_on_every_corpus_file() {
    for (dirs, version) in [
        (
            vec!["lib-rustyfi/dist/packages", "layout-tests/corpus"],
            RustyfiVersion::V0_0,
        ),
        (vec!["lib-rustyfi/dist-v01/packages"], RustyfiVersion::V0_1),
    ] {
        for path in corpus(&dirs) {
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            // Trailing whitespace on the last line, which `crate::format`
            // trims and the identity builder never did.
            let broken = format!("{src}{BREAK}   \n");
            let out = format_cst_outcome(&broken, version, &CstOptions::default());
            assert!(
                matches!(out, CstOutcome::FellBack { changed: true, .. }),
                "{}: tier 2 left an untidy unparseable buffer alone",
                path.display()
            );
        }
    }
}

/// **Acceptance measurement.** How many corpus files fail to parse as they
/// stand, and were therefore silently inert under the old identity fallback.
///
/// Printed and asserted to be zero. Zero is the answer that matters: it says
/// the defect was never reachable from the bundled corpus, so no corpus sweep
/// could have caught it and the CLI is the only place it could ever have been
/// noticed. If this ever fails, the number in the commit message is stale and
/// the file it names needs looking at.
#[test]
fn no_corpus_file_fails_to_parse_today() {
    let mut inert = Vec::new();
    let mut checked = 0;
    for (dirs, version) in [
        (
            vec!["lib-rustyfi/dist/packages", "layout-tests/corpus"],
            RustyfiVersion::V0_0,
        ),
        (vec!["lib-rustyfi/dist-v01/packages"], RustyfiVersion::V0_1),
    ] {
        for path in corpus(&dirs) {
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            checked += 1;
            let out = format_cst_outcome(&src, version, &CstOptions::default());
            if !out.parsed() {
                inert.push(format!("{}: {:?}", path.display(), out.parse_error()));
            }
        }
    }
    eprintln!(
        "corpus parse survey: {checked} files, {} of them do not parse",
        inert.len()
    );
    assert!(
        inert.is_empty(),
        "{} corpus file(s) do not parse:\n{}",
        inert.len(),
        inert.join("\n")
    );
}
