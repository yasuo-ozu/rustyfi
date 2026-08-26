//! The formatter, held to the three properties that make it safe to run on
//! somebody else's document, in **both generations** and against the **real
//! corpus** this repository ships.
//!
//! # The three properties
//!
//! 1. **Meaning preservation.** The formatted text lexes to the *same token
//!    stream* — same slots, same payloads, same order. That is not a proxy for
//!    meaning, it is meaning: everything downstream of `rustyfi-syntax` reads
//!    tokens, so an identical stream compiles to an identical document. See
//!    [`assert_same_tokens`].
//! 2. **Text and math areas are byte-identical.** Property 1 alone does not
//!    say this — `Token::Space` carries no length, so collapsing `{a  b}` to
//!    `{a b}` would slip through it while changing the inter-word glue the
//!    line breaker sees. So the bytes are compared directly, over regions
//!    derived from the *token kinds* rather than from the formatter's own area
//!    fold. See [`assert_text_areas_untouched`], and
//!    [`a_formatter_that_collapsed_prose_would_fail_the_text_area_check`] for
//!    the proof that the check can fail.
//! 3. **Idempotence.** `format(format(x)) == format(x)`.
//!
//! # Why the corpus sweep is the test that matters
//!
//! The fixtures below are hand-written, which means they contain exactly the
//! shapes their author thought of. `lib-rustyfi/dist/packages` +
//! `layout-tests/corpus` (162 files) and `lib-rustyfi/dist-v01/packages` (47)
//! are real, largely third-party, hand-formatted SATySFi — tab indentation,
//! aligned `val` blocks, comments inside `sig`s, CJK prose, math,
//! `\cmd(…)(…){…}` argument lists — and they are the only honest evidence
//! that the three properties hold on code nobody wrote for this test.
//!
//! What the sweep actually does, as of this commit: of the 162 0.0.6 files it
//! changes 87, and every change is one of the five normalisations
//! `crate::format` documents — blank-line capping in `code-printer.satyh`,
//! leading blank lines in `pervasives.satyh`, tab indentation in
//! `latexcmds.satyh`. Of the 47 0.1 files it changes 1. The numbers are not
//! asserted (see `CLAUDE.md` on measured numbers in CI); what is asserted is
//! that the sweep is not vacuous — that files were formatted, that some of
//! them changed, and that over a thousand text/math regions were compared.

use std::path::{Path, PathBuf};

use rustyfi_lsp::{format, format_auto, FormatOptions, RustyfiVersion};
use rustyfi_syntax::{Atom, Token};

// ---------------------------------------------------------------------------
// The three properties, as reusable assertions
// ---------------------------------------------------------------------------

/// Every token, with its payload, in order. `None` if the text does not lex.
fn tokens(src: &str, version: RustyfiVersion) -> Option<Vec<Token>> {
    rustyfi_syntax::lex_with_version(src, version)
        .ok()
        .map(|atoms| atoms.into_iter().map(|a| a.slot).collect())
}

/// The formatted text lexes to exactly the token stream the original did.
///
/// Deliberately the *whole* stream rather than a filtered one: a formatter bug
/// that split `::` into `:` `:`, merged `- 1` into `-1`, or turned a
/// `Token::Break` into a `Token::Space` all show up here as a difference, and
/// each of those is a document that renders differently.
fn assert_same_tokens(original: &str, formatted: &str, version: RustyfiVersion, what: &str) {
    let before = tokens(original, version).expect("the fixture lexes");
    let after = tokens(formatted, version)
        .unwrap_or_else(|| panic!("{what}: the formatted text no longer lexes"));
    if before != after {
        let at = before
            .iter()
            .zip(&after)
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| before.len().min(after.len()));
        panic!(
            "{what}: token {at} changed\n  before: {:?}\n   after: {:?}\n  (lengths {} -> {})",
            before.get(at),
            after.get(at),
            before.len(),
            after.len()
        );
    }
}

/// Which tokens the lexer can only have produced while reading inline text,
/// block text or math.
///
/// Derived from the token's *own identity*, which is what makes this
/// independent of the formatter: `rustyfi_syntax`'s lexer mints
/// `Token::Char`/`Token::Space`/`Token::Break` only in horizontal mode,
/// `Token::MathChar`/`Token::Superscript` only in math mode, and the
/// `EHorzGrp`/`EVertGrp`/`EMathGrp` closers only when popping out of one. No
/// bracket matching and no mode stack is involved here, so a bug in
/// `crate::area`'s fold cannot hide from this.
///
/// The openers count too, and they must: `lexer.rs:562-566` emits
/// `Token::BHorzGrp` with a span covering the `{` **and the whitespace it then
/// skipped**, so the leading spaces of `{  hello }` live inside that token.
///
/// `Token::Literal` is excluded on purpose — a backtick literal is program
/// syntax too (`` let s = `x` ``), so its presence says nothing about the
/// area. Its bytes are inside a token span either way.
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

/// The byte ranges a document's prose and math occupy: maximal runs of
/// adjacent text/math tokens, **including the gaps between them** so that a
/// comment or a stray byte between two `Token::Char`s is covered as well.
///
/// A run ends at the first token that is not one — which is exactly where a
/// program sub-area begins, e.g. the `(…)` of `\frame(2pt)(…){…}` written
/// inside inline text. Those parenthesised arguments are program text and the
/// formatter may normalise their whitespace, so including them here would
/// assert something untrue.
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

/// Not one byte of inline text, block text or math changed.
///
/// The regions are matched up by **token index**, not by offset: the formatter
/// shifts everything after its first edit, so the *n*th region of the original
/// is compared with the *n*th of the result. That is only well defined because
/// [`assert_same_tokens`] has already established the two streams correspond
/// one-for-one, which is why this asserts the counts before it trusts them.
fn assert_text_areas_untouched(
    original: &str,
    formatted: &str,
    version: RustyfiVersion,
    what: &str,
) -> usize {
    let before = rustyfi_syntax::lex_with_version(original, version).expect("the fixture lexes");
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

/// Format, then assert all three properties. Returns the formatted text and
/// how many text/math regions were checked, so a caller can prove its fixture
/// was not vacuous.
fn round_trip(src: &str, version: RustyfiVersion, what: &str) -> (String, usize) {
    let opts = FormatOptions::default();
    let once = format(src, version, &opts)
        .unwrap_or_else(|| panic!("{what}: declined to format a buffer that lexes"));
    assert_same_tokens(src, &once, version, what);
    let regions = assert_text_areas_untouched(src, &once, version, what);
    let twice = format(&once, version, &opts)
        .unwrap_or_else(|| panic!("{what}: declined to format its own output"));
    assert_eq!(twice, once, "{what}: format is not idempotent");
    (once, regions)
}

// ---------------------------------------------------------------------------
// The primary invariant: text and math are never touched
// ---------------------------------------------------------------------------

#[test]
fn double_spaces_and_blank_lines_inside_inline_text_are_content_and_survive() {
    // Every one of these would be "tidied" by a formatter that did not know
    // what area it was in, and every one of them changes the rendered PDF:
    // the doubled spaces are two `Token::Space`s of glue, the trailing spaces
    // before the newline join the `Token::Break`, and the blank line is a
    // paragraph-shaped gap in the source of one paragraph.
    let src = "let doc = {hello  world   \n\n  and  more}\n";
    let (out, regions) = round_trip(src, RustyfiVersion::V0_0, "inline prose");
    assert!(regions > 0, "the fixture must contain a text area");
    assert_eq!(out, src, "prose must come back byte for byte");
}

#[test]
fn block_text_and_math_keep_their_own_whitespace() {
    let src = "let doc = '<  +p {  a  b  }  \n\n  +p{ ${ x  +  y } }  >\n";
    let (out, regions) = round_trip(src, RustyfiVersion::V0_0, "block and math");
    assert!(regions > 0);
    assert_eq!(out, src);
}

#[test]
fn cjk_prose_is_reproduced_byte_for_byte() {
    // The corpus is largely Japanese, and CJK adjacency is what makes an
    // inter-character break legal or not (`linebreak.rs`). A formatter that
    // normalised a space next to a kanji would move a line break.
    let src = "let doc = {日本語  と  英語 mixed\n  そして  改行}\n";
    let (out, _) = round_trip(src, RustyfiVersion::V0_0, "CJK prose");
    assert_eq!(out, src);
}

#[test]
fn a_formatter_that_collapsed_prose_would_fail_the_text_area_check() {
    // The point of this test is that the previous three can FAIL. `collapse`
    // is a plausible-looking formatter — it squeezes runs of spaces, which is
    // what most language formatters do — applied to the same fixture. The
    // token stream survives it (`Token::Space` records no length), so property
    // 1 passes; property 2 is what catches it.
    fn collapse(src: &str) -> String {
        let mut out = String::new();
        let mut last_was_space = false;
        for c in src.chars() {
            if c == ' ' && last_was_space {
                continue;
            }
            last_was_space = c == ' ';
            out.push(c);
        }
        out
    }
    let src = "let doc = {hello  world}\n";
    let mangled = collapse(src);
    assert_ne!(mangled, src, "the mutant must actually change something");
    assert_same_tokens(src, &mangled, RustyfiVersion::V0_0, "mutant");

    let caught = std::panic::catch_unwind(|| {
        assert_text_areas_untouched(src, &mangled, RustyfiVersion::V0_0, "mutant")
    });
    assert!(
        caught.is_err(),
        "the text-area check passed a formatter that rewrote prose — it proves nothing"
    );
}

#[test]
fn a_comment_inside_inline_text_is_left_exactly_where_it_is() {
    // The one place a *gap* — bytes covered by no token — exists inside a text
    // area: `lexer.rs:1093-1097` swallows `%` to end of line and the
    // whitespace after it, emitting nothing. The whitespace it swallowed is
    // prose the author wrote, so the trailing run before the `%` and the
    // indentation after it both stay.
    let src = "let doc = {hello   % why\n   world}\n";
    let (out, regions) = round_trip(src, RustyfiVersion::V0_0, "comment in prose");
    assert!(regions > 0);
    assert_eq!(out, src);
}

#[test]
fn program_areas_nested_inside_inline_text_are_still_program_areas() {
    // `\frame(…)(…){…}`: the parenthesised arguments are read in program mode
    // even though they sit inside `{ … }`, so their whitespace IS
    // normalisable — and the `{ … }` around them still is not.
    let src = "let doc = {\\frame(2pt)(  1  ){  a  b  }}\n";
    let (out, _) = round_trip(src, RustyfiVersion::V0_0, "command arguments");
    assert_eq!(
        out, src,
        "interior spaces are alignment and are preserved in program text too"
    );

    // The same arguments spread over lines: the trailing whitespace inside the
    // parens goes, the prose does not.
    let src = "let doc = {\\frame(2pt)(   \n  1   \n){  a  b  }}\n";
    let (out, _) = round_trip(src, RustyfiVersion::V0_0, "multi-line arguments");
    assert_eq!(out, "let doc = {\\frame(2pt)(\n  1\n){  a  b  }}\n");
}

// ---------------------------------------------------------------------------
// What IS normalised
// ---------------------------------------------------------------------------

fn fmt(src: &str) -> String {
    format(src, RustyfiVersion::V0_0, &FormatOptions::default()).expect("a formattable buffer")
}

#[test]
fn trailing_whitespace_goes_from_program_lines() {
    assert_eq!(fmt("let x = 1   \nlet y = 2\t\n"), "let x = 1\nlet y = 2\n");
}

#[test]
fn a_comment_keeps_its_text_its_indentation_and_the_space_before_it() {
    // The two-space gap before an end-of-line comment is a convention the
    // corpus uses (`azmath/src/parens.satyh`), so it is inter-token space and
    // stays; what trails the comment is whitespace and goes.
    let src = "let x = 1  % why   \n  % indented   \nlet y = 2\n";
    assert_eq!(fmt(src), "let x = 1  % why\n  % indented\nlet y = 2\n");
}

#[test]
fn runs_of_blank_lines_are_capped_but_a_two_line_section_break_survives() {
    assert_eq!(
        fmt("let x = 1\n\n\n\n\n\nlet y = 2\n"),
        "let x = 1\n\n\nlet y = 2\n"
    );
    // Two blank lines is how the bundled `itemize.satyh` separates groups of
    // definitions. Capping at one would rewrite that file for no reason.
    let src = "let x = 1\n\n\nlet y = 2\n";
    assert_eq!(fmt(src), src);
}

#[test]
fn leading_blank_lines_go_entirely() {
    assert_eq!(fmt("\n\n\nlet x = 1\n"), "let x = 1\n");
    // Not the comment above the first binding, though — that is content.
    assert_eq!(
        fmt("\n\n% header\n\nlet x = 1\n"),
        "% header\n\nlet x = 1\n"
    );
}

#[test]
fn the_blank_line_after_a_header_is_counted_from_the_right_place() {
    // `lex_header` swallows the header's own line break into the token
    // (`lexer.rs:915-921`), so the gap after it starts at column 0 and its
    // first newline ends a BLANK line. Getting this wrong lets one extra blank
    // line through after every `@require:` in the corpus and nowhere else.
    assert_eq!(
        fmt("@require: a\n\n\n\n\n\nlet x = 1\n"),
        "@require: a\n\n\nlet x = 1\n"
    );
    let src = "@require: a\n\n\nlet x = 1\n";
    assert_eq!(fmt(src), src);
}

#[test]
fn a_headers_own_trailing_space_is_left_alone_because_it_is_inside_the_token() {
    // `@require: foo   ` lexes as `HeaderRequire("foo   ")` — the spaces are
    // part of the payload, so trimming them would change the package name the
    // loader is asked for. This is a limitation, and it is the right one: the
    // formatter does not edit token text.
    let src = "@require: foo   \nlet x = 1\n";
    assert_eq!(fmt(src), src);
    assert_same_tokens(src, &fmt(src), RustyfiVersion::V0_0, "header payload");
}

#[test]
fn indentation_is_preserved_verbatim_because_this_is_not_a_re_indenter() {
    // Deliberately ragged. A bracket-driven re-indenter would rewrite every
    // line of this; see the module comment on `crate::format` for the corpus
    // measurement that ruled that out.
    let src = "let f x =\n      let y = x in\n  y\nin\n";
    assert_eq!(fmt(src), src);
}

#[test]
fn tabs_in_indentation_become_the_columns_they_stood_for() {
    let src = "let f x =\n\t\tlet y = x in\n\t y\n";
    assert_eq!(
        format(
            src,
            RustyfiVersion::V0_0,
            &FormatOptions {
                tab_size: 4,
                ..FormatOptions::default()
            }
        )
        .unwrap(),
        "let f x =\n        let y = x in\n     y\n"
    );
    // A client that wants tabs keeps them.
    assert_eq!(
        format(
            src,
            RustyfiVersion::V0_0,
            &FormatOptions {
                insert_spaces: false,
                ..FormatOptions::default()
            }
        )
        .unwrap(),
        src
    );
}

#[test]
fn a_tab_inside_inline_text_survives_the_client_that_asked_for_spaces() {
    // Tabs are expanded in *program* indentation only. Inside `{ … }` a tab is
    // part of a `Token::Space` run and expanding it would be re-typesetting.
    let src = "let doc = {a\tb}\n";
    let (out, _) = round_trip(src, RustyfiVersion::V0_0, "tab in prose");
    assert_eq!(out, src);
}

#[test]
fn every_lsp_option_can_be_turned_off() {
    // The comment line is load-bearing. Without it this fixture asserted the
    // identity over a source with no `%` in it, and the comment arm of
    // `rewrite_gap` trimmed unconditionally — so "every rule off" was false
    // for the one case the test was named after, and it passed anyway.
    let src = "let x = 1   \n% note   \n\n\n\n\nlet y = 2   \n\n\n";
    let out = format(
        src,
        RustyfiVersion::V0_0,
        &FormatOptions {
            trim_trailing_whitespace: false,
            insert_final_newline: false,
            trim_final_newlines: false,
            max_blank_lines: usize::MAX,
            ..FormatOptions::default()
        },
    )
    .unwrap();
    assert_eq!(out, src, "with every rule off, formatting is the identity");
}

#[test]
fn whitespace_is_never_inserted_where_the_author_wrote_none() {
    // Not an oversight — see the module comment. Inserting a space is the one
    // whitespace edit that could re-tokenise a file, and `let x=1` is rare
    // enough that the trade is not close.
    let src = "let x=1\nlet y=x+1\n";
    assert_eq!(fmt(src), src);
}

// ---------------------------------------------------------------------------
// Declining
// ---------------------------------------------------------------------------

#[test]
fn a_buffer_that_does_not_lex_is_declined_rather_than_guessed_at() {
    // An unterminated inline area: the mode stack never comes back to program
    // text, so there is no area map to format against.
    assert_eq!(
        format(
            "let doc = {hello\n",
            RustyfiVersion::V0_0,
            &FormatOptions::default()
        ),
        None
    );
    // An unterminated backtick literal, the other common half-typed shape.
    assert_eq!(
        format(
            "let s = `open\n",
            RustyfiVersion::V0_0,
            &FormatOptions::default()
        ),
        None
    );
}

#[test]
fn a_buffer_that_lexes_but_does_not_parse_is_still_formatted() {
    // Formatting reads the token stream, not the tree, so a construct this
    // port's grammar has not implemented — or a half-written binding — is
    // still tidied. Refusing here would be refusing for a reason unrelated to
    // the answer.
    let src = "let x = = = 1   \n\n\n\n";
    assert!(!rustyfi_lsp::analyze(src, RustyfiVersion::V0_0).is_empty());
    assert_eq!(fmt(src), "let x = = = 1\n");
}

#[test]
fn an_empty_buffer_is_left_empty_rather_than_given_a_line() {
    assert_eq!(
        format("", RustyfiVersion::V0_0, &FormatOptions::default()),
        Some(String::new())
    );
    // A buffer of nothing but whitespace has a line, and it ends properly.
    assert_eq!(fmt("   \n  \n"), "\n");
}

// ---------------------------------------------------------------------------
// Both generations
// ---------------------------------------------------------------------------

#[test]
fn the_v01_surface_formats_under_its_own_grammar() {
    // `val`/`module … : sig … end = struct … end` is 0.1's shape, and
    // `signature`/`include`/`use` are words 0.0.6 does not reserve.
    let src = "module M : sig   \n  val x : int   \nend = struct   \n\n\n\n\n  val x = 1   \nend";
    let out = format(src, RustyfiVersion::V0_1, &FormatOptions::default()).expect("formats");
    assert_eq!(
        out,
        "module M : sig\n  val x : int\nend = struct\n\n\n  val x = 1\nend\n"
    );
    assert_same_tokens(src, &out, RustyfiVersion::V0_1, "0.1 module");
}

#[test]
fn v01_inline_text_is_as_untouchable_as_v006_inline_text() {
    let src = "module M = struct\n  val doc = {hello  world}   \nend\n";
    let (out, regions) = round_trip(src, RustyfiVersion::V0_1, "0.1 prose");
    assert!(regions > 0);
    assert_eq!(out, "module M = struct\n  val doc = {hello  world}\nend\n");
}

#[test]
fn the_generation_is_detected_the_same_way_diagnostics_detect_it() {
    // `@stage:` is a hard 0.0.6 signal (0.1 deleted the header), and a buffer
    // that carries it must not be read with 0.1's lexer — which rejects it.
    let src = "@stage: 1\nlet x = 1   \n";
    assert_eq!(
        format_auto(src, &FormatOptions::default()),
        Some("@stage: 1\nlet x = 1\n".to_string())
    );
    // A `use` head is a hard 0.1 signal.
    let src = "use package `foo`\nmodule M = struct\n  val x = 1   \nend\n";
    assert_eq!(
        format_auto(src, &FormatOptions::default()),
        Some("use package `foo`\nmodule M = struct\n  val x = 1\nend\n".to_string())
    );
}

// ---------------------------------------------------------------------------
// The corpus
// ---------------------------------------------------------------------------

/// The 0.0.6 corpus: everything this repository ships that is read with the
/// 0.0.6 grammar. `dist-v01` is deliberately left out — its files are 0.1 and
/// belong to the sweep below.
fn corpus_v006() -> Vec<PathBuf> {
    let root = repo_root();
    let mut out = Vec::new();
    collect(&root.join("lib-rustyfi/dist/packages"), &mut out);
    collect(&root.join("layout-tests/corpus"), &mut out);
    out.sort();
    out
}

fn corpus_v01() -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect(&repo_root().join("lib-rustyfi/dist-v01/packages"), &mut out);
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

fn repo_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `<root>/crates/rustyfi-lsp`.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two levels below the workspace root")
        .to_path_buf()
}

/// Format every file, and hold all three properties on each of them.
///
/// A file that does not lex under the generation it is swept with is *skipped*
/// — this is a formatter test, not a lexer test. `max_skipped` is 0 today
/// because every file in both corpora lexes under the generation it is swept
/// with, and a new one that does not is worth being told about: it means the
/// sweep has quietly shrunk, which is the way a corpus test goes vacuous.
fn sweep(files: &[PathBuf], version: RustyfiVersion, max_skipped: usize) -> (usize, usize) {
    assert!(
        files.len() > 20,
        "expected the bundled corpus, found {} files — is the checkout complete?",
        files.len()
    );
    let (mut formatted, mut skipped, mut regions, mut changed) = (0, 0, 0, 0);
    for path in files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        let what = path.display().to_string();
        if tokens(&src, version).is_none() {
            skipped += 1;
            continue;
        }
        let (out, r) = round_trip(&src, version, &what);
        assert_only_whitespace_differs(&src, &out, &what);
        assert_indentation_preserved(&src, &out, &what);
        formatted += 1;
        regions += r;
        changed += usize::from(out != src);
    }
    assert!(
        skipped <= max_skipped,
        "{skipped} of {} files did not lex under {version:?}; the sweep is not measuring what it \
         claims to",
        files.len()
    );
    assert!(
        regions > 1000,
        "only {regions} text/math regions across {formatted} files — the text-area check is \
         nearly vacuous"
    );
    (formatted, changed)
}

#[test]
fn the_bundled_and_third_party_v006_corpus_round_trips() {
    let files = corpus_v006();
    let (formatted, changed) = sweep(&files, RustyfiVersion::V0_0, 0);
    // Not asserted as an exact number — see `CLAUDE.md` on measured numbers in
    // CI — but the sweep must have actually *done* something, or it proves
    // only that the identity function is idempotent.
    assert!(
        changed > 0,
        "{formatted} files formatted and none of them changed"
    );
}

#[test]
fn the_bundled_v01_corpus_round_trips() {
    let files = corpus_v01();
    let (formatted, changed) = sweep(&files, RustyfiVersion::V0_1, 0);
    assert!(formatted > 20, "only {formatted} 0.1 files were formatted");
    // The same guard its 0.0.6 sibling carries, and it matters *more* here:
    // only one of these 47 files is untidy, so the day somebody tidies that
    // one this sweep would silently become an assertion that the identity
    // function is idempotent. If this fires, the fix is to make a corpus file
    // untidy again on purpose — not to delete the guard.
    assert!(
        changed > 0,
        "{formatted} 0.1 files formatted and none of them changed"
    );
}

#[test]
fn the_corpus_round_trips_with_the_generation_detected_per_file() {
    // The path an editor actually takes: no `--lang`, so each file's
    // generation is sniffed. Weaker than the two sweeps above (a file read
    // under the wrong grammar simply declines), but it is the configuration
    // most users run.
    let mut files = corpus_v006();
    files.extend(corpus_v01());
    let opts = FormatOptions::default();
    let mut formatted = 0;
    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        let Some(once) = format_auto(&src, &opts) else {
            continue;
        };
        let version = rustyfi_lsp::detect_version(&src);
        let what = path.display().to_string();
        assert_same_tokens(&src, &once, version, &what);
        assert_text_areas_untouched(&src, &once, version, &what);
        assert_only_whitespace_differs(&src, &once, &what);
        assert_indentation_preserved(&src, &once, &what);
        assert_eq!(
            format_auto(&once, &opts).as_deref(),
            Some(once.as_str()),
            "{what}: not idempotent under detection"
        );
        formatted += 1;
    }
    assert!(
        formatted > 100,
        "only {formatted} of {} corpus files formatted under detection",
        files.len()
    );
}

// ---------------------------------------------------------------------------
// Two properties the three above are structurally blind to
//
// Properties 1-3 all quantify over *tokens* or over text/math *regions*, and a
// program-area gap is neither: the lexer emits no token for it, so any rewrite
// confined to program whitespace is invisible to all three at once. That is not
// a hypothetical hole — turn `format.rs`'s `is_indentation` arm into
// `" ".to_string()` and the formatter becomes a destructive re-indenter that
// flattens every hand-aligned continuation in the corpus, while all three
// sweeps still pass. So the sweep also holds the two promises the module doc
// and the README actually make about those gaps.
// ---------------------------------------------------------------------------

/// Every byte that differs between input and output is **whitespace**.
///
/// Deleting all whitespace from both must give the same string. This is the
/// check that covers what the token stream cannot see at all: `%` comments are
/// skipped by the lexer, so a formatter that dropped one, truncated one, or
/// moved a token past one would satisfy [`assert_same_tokens`] exactly.
fn assert_only_whitespace_differs(original: &str, formatted: &str, what: &str) {
    let squeeze = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };
    let (b, a) = (squeeze(original), squeeze(formatted));
    if b != a {
        let at = b
            .char_indices()
            .zip(a.chars())
            .find(|((_, x), y)| x != y)
            .map(|((i, _), _)| i)
            .unwrap_or_else(|| b.len().min(a.len()));
        let window = |s: &String| -> String {
            s.chars()
                .skip(at.saturating_sub(30))
                .take(70)
                .collect::<String>()
        };
        panic!(
            "{what}: a non-whitespace byte changed at position {at} of the squeezed text\n  \
             before: …{}…\n   after: …{}…",
            window(&b),
            window(&a)
        );
    }
}

/// The leading whitespace of a line, and the rest of it.
fn split_indent(line: &str) -> (&str, &str) {
    let end = line
        .find(|c: char| c != ' ' && c != '\t')
        .unwrap_or(line.len());
    line.split_at(end)
}

/// The lines of `s` that carry something other than whitespace, `\r` stripped.
///
/// Blank lines are dropped because the formatter is *allowed* to add and remove
/// them (the blank-line cap, the leading and trailing ones, the final newline);
/// what it is not allowed to do is add, remove, merge or re-indent a line that
/// has content on it.
fn content_lines(s: &str) -> Vec<&str> {
    s.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .filter(|l| !l.trim().is_empty())
        .collect()
}

/// **"It does not re-indent", executable.**
///
/// The formatter's five normalisations are all *deletions* of whitespace, with
/// exactly one exception: `normalise_indent` rewrites a leading run that
/// contains a **tab**, and only then (`format.rs`'s `if !opts.insert_spaces ||
/// !lead.contains('\t') { return lead.to_string(); }`). So for every line whose
/// source indentation is spaces alone, the output's indentation must be
/// byte-identical — not merely whitespace, *identical*. Lines whose source
/// indentation contains a tab are exempted from the indentation comparison and
/// still checked for content.
///
/// Trailing whitespace is compared with `trim_end` on both sides rather than
/// asserted equal: the formatter trims it in program areas and preserves it
/// inside a text area (where it is a `Token::Space`), and
/// [`assert_text_areas_untouched`] is what holds the second of those.
fn assert_indentation_preserved(original: &str, formatted: &str, what: &str) {
    let (before, after) = (content_lines(original), content_lines(formatted));
    assert_eq!(
        before.len(),
        after.len(),
        "{what}: the number of lines with content on them changed ({} -> {}) — the formatter \
         neither adds, removes nor merges such a line",
        before.len(),
        after.len()
    );
    for (i, (b, a)) in before.iter().zip(&after).enumerate() {
        let (bi, bc) = split_indent(b);
        let (ai, ac) = split_indent(a);
        assert_eq!(
            bc.trim_end(),
            ac.trim_end(),
            "{what}: the content of content-line {i} changed"
        );
        if !bi.contains('\t') {
            assert_eq!(
                bi, ai,
                "{what}: content-line {i} was re-indented ({:?} -> {:?}); the only indentation \
                 the formatter may rewrite is one containing a tab",
                bi, ai
            );
        }
    }
}

#[test]
fn a_formatter_that_re_indented_would_fail_the_indentation_check() {
    // The falsification for `assert_indentation_preserved`, in the same shape
    // as the text-area check's own: a check nobody has seen fail is a check
    // nobody knows can.
    let src = "let x =\n      f 1\n";
    let reindented = "let x =\n f 1\n";
    assert_same_tokens(src, reindented, RustyfiVersion::V0_0, "re-indent");
    assert_text_areas_untouched(src, reindented, RustyfiVersion::V0_0, "re-indent");
    assert_only_whitespace_differs(src, reindented, "re-indent");
    let caught = std::panic::catch_unwind(|| {
        assert_indentation_preserved(src, reindented, "re-indent");
    });
    assert!(caught.is_err(), "the indentation check cannot fail");
}

#[test]
fn a_formatter_that_ate_a_comment_would_fail_the_whitespace_only_check() {
    // Comments are invisible to the lexer, so this is the one of the two that
    // properties 1-3 cannot substitute for even in principle.
    let src = "let x = 1 % why\n";
    let eaten = "let x = 1 %\n";
    assert_same_tokens(src, eaten, RustyfiVersion::V0_0, "eaten comment");
    let caught = std::panic::catch_unwind(|| {
        assert_only_whitespace_differs(src, eaten, "eaten comment");
    });
    assert!(caught.is_err(), "the whitespace-only check cannot fail");
}

// ---------------------------------------------------------------------------
// `<[ … ]>` — the path literal that used to walk the area replay out of math
// ---------------------------------------------------------------------------

/// An unmatched `<[` inside a program sub-area of math used to pop the wrong
/// entry and hand the rest of the math to the formatter as program text.
///
/// `rustyfi_syntax`'s lexer emits `Token::BPath` (`lexer.rs:712`) and
/// `Token::EPath` (`:550`) with **no** `push_mode`/`pop_mode`, and nothing
/// requires the two to pair — an unmatched `<[` is not a lex error. When
/// `crate::area` pushed on `BPath` anyway, the `)` below popped the path's
/// phantom `Program` instead of the `!(`'s, leaving the replay in `Program`
/// while the lexer was back in `Math`; the blank lines that follow are inside
/// `${ … }` and were capped.
#[test]
fn an_unmatched_path_opener_does_not_let_the_formatter_into_the_math() {
    let src = "let m = ${ \\frac!( 1 <[ 2 )\n\n\n\n\n} in m\n";
    let out = format(src, RustyfiVersion::V0_0, &FormatOptions::default()).expect("formats");
    assert_eq!(out, src, "bytes inside `${{ … }}` were rewritten");
    round_trip(src, RustyfiVersion::V0_0, "unmatched `<[` in math");
}

/// The conservative twin of the same bug, in the same buffer: once the `}`
/// popped, the replay was left believing it was still in `Math`, so the
/// program text *after* the math was never tidied — including the final
/// newline the client asked for.
#[test]
fn an_unmatched_path_opener_does_not_leave_the_replay_stuck_in_math() {
    let src = "let m = ${ \\frac!( <[ 1 ) } in m   \n\n\n\n\n";
    let out = format(src, RustyfiVersion::V0_0, &FormatOptions::default()).expect("formats");
    assert_eq!(out, "let m = ${ \\frac!( <[ 1 ) } in m\n");
    round_trip(src, RustyfiVersion::V0_0, "stuck-in-math `<[`");
}

/// A *matched* path literal is program text throughout, and stays so — the
/// fix must not have cost the ordinary case anything. This is also the
/// completion side of the change: `area_at` answered `Program` inside a
/// balanced `<[ … ]>` before (a `Program` pushed onto a `Program`) and answers
/// `Program` now, so no namespace moved.
#[test]
fn a_matched_path_literal_is_program_text_and_its_whitespace_is_tidied() {
    let src = "let p = <[ (0pt, 0pt) -- (1pt, 1pt) ]>   \nin p   \n\n\n\n";
    let out = format(src, RustyfiVersion::V0_0, &FormatOptions::default()).expect("formats");
    assert_eq!(out, "let p = <[ (0pt, 0pt) -- (1pt, 1pt) ]>\nin p\n");
    round_trip(src, RustyfiVersion::V0_0, "matched path literal");
    // The whitespace *inside* the literal is tidied too, which is the same
    // fact `area_at` reports to completion: `Program` at every point in a
    // balanced `<[ … ]>`, before the fix and after it.
    let src = "let p = <[ (0pt, 0pt)   \n   -- (1pt, 1pt) ]> in p\n";
    assert_eq!(
        format(src, RustyfiVersion::V0_0, &FormatOptions::default()).expect("formats"),
        "let p = <[ (0pt, 0pt)\n   -- (1pt, 1pt) ]> in p\n"
    );
}

/// The other half of the pair, and the reason the deviation could never have
/// been repaired by "matching them up": a stray `]>` is not a lex error
/// either (the `']'` arm emits `Token::EPath` with no `pop_mode`). Here it
/// pops the `!(`'s `Program` a token early, so the real `)` pops the `Math`
/// and the rest of the math becomes rewritable — the same corruption reached
/// from the closing side.
#[test]
fn a_stray_path_closer_pops_nothing() {
    let src = "let m = ${ \\frac!( ]> 1 )\n\n\n\n\n} in m\n";
    let out = format(src, RustyfiVersion::V0_0, &FormatOptions::default()).expect("formats");
    assert_eq!(out, src, "bytes inside `${{ … }}` were rewritten");
}
