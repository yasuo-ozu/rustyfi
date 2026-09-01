//! Slices 1 and 2's acceptance criteria: re-indentation and canonical
//! intra-line spacing of 0.0.6 program text, held to five properties over the
//! real corpus.
//!
//! Slice 0's claim — `format_cst(src) == Some(src)` — is gone for 0.0.6 by
//! design; that was the point of slice 1. `tests/format_cst_identity.rs` keeps
//! it for the 0.1 corpus, which has no builder yet, and for the shapes these
//! slices genuinely do not touch. What replaces it here is not weaker, it is
//! *different*: five claims, each of which a broken layout rule can fail.
//!
//! 1. **Token-stream identity.** The formatted text lexes to the same
//!    `Vec<Token>`. `format_cst`'s always-on verifier already checks this per
//!    call and DECLINES on failure — so the sweep asserts it *again*, from
//!    outside, and separately asserts nothing declined. Without the second
//!    half, a formatter whose verifier rejected every file would sweep green.
//! 2. **Every text/math token's own bytes are identical**, over token kinds
//!    rather than the formatter's own idea of where an area is. Per TOKEN
//!    rather than per region since block text and math got real layout: the
//!    region form extended each run over the gaps between its tokens, and
//!    those gaps are exactly what those two areas' policy frees. The ONE
//!    relaxation is inline text's, and it is a named set of seven token kinds
//!    compared modulo whitespace — see [`swallows_trivia`].
//! 3. **Idempotence**: `format_cst(format_cst(x)) == format_cst(x)`.
//!    `LineBreaks::Preserve` reads the author's line breaks, which
//!    `engine.md` section 6 calls idempotence hazard class 1 — so this is the
//!    property with the most to prove, and it is proved on real files.
//! 4. **No non-blank line is added, removed or merged, and no token moved
//!    between two of them.** A stronger statement than "the token stream is
//!    the same", which is global and so cannot see a token migrating across a
//!    line boundary. Under slice 1 this was `line.trim()` equality — "only the
//!    indentation moved". Slice 2 rewrites gaps in the middle of a line, so
//!    the comparison is now [`squeeze`], and that function's doc comment
//!    carries the argument for why the weakening is still a real check.
//! 5. **The walk stays in step with the atom stream** (`cst_walk_desync`).
//!    Nothing else in this file could notice a drift: it misattributes
//!    indentation, and every property above holds either way.
//!
//! # What this sweep CANNOT see, measured
//!
//! Mutation-tested, and the result is the reason the per-construct layout
//! fixtures in `build006.rs`/`build01.rs` are not decoration. Forcing every
//! `Auto` group FLAT and forcing every one BROKEN were both applied to
//! `render.rs`; so was a one-column error in `fits`'s budget, in both
//! directions; so was turning `OpChain`'s all-or-nothing rule into a fill.
//!
//! ```text
//!   mutation                        this sweep   0.1 sweep   layout fixtures
//!   every group flat                   PASS        PASS        34 fail
//!   every group broken                 FAIL         FAIL       86 fail
//!   fits budget one column short       PASS        PASS         2 fail
//!   fits budget one column long        PASS        PASS         5 fail
//!   OpChain fills instead              PASS        PASS         3 fail
//! ```
//!
//! Four of the five are invisible here, and correctly so: a wrong break
//! DECISION changes no token, moves no non-whitespace byte, and is a fixpoint
//! — every property below still holds. That is not a hole to be plugged by
//! adding an assertion here; it is what says the sweep and the fixtures are
//! checking different things. It is also the honest explanation for how a
//! break landing inside `\frac{…}{…}` and inside `e^{-x^2}` reached a user:
//! the sweep was green throughout, and only a human reading the output could
//! object.
//!
//! The impact numbers are printed, not asserted — `CLAUDE.md` on measured
//! numbers in CI, and the CI build job has no fonts, so a metric assertion is
//! an outage waiting to happen. What *is* asserted is non-vacuity: files were
//! swept, and some of them changed. "Nothing changed" is how a re-indenter goes
//! quietly green.

use std::path::{Path, PathBuf};

use rustyfi_lsp::{cst_walk_desync, format_cst, CstOptions, RustyfiVersion};
use rustyfi_syntax::Token;

// ---------------------------------------------------------------------------
// corpus discovery — same two roots `tests/format.rs` and
// `tests/format_cst_identity.rs` use
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
/// Slices 1 and 2 keep every break the author wrote, so neither can merge two
/// of these nor split one. Blank lines are excluded because the blank-line cap
/// really does remove some — and a line of nothing but spaces counts as blank
/// on both sides, which is trailing-whitespace trimming rather than a lost
/// line.
fn content_lines(s: &str) -> Vec<&str> {
    s.lines().filter(|l| !l.trim().is_empty()).collect()
}

/// A content line with **every** whitespace character removed.
///
/// The comparison property 4 makes line by line, and slice 2 is why it is this
/// and not `trim()`. Under slice 1 a line could only gain or lose *leading*
/// whitespace, so `trim()` was the exact statement of "only the indentation
/// moved". Slice 2 rewrites gaps in the middle of a line (`b   =   2` ->
/// `b = 2`) and inserts spaces that were never written (`a=1` -> `a = 1`), so
/// `trim()` now fails on correct output and collapsing runs still fails on the
/// insertion.
///
/// Deleting whitespace entirely is not a weakening to nothing, because
/// property 1 does not subsume it: the token stream is a *global* sequence, so
/// a bug that moved a token from the end of one line to the start of the next
/// leaves it identical. This is what says every token stayed on the line the
/// author put it on — which, together with the line COUNT below, is the whole
/// of "no line was added, removed or merged".
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
    // merged" was exact and was the property with the most to say. Deciding
    // line breaks is precisely the act of adding, removing and merging content
    // lines; asserting otherwise would assert the feature does not work.
    //
    // What survives is the part that is still true and still catches the bug
    // the per-line form was really there for: no non-whitespace byte is added,
    // removed or reordered in THIS configuration either. 4a says it for the
    // live default; a formatter that dropped a token only when the wraps were
    // off would slip past 4a alone.
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

    // 6. **The budget is a live input, so it is swept.** Every property above
    // runs at the default 100 columns, and at 100 a great many groups fit and
    // are never asked to break — so a `fits` that was wrong by a column, or a
    // group whose broken arm is malformed, can hide behind a corpus that
    // mostly fits. Sixty forces almost every group open and two hundred forces
    // almost every one flat; both have to survive the verifier and both have to
    // be fixpoints. This is the check slice 3 adds, and it is the one that
    // found the fence-post.
    for width in [60usize, 200] {
        let opts = CstOptions { max_width: width, ..CstOptions::default() };
        let once = format_cst(src, version, &opts).unwrap_or_else(|| {
            panic!("{what}: DECLINED at max_width {width}")
        });
        assert_same_tokens(src, &once, version, what);
        let twice = format_cst(&once, version, &opts)
            .unwrap_or_else(|| panic!("{what}: declined its own output at max_width {width}"));
        assert_eq!(
            twice, once,
            "{what}: not idempotent at max_width {width}"
        );
    }

    // 5.
    if let Some(desync) = cst_walk_desync(src, version, &opts) {
        assert_eq!(
            desync, 0,
            "{what}: the CST walk drifted out of step with the atom stream \
             {desync} time(s), so indentation is attributed to the wrong depth. \
             Nothing else in this file can see that."
        );
    }

    (once != src, differing, a.len(), regions, respelled)
}

// ---------------------------------------------------------------------------
// the sweep
// ---------------------------------------------------------------------------

#[test]
fn slice_1_holds_all_five_properties_over_the_v006_corpus() {
    let files = corpus(&["lib-rustyfi/dist/packages", "layout-tests/corpus"]);
    assert!(
        files.len() > 20,
        "expected the bundled corpus, found {} files — is the checkout complete?",
        files.len()
    );
    let (mut checked, mut changed, mut lines_changed, mut lines, mut regions) = (0, 0, 0, 0, 0);
    let mut respelled = 0usize;
    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        let what = path.display().to_string();
        let (did_change, differing, total, r, resp) = check(&src, RustyfiVersion::V0_0, &what);
        respelled += resp;
        checked += 1;
        changed += usize::from(did_change);
        lines_changed += differing;
        lines += total;
        regions += r;
    }
    eprintln!(
        "slices 1+2, 0.0.6 corpus: {checked} files checked, {changed} changed, \
         {lines_changed} of {lines} content lines re-laid-out, \
         {regions} text/math tokens compared against their area's policy, \
         {respelled} inline gaps re-spelled by slice 6"
    );
    assert!(
        checked > 100,
        "only {checked} of {} files reached the comparison — this sweep has gone vacuous",
        files.len()
    );
    // THIS ASSERTION HAS TWO TRUE STATES, and which one holds is a fact about
    // the corpus rather than about the formatter. Whoever changes a layout rule
    // owns flipping it, in the same commit, to whichever is true.
    //
    //   A. `assert_eq!(changed, 0, …)` — the corpus is a FIXED POINT. True
    //      after the formatter has been re-applied to the corpus and the result
    //      committed. It is the stronger claim and the one that should hold on
    //      `main`: `rustyfi fmt --check` passes in CI and format-on-save is a
    //      no-op on a formatted file.
    //   B. `assert!(changed > 0, …)` — the rules moved and the corpus has not
    //      caught up yet. True inside a slice that adds a rule, from the commit
    //      that adds it until the commit that re-applies the formatter.
    //
    // **B is what holds here.** Slice 2 added eight spacing rules and the
    // corpus in this commit is still formatted to slice 1's, so `changed` is
    // non-zero: 84 files from the spacing rules alone, measured with every
    // other formatter change held out. Re-apply with
    // `rustyfi fmt lib-rustyfi/dist/packages layout-tests/corpus`, commit the
    // result, and put A back.
    //
    // Neither form is the anti-vacuity check, and that is worth restating,
    // because the history here is a trap. This assertion USED to read
    // `changed > 20` as a proof that re-indentation happens at all — the corpus
    // was doing double duty as that fixture — and formatting the corpus
    // destroyed the fixture without the assertion noticing it had become a
    // tautology. The role MOVED onto input this sweep does not own:
    // `build006.rs`'s string-literal unit tests. B above says "the corpus is
    // stale", nothing more.
    assert!(
        changed > 0,
        "no corpus file changed, but slice 2 has just added spacing rules the \
         committed corpus does not yet follow. Either the corpus was re-applied \
         (in which case put the `assert_eq!(changed, 0, …)` form back — see the \
         comment above) or a rule silently stopped firing."
    );
    assert!(
        regions > 1000,
        "only {regions} text/math tokens were compared, so property 2 is \
         close to vacuous"
    );
    // Property 1's relaxation is the one thing in this file that ACCEPTS a
    // difference, so it is the one thing that can go quietly vacuous: a
    // formatter that stopped re-wrapping inline text would sweep green while
    // the licence sat there licensing nothing.
    assert!(
        respelled > 0,
        "no inline gap was re-spelled anywhere in the corpus, so property 1's \
         `Space` <-> `Break` licence is not being exercised — either slice 6 \
         stopped firing or the relaxation should come out"
    );
}

// The 0.1 corpus sweep lived here, as `the_v01_corpus_holds_the_properties`.
// It has MOVED to `tests/format_cst_slice1_v01.rs`'s
// `slice_1_holds_all_five_properties_over_the_v01_corpus`, which runs the same
// five properties over the same 47 files and adds what this copy could not:
// the per-file corpus impact (files changed, content lines re-indented), and
// `every_v01_corpus_file_parses_and_is_laid_out_by_the_builder`, which is the
// only check that can tell a correctly-laid-out file from one `build01.rs`
// DECLINED — both come back byte-identical from the outside.
//
// One home rather than two, because two sweeps over one corpus disagree
// silently: this file's `check` is 0.0.6's and will grow 0.1-irrelevant
// assertions as the slices diverge.

/// The shapes slice 1 moves that `format_cst_identity.rs` used to assert were
/// no-ops, with what they become and why.
#[test]
fn the_awkward_shapes_slice_1_does_change() {
    let opts = CstOptions::default();
    for (src, want, why) in [
        (
            "\t\tlet x = 1 in x\n",
            "let x = 1\nin\nx\n",
            "tab indentation: subsumed by recomputing every indent — and the \
             file's own `in` and body take a line each, which is what every \
             real document is (`in` at column 0, `document … '< … >` under it)",
        ),
        (
            "@require: stdja-mini\n\n\n\n\nlet x = 1 in x",
            "@require: stdja-mini\n\n\nlet x = 1\nin\nx\n",
            "a run of blank lines, capped at `max_blank_lines` by the renderer",
        ),
        (
            "3\r",
            "3\n",
            "a bare-CR file: slice 1 INVENTS the final terminator, and \
             `dominant_newline` deliberately answers \"\\n\" for a file whose \
             only break is a lone `\\r` (nobody is still writing those, and \
             inventing `\\r` would be worse)",
        ),
    ] {
        assert_eq!(
            format_cst(src, RustyfiVersion::V0_0, &opts).as_deref(),
            Some(want),
            "{why}"
        );
        // Each of them still has to be idempotent and token-preserving.
        assert_same_tokens(src, want, RustyfiVersion::V0_0, why);
        assert_eq!(
            format_cst(want, RustyfiVersion::V0_0, &opts).as_deref(),
            Some(want),
            "{why}: not a fixpoint"
        );
    }
}
