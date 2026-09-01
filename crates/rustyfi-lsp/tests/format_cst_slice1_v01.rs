//! Slice 1's acceptance criteria for the **0.1** grammar, plus the two
//! universal normalisations that now hold for *both* generations.
//!
//! The 0.0.6 twin is `tests/format_cst_slice1.rs` and the five properties are
//! the same five, restated here because a test binary cannot share code with
//! another one:
//!
//! 1. **Token-stream identity.** The formatted text lexes to the same
//!    `Vec<Token>`. `format_cst`'s always-on verifier already checks this per
//!    call and DECLINES on failure, so this sweep also asserts nothing declined
//!    — a decline is exactly how a broken printer would hide inside a
//!    token-identity sweep.
//! 2. **Every text/math token's own bytes are identical**, token by token —
//!    per TOKEN rather than per region since block text and math got real
//!    layout, because the region form measured the gaps between the tokens
//!    too and those gaps are what the policy frees. The ONE relaxation is
//!    inline text's — see [`swallows_trivia`].
//! 3. **Idempotence**: formatting the output changes nothing.
//! 4. **No non-blank line is added, removed or merged, and no token moved
//!    between two of them.**
//! 5. **The walk stays in step with the atom stream** (`cst_walk_desync`).
//!    Nothing else here could notice a drift: it misattributes indentation, and
//!    every property above holds either way.
//!
//! # What moved here, and from where
//!
//! `tests/format_cst_identity.rs` asserted `format_cst(src) == Some(src)` over
//! the whole 47-file 0.1 corpus, on the ground that 0.1 had no builder and took
//! the identity path. `build01.rs` makes that false by design. The claim was
//! **moved, not weakened**: what is still absolutely checkable — the five
//! properties, over the same 47 files — is asserted below, and the shapes whose
//! bytes genuinely change are pinned with their expected output and a reason in
//! [`the_awkward_shapes_that_change_now_that_both_generations_are_normalised`].
//!
//! **The anti-vacuity check is NOT here.** It lives in `build01.rs`'s
//! string-literal unit tests, and that is a lesson from the 0.0.6 side rather
//! than a preference: its sweep used "the corpus must change" as the proof that
//! re-indentation happened at all, and re-formatting the corpus destroyed that
//! fixture without the assertion noticing it had become a tautology. The 0.1
//! corpus is already laid out the way this slice lays it out, so `changed` here
//! is *near zero and that is the healthy state*; a mutation to the indentation
//! rule is killed by the unit tests, which own input this sweep does not.

use std::path::{Path, PathBuf};

use rustyfi_lsp::{cst_walk_desync, format_cst, CstOptions, RustyfiVersion};
use rustyfi_syntax::token::Token;

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
// the properties
// ---------------------------------------------------------------------------

fn tokens(src: &str, version: RustyfiVersion) -> Option<Vec<Token>> {
    rustyfi_syntax::lex_with_version(src, version)
        .ok()
        .map(|atoms| atoms.into_iter().map(|a| a.slot).collect())
}

/// Is `c` CJK by **this port's own range classifier**?
///
/// An INDEPENDENT transcription of `rustyfi-backend/src/font.rs:83-92`'s
/// `char_script` — the arms that answer `Kana` or `HanIdeographic` — written
/// here rather than imported from `rustyfi_lsp`. A sweep that called the
/// library's own copy would accept whatever the library believes; two
/// transcriptions of one upstream table disagree when either drifts, and
/// `crates/rustyfi/tests/ws_inline_rewrap.rs` pins the library's copy against
/// `char_script` itself.
///
/// Note what is in here and is neither Han nor Kana: U+3000-303F (`。` `、`
/// `「` `々` `・` and the ideographic SPACE) and U+FF00-FFEF (`Ａ` U+FF21,
/// whose Unicode Script is Latin). Each was measured at 3.96 pt of displaced
/// ink. Hangul, Thai, Lao, Greek, Cyrillic and emoji are NOT here: this port
/// routes them through `OtherScript` and they are measurably safe.
fn cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x3000..=0x303F
            | 0x3040..=0x30FF
            | 0x31F0..=0x31FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0xFF00..=0xFFEF
            | 0x20000..=0x2FA1F
    )
}

/// May the whitespace token at `i` be re-spelled — a `Space` written as a
/// `Break`, or the other way round?
///
/// **Property 1's one relaxation, and the whole of it.** A whitespace run
/// inside `{ … }` is one token whose identity is fixed by its FIRST character
/// (`lexer.rs:1149-1155`), so slice 6's re-wrap changes exactly this slot and
/// nothing else. The measured licence
/// (`docs/plans/formatter-cst/README.md` rule 3; 123 fixture pairs, 801
/// in-process compiles, 221 vacuity probes, 0 vacuous) is:
///
/// > a break may be inserted OR removed at a gap iff NOT (the codepoint
/// > immediately before it is CJK AND the codepoint immediately after it is
/// > CJK)
///
/// "Immediately" is *within the same elaborated text run*, which is exactly
/// "the neighbouring token is a `Token::Char`" — a command, `${…}`, `#var;`,
/// a backtick literal and a group edge are all other tokens and all count as
/// non-CJK. The neighbour's **payload** is read rather than its source bytes,
/// because `\&` lexes to `Char("&")` and an escaped space to `Char(" ")` and
/// the payload is what the typesetter sees.
///
/// Everything else property 1 ever asserted is unchanged: the token COUNT, and
/// every other slot including every `Char` payload. So a run that stopped
/// existing, a run that was invented, a `%` comment that vanished from inside
/// a run and any edit outside inline text all still fail here.
fn gap_may_be_respelled(toks: &[Token], i: usize) -> bool {
    let edge = |t: Option<&Token>, last: bool| match t {
        Some(Token::Char(s)) => match last {
            true => s.chars().next_back(),
            false => s.chars().next(),
        },
        _ => None,
    };
    let before = edge(i.checked_sub(1).and_then(|j| toks.get(j)), true);
    let after = edge(toks.get(i + 1), false);
    !(before.is_some_and(cjk) && after.is_some_and(cjk))
}

/// Property 1, with slice 6's relaxation. Returns how many gaps were
/// re-spelled, so the sweep can say the feature fired.
fn assert_same_tokens(
    original: &str,
    formatted: &str,
    version: RustyfiVersion,
    what: &str,
) -> usize {
    let before = tokens(original, version).expect("the input lexes");
    let after = tokens(formatted, version)
        .unwrap_or_else(|| panic!("{what}: the formatted text no longer lexes"));
    assert_eq!(
        before.len(),
        after.len(),
        "{what}: the token COUNT changed ({} -> {}) — a run was invented or emptied",
        before.len(),
        after.len()
    );
    let mut respelled = 0usize;
    for (at, (a, b)) in before.iter().zip(&after).enumerate() {
        if a == b {
            continue;
        }
        let licensed = matches!(
            (a, b),
            (Token::Space, Token::Break) | (Token::Break, Token::Space)
        ) && gap_may_be_respelled(&before, at);
        assert!(
            licensed,
            "{what}: token {at} changed\n  before: {:?}\n   after: {:?}\n  \
             (the ONLY licensed difference is `Space` <-> `Break` at a gap the \
             re-wrap predicate clears; see `gap_may_be_respelled`)",
            a, b
        );
        respelled += 1;
    }
    respelled
}

/// Which tokens the lexer can only have produced while reading inline text,
/// block text or math.
///
/// Lifted from `tests/format.rs`, deliberately including the *openers*:
/// `lexer.rs:562-566` gives `Token::BHorzGrp` a span covering the `{` **and the
/// whitespace it then skipped**, so the leading spaces of `{  hello }` live
/// inside that token.
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

/// The seven tokens whose own span holds trivia the lexer had already decided
/// to discard — **the whole of slice 4's inline-text relaxation, and nothing
/// wider**.
///
/// Everywhere else, whitespace inside a text or math area lives in a **gap**:
/// `lex_vertical` (`lexer.rs:1029-1032`), `lex_math` (`:1338-1340`) and
/// `lex_active` (`:1241-1243`) each skip it without emitting, which is why
/// block text and math needed no exception at all. Horizontal mode is the
/// exception and these are its whole surface — the `(break|space)*
/// <terminator>` family swallows the run in front of `}`, `{`, `<`, `|` and an
/// item bullet (`:1112-1147`), a `{` calls `skip_spaces()` and swallows what
/// follows it (`:562-567`), and any other run becomes one whole `Space`/`Break`
/// (`:1149-1155`).
///
/// So these seven are compared **modulo whitespace** and every other text/math
/// token byte for byte. What that leaves unchecked is exactly what the policy
/// claims it may do, and no more: the two things it must NOT do are checked
/// elsewhere and more sharply. That a run keeps its newline if it had one is
/// `Space` versus `Break`, a SLOT difference, so property 1 catches it; that a
/// run is never emptied is a token that stops existing, so property 1 catches
/// that too. What this comparison adds is that nothing which is not whitespace
/// went missing — a `%` comment, most of all.
fn swallows_trivia(t: &Token) -> bool {
    matches!(
        t,
        Token::Space
            | Token::Break
            | Token::BHorzGrp
            | Token::EHorzGrp
            | Token::Item(_)
            | Token::Sep
            | Token::BVertGrp
    )
}

/// `s` with every whitespace character removed.
fn strip_ws(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// **Property 2, per TOKEN rather than per region.**
///
/// The region form this replaced took each maximal run of text/math tokens,
/// extended it over the *gaps between them*, and asserted the whole range was
/// byte-identical. That is the right property while all three areas are one
/// `Doc::Verbatim` — and it is exactly the wrong shape for freeing one area at
/// a time, because a `'< +p{hi} >` is ONE such run: the block delimiters, the
/// block command and the inline text inside it are all text tokens, so there is
/// no way to free the block area's gaps while holding the inline text's bytes.
///
/// Per token there is. Block text and math re-indent by moving bytes that are
/// in **gaps** — `lex_vertical` (`lexer.rs:1029-1032`) and `lex_math`
/// (`:1338-1340`) emit no token for whitespace or comments — so every token in
/// them stays byte-identical and this function is not weakened at all for
/// those two areas; the region form failed only because it was measuring the
/// gaps as well. Inline text is the one area whose whitespace IS a token, and
/// [`swallows_trivia`] names the seven that carry it.
///
/// Returns how many text/math tokens were compared.
fn assert_text_areas_hold_their_policy(
    original: &str,
    formatted: &str,
    version: RustyfiVersion,
    what: &str,
) -> usize {
    let before = rustyfi_syntax::lex_with_version(original, version).expect("the input lexes");
    let after =
        rustyfi_syntax::lex_with_version(formatted, version).expect("the formatted text lexes");
    // Property 1 has already said the slots match in order, so a length
    // difference here would be that failure showing up twice.
    assert_eq!(
        before.len(),
        after.len(),
        "{what}: the token count changed ({} -> {})",
        before.len(),
        after.len()
    );
    let mut compared = 0usize;
    for (i, (b, a)) in before.iter().zip(&after).enumerate() {
        if !is_text_or_math(&b.slot) {
            continue;
        }
        compared += 1;
        let bt = &original[b.span.start.byte..b.span.end.byte];
        let at = &formatted[a.span.start.byte..a.span.end.byte];
        match swallows_trivia(&b.slot) {
            false => assert_eq!(
                bt, at,
                "{what}: text/math token {i} ({:?}) was rewritten",
                b.slot
            ),
            true => assert_eq!(
                strip_ws(bt),
                strip_ws(at),
                "{what}: text/math token {i} ({:?}) lost or gained something \
                 that is not whitespace — a `%` comment, most likely",
                b.slot
            ),
        }
    }
    compared
}

/// The lines that carry something, in order.
///
/// Blank lines are excluded because the blank-line cap really does remove some
/// — and a line of nothing but spaces counts as blank on both sides, which is
/// trailing-whitespace trimming rather than a lost line.
fn content_lines(s: &str) -> Vec<&str> {
    s.lines().filter(|l| !l.trim().is_empty()).collect()
}

/// A content line with every whitespace character removed.
///
/// Slice 1 can only change a line's *leading* whitespace, so `trim()` would be
/// the exact statement — but this file is written to survive slice 2's arrival
/// on the 0.1 side, and property 1 does not subsume the weaker form: the token
/// stream is a *global* sequence, so a bug that moved a token from the end of
/// one line to the start of the next leaves it identical. This is what says
/// every token stayed on the line the author put it on.
fn squeeze(line: &str) -> String {
    line.chars().filter(|c| !c.is_whitespace()).collect()
}

/// [`CstOptions`] with slice 6 on and comment reflow off.
///
/// Property 4a's configuration: the inline re-wrap moves whitespace and
/// nothing else, so `squeeze` is exactly invariant under it, while comment
/// reflow re-emits a `%` marker per invented line and is not.
fn inline_wrap_only() -> CstOptions {
    CstOptions {
        wrap_comments: false,
        wrap_inline_text: true,
        ..CstOptions::default()
    }
}

/// [`CstOptions`] with both features that may move a token to another LINE
/// turned off: comment reflow and slice 6's inline re-wrap.
///
/// Property 4 used to be one claim — "no content line was added, removed or
/// merged, and no token moved between two of them" — and it was exact while
/// every rule was confined to one line. Both wrapping features break it BY
/// DESIGN, and the honest response is not to weaken the claim to something
/// both configurations satisfy: it is to keep the strong claim where it is
/// still true and add a weaker one that holds everywhere.
///
/// So property 4 is now 4a (global: `squeeze` of the whole file is unchanged,
/// asserted against the live default, so no non-whitespace byte is added,
/// removed or reordered by anything) and 4b (per line, asserted against
/// THIS option set, at exactly its old strength). Neither replaces the other:
/// 4a cannot see a token migrating between two lines and 4b cannot see the
/// wrapped configuration at all.
fn no_wrap() -> CstOptions {
    CstOptions {
        wrap_comments: false,
        wrap_inline_text: false,
        ..CstOptions::default()
    }
}

/// One file, all five properties. Returns (changed, content lines differing,
/// content lines, text/math tokens compared, inline gaps RE-SPELLED by
/// slice 6 — the last of which is what says the re-wrap fired at all).
fn check(src: &str, version: RustyfiVersion, what: &str) -> (bool, usize, usize, usize, usize) {
    let opts = CstOptions::default();
    let once = format_cst(src, version, &opts).unwrap_or_else(|| {
        panic!(
            "{what}: DECLINED a buffer that lexes. That is not a pass — the \
             verifier rejects the output by returning `None`, so a decline is \
             exactly how a broken printer would hide inside a token-identity \
             sweep."
        )
    });

    // 1, 2.
    let respelled = assert_same_tokens(src, &once, version, what);
    let regions = assert_text_areas_hold_their_policy(src, &once, version, what);

    // 3.
    let twice = format_cst(&once, version, &opts)
        .unwrap_or_else(|| panic!("{what}: declined its own output on the second pass"));
    if twice != once {
        let at = once
            .bytes()
            .zip(twice.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| once.len().min(twice.len()));
        let lo = at.saturating_sub(60);
        panic!(
            "{what}: not idempotent — the second pass differs at byte {at}\n\
             once : {:?}\n\
             twice: {:?}",
            &once[lo..(at + 60).min(once.len())],
            &twice[lo..(at + 60).min(twice.len())],
        );
    }

    // 4, in two halves — see `squeeze`'s doc comment for why one property
    // became two when slice 6 landed.
    //
    // 4a, with the INLINE re-wrap on: every non-whitespace byte of the file,
    // in order, is unchanged. Nothing was added, removed or reordered. Held
    // to the inline feature alone rather than to the live default because
    // comment reflow re-emits the `%` MARKER on every line it invents, so
    // `squeeze` legitimately gains a `%` — that feature's content property is
    // exact and lives in `format_cst_comment_wrap.rs`'s `assert_reflow_only`,
    // which reconstructs each comment's marker and body rather than squeezing
    // them.
    let inline_only = format_cst(src, version, &inline_wrap_only())
        .unwrap_or_else(|| panic!("{what}: DECLINED with the inline re-wrap alone"));
    assert_eq!(
        squeeze(src),
        squeeze(&inline_only),
        "{what}: the inline re-wrap added, removed or moved a non-whitespace byte"
    );
    // 4b, against a run with the two WRAPPING features off. **Slice 3 retired
    // the per-line half of this claim, and it was not a weakening — it was the
    // claim becoming false on purpose.** Under slices 1 and 2 the output's line
    // structure was the author's, so "no content line was added, removed or
    // merged" was exact; deciding line breaks IS adding, removing and merging
    // content lines, and asserting otherwise would assert the feature does not
    // work. What survives is the part that is still true and still catches the
    // bug the per-line form was there for: no non-whitespace byte is added,
    // removed or reordered in THIS configuration either.
    let unwrapped = format_cst(src, version, &no_wrap()).unwrap_or_else(|| {
        panic!("{what}: DECLINED with both wrapping features off")
    });
    assert_eq!(
        squeeze(src),
        squeeze(&unwrapped),
        "{what}: laying out the line structure added, removed or moved a \
         non-whitespace byte"
    );
    let (a, b) = (content_lines(src), content_lines(&unwrapped));
    let differing = a.len().abs_diff(b.len());

    // 6. **The budget is a live input, so it is swept.** Everything above runs
    // at the default 100 columns, where a great many groups fit and are never
    // asked to break. Sixty forces almost every group open and two hundred
    // forces almost every one flat; both have to survive the verifier and both
    // have to be fixpoints.
    for width in [60usize, 200] {
        let opts = CstOptions { max_width: width, ..CstOptions::default() };
        let once = format_cst(src, version, &opts)
            .unwrap_or_else(|| panic!("{what}: DECLINED at max_width {width}"));
        assert_same_tokens(src, &once, version, what);
        let twice = format_cst(&once, version, &opts)
            .unwrap_or_else(|| panic!("{what}: declined its own output at max_width {width}"));
        assert_eq!(twice, once, "{what}: not idempotent at max_width {width}");
    }

    // 5.
    let desync = cst_walk_desync(src, version, &opts).unwrap_or_else(|| {
        panic!("{what}: the walk could not be run at all, so property 5 is vacuous")
    });
    assert_eq!(
        desync, 0,
        "{what}: the CST walk drifted out of step with the atom stream \
         {desync} time(s), so indentation is attributed to the wrong depth. \
         Nothing else in this file can see that."
    );

    (once != src, differing, a.len(), regions, respelled)
}

// ---------------------------------------------------------------------------
// the sweep
// ---------------------------------------------------------------------------

#[test]
fn slice_1_holds_all_five_properties_over_the_v01_corpus() {
    let files = corpus(&["lib-rustyfi/dist-v01/packages"]);
    assert!(
        files.len() > 20,
        "expected the bundled 0.1 corpus, found {} files — is the checkout complete?",
        files.len()
    );
    let (mut checked, mut changed, mut lines_changed, mut lines, mut regions) = (0, 0, 0, 0, 0);
    let mut respelled = 0usize;
    let mut movers: Vec<String> = Vec::new();
    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        let what = path.display().to_string();
        let (did_change, differing, total, r, resp) = check(&src, RustyfiVersion::V0_1, &what);
        respelled += resp;
        checked += 1;
        changed += usize::from(did_change);
        if did_change {
            movers.push(format!("{what} ({differing} lines)"));
        }
        lines_changed += differing;
        lines += total;
        regions += r;
    }
    eprintln!(
        "slice 1, 0.1 corpus: {checked} files checked, {changed} changed, \
         {lines_changed} of {lines} content lines re-indented, \
         {regions} text/math tokens compared against their area's policy, \
         {respelled} inline gaps re-spelled by slice 6"
    );
    for m in &movers {
        eprintln!("  changed: {m}");
    }
    assert!(
        checked > 20,
        "only {checked} of {} files reached the comparison — this sweep has gone vacuous",
        files.len()
    );
    // No `changed > 0` here, deliberately: see the module header. The 0.1 corpus
    // is hand-written in the layout this slice computes, so a near-zero number
    // is the healthy state and the proof that re-indentation happens at all
    // lives in `build01.rs`'s unit tests, on input this sweep does not own.
    assert!(
        regions > 100,
        "only {regions} text/math tokens were compared, so property 2 is close \
         to vacuous"
    );
    assert!(
        lines > 3000,
        "only {lines} content lines were compared, so this sweep is thinner than \
         the corpus it claims to cover"
    );
}

/// Every 0.1 corpus file must reach the **builder**, not the identity path.
///
/// The sweep above cannot tell the two apart: a file the builder declines comes
/// back byte-identical, which is also what a correctly-laid-out file looks like.
/// `cst_walk_desync` answers `None` exactly when the parse failed, so this is
/// the check that says the parse succeeded on all 47.
#[test]
fn every_v01_corpus_file_parses_and_is_laid_out_by_the_builder() {
    let files = corpus(&["lib-rustyfi/dist-v01/packages"]);
    let opts = CstOptions::default();
    let mut parsed = 0;
    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        assert!(
            cst_walk_desync(&src, RustyfiVersion::V0_1, &opts).is_some(),
            "{}: does not parse, so it silently took the identity path and \
             nothing in this file measured the 0.1 builder on it",
            path.display()
        );
        parsed += 1;
    }
    assert!(parsed > 20, "only {parsed} files were checked");
}

// ---------------------------------------------------------------------------
// the universal normalisations — BOTH generations
// ---------------------------------------------------------------------------

fn fmt(src: &str, version: RustyfiVersion) -> String {
    format_cst(src, version, &CstOptions::default())
        .unwrap_or_else(|| panic!("{version:?} declined {src:?}"))
}

/// A file with no final newline gains exactly one, under both grammars.
///
/// This is what stopped the CST formatter being a drop-in replacement for the
/// lex-based `format`: pointing the playground at `format_cst` alone broke two
/// self-test checks, because 0.0.6 trimmed and capped but never terminated, and
/// 0.1 took the identity path and did none of the three.
#[test]
fn a_file_with_no_final_newline_gains_exactly_one() {
    for (src, want) in [
        ("let x = 1 in x", "let x = 1\nin\nx\n"),
        ("let x = 1\nin\nx\n", "let x = 1\nin\nx\n"),
        // Already terminated, twice over: the extra blank lines go and one
        // terminator stays. `trim_final_newlines` in LSP's own vocabulary.
        ("let x = 1 in x\n\n\n\n", "let x = 1\nin\nx\n"),
    ] {
        assert_eq!(fmt(src, RustyfiVersion::V0_0), want, "0.0.6: {src:?}");
    }
    for (src, want) in [
        (
            "module M = struct\n  val x = 1\nend",
            "module M = struct\n  val x = 1\nend\n",
        ),
        (
            "module M = struct\n  val x = 1\nend\n",
            "module M = struct\n  val x = 1\nend\n",
        ),
        (
            "module M = struct\n  val x = 1\nend\n\n\n\n",
            "module M = struct\n  val x = 1\nend\n",
        ),
    ] {
        assert_eq!(fmt(src, RustyfiVersion::V0_1), want, "0.1: {src:?}");
    }
}

/// A file whose last token is a **header** must NOT gain a second newline.
///
/// `lex_header` swallows the line's terminator into the token
/// (`lexer.rs:915-933`), so such a buffer already ends in a newline over an
/// EMPTY final gap. Judging by the emitted text alone is what made the lex-based
/// formatter append one every save and `format` non-idempotent
/// (`format.rs:536-556`). Every file mid-typing, before its body is written, is
/// this shape.
#[test]
fn a_file_ending_in_a_header_does_not_gain_a_second_newline() {
    for version in [RustyfiVersion::V0_0, RustyfiVersion::V0_1] {
        assert_eq!(fmt("@require: stdja\n", version), "@require: stdja\n");
        // …and one with no terminator at all still gets exactly one.
        assert_eq!(fmt("@require: stdja", version), "@require: stdja\n");
        // Two headers, the second unterminated.
        assert_eq!(
            fmt("@require: a\n@require: b", version),
            "@require: a\n@require: b\n"
        );
        // And it is a fixpoint, which is the property the bug actually broke.
        let once = fmt("@require: stdja\n", version);
        assert_eq!(fmt(&once, version), once, "{version:?}: not idempotent");
    }
    // CRLF, where the terminator is two bytes and the lexer deliberately takes
    // both: the trim removes whole terminators and writes the file's own back.
    let out = fmt("@require: stdja\r\n", RustyfiVersion::V0_0);
    assert_eq!(out, "@require: stdja\r\n");
    for (i, b) in out.as_bytes().iter().enumerate() {
        match b {
            b'\r' => assert_eq!(out.as_bytes().get(i + 1), Some(&b'\n'), "lone CR at {i}"),
            b'\n' => assert_eq!(
                i.checked_sub(1).and_then(|j| out.as_bytes().get(j)),
                Some(&b'\r'),
                "lone LF at {i}"
            ),
            _ => {}
        }
    }
}

/// Trailing whitespace goes, on every line including the last, under both
/// grammars — and a run of blank lines caps at two.
#[test]
fn trailing_whitespace_goes_and_a_blank_run_caps_at_two() {
    assert_eq!(
        fmt("let a = 1   \nin ()   \n", RustyfiVersion::V0_0),
        "let a = 1\nin\n()\n"
    );
    // The last line, with no terminator after it: under slice 1 alone this was
    // "not trailing whitespace on a line" and survived.
    assert_eq!(
        fmt("let x = 1 in x  ", RustyfiVersion::V0_0),
        "let x = 1\nin\nx\n"
    );
    assert_eq!(
        fmt(
            "module M = struct\n  val x = 1   \nend  ",
            RustyfiVersion::V0_1
        ),
        "module M = struct\n  val x = 1\nend\n"
    );
    // Five blank lines cap at two, in both generations, and the cap runs
    // against the FINAL line structure — `render.rs`'s `flush_blanks` counts
    // them and `finish` terminates afterwards, which is the ordering
    // `format.rs:466-483` records the bug from getting backwards.
    assert_eq!(
        fmt("let a = 1\n\n\n\n\n\nin ()\n", RustyfiVersion::V0_0),
        "let a = 1\n\n\nin\n()\n"
    );
    assert_eq!(
        fmt(
            "module M = struct\n  val a = 1\n\n\n\n\n\n  val b = 2\nend\n",
            RustyfiVersion::V0_1
        ),
        "module M = struct\n  val a = 1\n\n\n  val b = 2\nend\n"
    );
    // Trailing blank lines are not "a run": there is no content after them, so
    // they are the final-newline rule's business and none survives.
    assert_eq!(
        fmt("let a = 1\nin ()\n   \n\n\n", RustyfiVersion::V0_0),
        "let a = 1\nin\n()\n"
    );
}

/// The shapes `format_cst_identity.rs` asserted were no-ops and that now change,
/// each with its output and its reason. Moved rather than deleted.
#[test]
fn the_awkward_shapes_that_change_now_that_both_generations_are_normalised() {
    for (src, want, version, why) in [
        (
            "3",
            "3\n",
            RustyfiVersion::V0_0,
            "a one-line file with no final newline: the renderer terminates it",
        ),
        (
            "% no final newline",
            "% no final newline\n",
            RustyfiVersion::V0_0,
            "a comment-only file is still a file, and still gets a terminator",
        ),
        (
            "let x = 1 in {a  b}",
            "let x = 1\nin\n{a b}\n",
            RustyfiVersion::V0_0,
            "the file's end moves AND slice 6 writes the reflowable gap as one \
             space — a run's LENGTH is free everywhere (rule 1, 123 of 123), \
             because the lexer collapses the run to one token and \
             `elaborate.rs:2989-2990` maps it to a single `' '`",
        ),
        (
            "let s = `  spaced  ` in s",
            "let s = `  spaced  `\nin\ns\n",
            RustyfiVersion::V0_0,
            "a literal's interior is inside a token span and is never trimmed",
        ),
        (
            "let x = 1 in x  % trailing comment",
            "let x = 1\nin\nx  % trailing comment\n",
            RustyfiVersion::V0_0,
            "a trailing comment keeps its two-space gap; the file gains its \
             terminator after it",
        ),
        (
            "module M = struct\n  val x = 1\nend",
            "module M = struct\n  val x = 1\nend\n",
            RustyfiVersion::V0_1,
            "the same rule on the 0.1 path, which used to make no edits at all",
        ),
    ] {
        assert_eq!(fmt(src, version), want, "{why}");
        // Each of them still has to be token-preserving and a fixpoint.
        assert_same_tokens(src, want, version, why);
        assert_eq!(fmt(want, version), want, "{why}: not a fixpoint");
    }
    // The empty buffer is the one case that gains nothing: there is no file to
    // terminate, and `format_cst` answers before the renderer is reached.
    assert_eq!(fmt("", RustyfiVersion::V0_0), "");
    assert_eq!(fmt("   \n\n", RustyfiVersion::V0_0), "");
}
