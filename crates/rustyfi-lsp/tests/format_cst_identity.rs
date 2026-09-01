//! Slice 0's acceptance criterion — `format_cst(src) == Some(src)` — kept
//! exactly where it is still true.
//!
//! Slice 1 re-indents program text, so the no-op claim cannot hold for a corpus
//! either grammar has a builder for: that was the point of the slice.
//! `tests/format_cst_slice1.rs` replaced it for 0.0.6 and
//! `tests/format_cst_slice1_v01.rs` for 0.1, each with five properties in place
//! of the one. Nothing here was weakened to make room for either. What survives
//! is every part of the claim slice 1 does not touch:
//!
//! - the **awkward shapes** whose layout slice 1 leaves alone, and only those.
//!   The three it moved when 0.0.6 landed — tab indentation, a run of blank
//!   lines, and a bare-CR file's invented final terminator — went to
//!   `format_cst_slice1.rs`'s `the_awkward_shapes_slice_1_does_change`. The
//!   ones that moved when the FINAL-NEWLINE rule landed for both generations
//!   went to `format_cst_slice1_v01.rs`'s
//!   `the_awkward_shapes_that_change_now_that_both_generations_are_normalised`,
//!   each with its expected output and the reason. None of them was deleted.
//!
//! **The 0.1 corpus sweep lived here** and is now
//! `format_cst_slice1_v01.rs`'s `slice_1_holds_all_five_properties_over_the_v01_corpus`:
//! `build01.rs` lays 0.1 out, so a 0.1 buffer no longer goes through
//! `build_identity` at all unless it fails to parse — which none of the 47 do,
//! and `every_v01_corpus_file_parses_and_is_laid_out_by_the_builder` is the
//! test that says so, because a declined file and a correctly-laid-out one look
//! identical from the outside.
//!
//! The value of the surviving half is unchanged: there is no judgement anywhere
//! in `output == input`, so a failure here is about the IR, the renderer, the
//! trivia scan or the verifier rather than about a layout rule.

use rustyfi_lsp::{format_cst, CstOptions, RustyfiVersion};

// `slice_0_is_a_no_op_on_the_v006_corpus` lived here. Slice 1 makes its claim
// false BY DESIGN — `format_cst` now re-indents 0.0.6 program text — and its
// replacement is `tests/format_cst_slice1.rs`'s
// `slice_1_holds_all_five_properties_over_the_v006_corpus`, which sweeps the
// same 162 files for token identity, text/math byte identity, idempotence,
// "no content line added, removed or merged", and walk/atom-stream agreement.
// It is not the weaker test: byte identity was checkable absolutely, and the
// five properties are what remain checkable absolutely once bytes may move.

/// The shapes a corpus file is unlikely to contain, and the ones the lex-based
/// formatter's bug history says to check: CRLF, a file with no final newline, a
/// file that is nothing but a comment, and the header token's swallowed
/// terminator. Read under 0.0.6, where slice 1 is live, so each of these is a
/// claim that slice 1 leaves the shape alone rather than that nothing can move.
#[test]
fn slice_0_is_a_no_op_on_the_awkward_shapes_slice_1_leaves_alone() {
    let opts = CstOptions::default();
    // Three shapes that were in this list under slice 0 are now in
    // `format_cst_slice1.rs::the_awkward_shapes_slice_1_does_change`, with
    // their expected outputs: `"\t\tlet x = 1 in x\n"` (tab indentation),
    // `"@require: …\n\n\n\n\nlet …"` (a blank-line run past the cap) and
    // `"3\r"` (a bare CR, where slice 1 invents the terminator). They moved
    // rather than being dropped, because each is a shape a corpus file is
    // unlikely to contain and each broke the lex-based formatter once.
    // Every entry here now ENDS IN A NEWLINE, and that is the second wave of
    // moves rather than a coincidence: the renderer terminates every non-empty
    // file exactly once, so a shape that used to be pinned unterminated is
    // pinned terminated in `format_cst_slice1_v01.rs`'s
    // `the_awkward_shapes_that_change_now_that_both_generations_are_normalised`
    // instead. What still belongs here is the claim that the *interior* of each
    // of these does not move — the swallowed edges of a text area, the
    // literal's spaces, the trailing comment's gap, the CRLF pairing, the
    // header's swallowed terminator.
    //
    // The THIRD wave of moves is slice 6, and it took exactly one entry:
    // `"let x = 1\nin\n{a  b}\n"`, whose double space is a `Token::Space` and
    // therefore a gap the inline re-wrap writes as one space. It is asserted
    // just below with its new value rather than dropped, and its `{  a  }`
    // sibling — the same shape at the delimiters, where the run is swallowed
    // into the delimiter's own span and is not a whitespace token at all —
    // takes its place in the no-op list.
    //
    // The FOURTH wave is the inverted spacing default, and it took the math
    // entry: `${x   +   y}` collapses to `${x + y}`, because math whitespace
    // produces no token and is invisible to the typesetter. It is asserted
    // below with its new value. `'< +p{hi} >` stays, single-spaced already.
    for src in [
        "",
        "3\n",
        "3\r\n",
        "% just a comment\n",
        "@require: stdja-mini\r\nlet x = 1\r\nin\r\nx\r\n",
        "let x = 1\nin\n{  a  }\n",
        "let x = 1\nin\n'< +p{hi} >\n",
        "let s = `  spaced  `\nin\ns\n",
        "let x = 1\nin\nx  % trailing comment\n",
    ] {
        let got = format_cst(src, RustyfiVersion::V0_0, &opts);
        assert_eq!(got.as_deref(), Some(src), "slice 0 changed {src:?}");
    }
    // The inverted default's one entry here: math is an area whose gaps it
    // reaches, and its default is `Space::Collapse` — a run becomes one
    // space, an empty gap stays empty (`${abc}` is untouched).
    assert_eq!(
        format_cst("let x = 1 in ${x   +   y}\n", RustyfiVersion::V0_0, &opts).as_deref(),
        Some("let x = 1\nin\n${x + y}\n"),
    );
    assert_eq!(
        format_cst("let x = 1 in ${abc}\n", RustyfiVersion::V0_0, &opts).as_deref(),
        Some("let x = 1\nin\n${abc}\n"),
    );
    // Slice 6's one entry, with its reason: a run's LENGTH is free everywhere
    // (rule 1 of the measurement, 123 of 123 — the lexer collapses the run to
    // one token and `elaborate.rs:2989-2990` maps it to a single `' '`), and
    // the re-wrap writes every gap it may touch as exactly one space.
    assert_eq!(
        format_cst("let x = 1\nin\n{a  b}\n", RustyfiVersion::V0_0, &opts).as_deref(),
        Some("let x = 1\nin\n{a b}\n"),
    );
    // Turning slice 6 off puts it back in the no-op list, which is what says
    // the move is this feature's and not something else's.
    let no_wrap = CstOptions { wrap_inline_text: false, ..CstOptions::default() };
    assert_eq!(
        format_cst("let x = 1\nin\n{a  b}\n", RustyfiVersion::V0_0, &no_wrap).as_deref(),
        Some("let x = 1\nin\n{a  b}\n"),
    );
}
