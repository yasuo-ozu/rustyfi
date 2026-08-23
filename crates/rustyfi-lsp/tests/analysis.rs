//! `analyze` and friends: the protocol-free half of the crate.
//!
//! The `utf16_columns` group is the one that matters most — every other test
//! here passes just as happily against a byte-offset implementation.

use rustyfi_lsp::{analyze, analyze_auto, analyze_detected, Diag, RustyfiVersion, Severity};

/// `(line, character, end_line, end_character)` of a diagnostic, for
/// comparing against a hand-computed position.
fn at(d: &Diag) -> (u32, u32, u32, u32) {
    (d.line, d.character, d.end_line, d.end_character)
}

fn only(diags: &[Diag]) -> &Diag {
    assert_eq!(diags.len(), 1, "expected exactly one diagnostic, got {diags:?}");
    &diags[0]
}

// ---------------------------------------------------------------------------
// Clean documents
// ---------------------------------------------------------------------------

#[test]
fn a_clean_0_0_6_document_has_no_diagnostics() {
    let src = "@require: stdjabook\n\
               let x = 1 in\n\
               document (| title = `Hello`; author = `me` |) '<\n\
                 +p { Hello, world! }\n\
               >\n";
    assert_eq!(analyze(src, RustyfiVersion::V0_0), Vec::new());
    assert_eq!(analyze_auto(src), Vec::new());
}

#[test]
fn a_clean_0_1_library_has_no_diagnostics() {
    let src = "@require: basic\n\
               module M :> sig\n\
                 val double : int -> int\n\
               end = struct\n\
                 val double n = n * 2\n\
               end\n";
    assert_eq!(analyze(src, RustyfiVersion::V0_1), Vec::new());
}

#[test]
fn a_buffer_with_no_tokens_is_not_an_error() {
    // Checked under BOTH generations because they disagree natively — 0.1's
    // `FileV1` requires at least one binding — and the point is that the
    // disagreement does not reach the user.
    for src in ["", "   \n\n\t", "% just a comment\n% and another\n"] {
        for v in RustyfiVersion::supported() {
            assert_eq!(analyze(src, *v), Vec::new(), "{src:?} under {v}");
        }
        assert_eq!(analyze_auto(src), Vec::new(), "{src:?}");
    }
}

// ---------------------------------------------------------------------------
// Parse errors at known positions
// ---------------------------------------------------------------------------

#[test]
fn a_parse_error_lands_on_the_offending_token_in_0_0_6() {
    // Line 2 is `let y = ] in x`; `]` is at character 8.
    let src = "@require: stdjabook\n\
               let x = 1 in\n\
               let y = ] in x\n";
    let diags = analyze(src, RustyfiVersion::V0_0);
    let d = only(&diags);
    assert_eq!(at(d), (2, 8, 2, 9));
    assert_eq!(d.severity, Severity::Error);
    assert!(!d.message.is_empty());
}

#[test]
fn a_parse_error_lands_on_the_offending_token_in_0_1() {
    // Line 3 is `  val b = = 2`; the second `=` is at character 10, where the
    // expression should have started.
    let src = "@require: basic\n\
               module M = struct\n\
               \x20 val a = 1\n\
               \x20 val b = = 2\n\
               end\n";
    let diags = analyze(src, RustyfiVersion::V0_1);
    let d = only(&diags);
    assert_eq!(at(d), (3, 10, 3, 11), "{}", d.message);
    assert_eq!(d.severity, Severity::Error);
}

#[test]
fn an_unterminated_inline_block_is_reported_where_it_runs_out() {
    let src = "@require: stdjabook\n\
               document (| title = `t` |) '<\n\
               \x20 +p { hello\n";
    let d = &analyze(src, RustyfiVersion::V0_0)[0];
    assert_eq!(d.line, 2, "reported on the unterminated line: {d:?}");
    assert!(
        d.message.contains("end of input"),
        "message should say what ran out: {}",
        d.message
    );
}

#[test]
fn every_diagnostic_range_is_non_degenerate_and_ordered() {
    // An editor draws nothing for a zero-width range, and some clients
    // mis-render a reversed one.
    for src in [
        "let x = ] in x",
        "@require:\n",
        "let x = `unterminated",
        "@require: stdjabook\ndocument (| |) '<\n  +p { x\n",
        "module M = struct\n  val a = = 1\nend\n",
    ] {
        for d in analyze_auto(src) {
            let start = (d.line, d.character);
            let end = (d.end_line, d.end_character);
            assert!(start < end, "degenerate or reversed range in {src:?}: {d:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// UTF-16 columns — the case a byte-offset implementation gets wrong
// ---------------------------------------------------------------------------

#[test]
fn utf16_columns_japanese_before_the_error_on_the_same_line() {
    // `let x = ` (8) + the literal ``こんにちは`` (7: two backticks and five
    // kana) + ` in let y = ` (12) puts the offending `]` at UTF-16 column 27.
    // Each kana is three UTF-8 bytes, so the BYTE offset is 37 — a
    // byte-offset implementation would put the squiggle ten columns past the
    // end of the line.
    let src = "let x = `こんにちは` in let y = ] in y";
    assert_eq!(src.find(']'), Some(37), "byte offset, for contrast");
    let diags = analyze(src, RustyfiVersion::V0_0);
    let d = only(&diags);
    assert_eq!(at(d), (0, 27, 0, 28), "{}", d.message);
}

#[test]
fn utf16_columns_japanese_on_earlier_lines_does_not_shift_later_ones() {
    // The bug a "count bytes since the start of the file" implementation has.
    let src = "@require: stdjabook\n\
               let greeting = `こんにちは、世界` in\n\
               let y = ] in y\n";
    let diags = analyze(src, RustyfiVersion::V0_0);
    let d = only(&diags);
    assert_eq!(at(d), (2, 8, 2, 9), "{}", d.message);
}

#[test]
fn utf16_columns_japanese_in_a_0_1_library() {
    // The other generation, where the error is located by the high-water mark
    // rather than by the error tree — a different code path to `LineIndex`.
    let src = "@require: basic\n\
               module M = struct\n\
               \x20 val greeting = `こんにちは、世界`\n\
               \x20 val b = = 2\n\
               end\n";
    let diags = analyze(src, RustyfiVersion::V0_1);
    let d = only(&diags);
    assert_eq!(at(d), (3, 10, 3, 11), "{}", d.message);
}

#[test]
fn utf16_columns_astral_characters_count_as_two() {
    // An emoji is one `char` but two UTF-16 code units, which separates a
    // correct implementation from a `char`-counting one — `Loc::col` being
    // exactly that, and why this crate re-derives columns from `Loc::byte`.
    let emoji = "let x = `🎉` in let y = ] in y";
    let ascii = "let x = `a` in let y = ] in y";
    let emoji_col = only(&analyze(emoji, RustyfiVersion::V0_0)).character;
    let ascii_col = only(&analyze(ascii, RustyfiVersion::V0_0)).character;
    assert_eq!(ascii_col, 23);
    assert_eq!(
        emoji_col, 24,
        "🎉 occupies two UTF-16 units where `a` occupies one, so the column \
         moves by exactly one; a char-counting implementation reports {ascii_col}"
    );
}

// ---------------------------------------------------------------------------
// Version selection
// ---------------------------------------------------------------------------

#[test]
fn a_0_1_file_read_with_the_0_0_grammar_is_a_screenful_of_nonsense() {
    // The failure mode `detect_version` exists to prevent, asserted directly
    // so that a regression in the detector fails a test rather than quietly
    // costing quality.
    let src = "@require: basic\n\
               module M :> sig\n\
                 val double : int -> int\n\
               end = struct\n\
                 val double n = n * 2\n\
               end\n";
    assert!(
        !analyze(src, RustyfiVersion::V0_0).is_empty(),
        "this file genuinely does not parse as 0.0.6"
    );
    assert!(
        analyze_auto(src).is_empty(),
        "...and auto-detection must not report that"
    );
}

#[test]
fn a_decisive_version_signal_is_obeyed_even_when_the_file_is_broken() {
    // `use Foo` pins 0.1, and the file is broken under both grammars.
    let (v, diags) = analyze_detected("use Foo\nval x = = 1\n");
    assert_eq!(v, RustyfiVersion::V0_1);
    assert!(!diags.is_empty());

    let (v, _) = analyze_detected("@stage: 0\nlet x = ] in x\n");
    assert_eq!(v, RustyfiVersion::V0_0);
}

#[test]
fn an_ambiguous_broken_file_is_reported_under_whichever_grammar_got_further() {
    // `module M = struct … end` is no version signal at all, so the sniffer
    // returns `None` and the 0.0.6 default is tried first. It dies at the
    // `module` head; the 0.1 reading reaches the real typo on line 3. The
    // 0.1 reading is the useful one and must win.
    let src = "@require: basic\n\
               module M = struct\n\
               \x20 val a = 1\n\
               \x20 val b = = 2\n\
               end\n";
    let (v, diags) = analyze_detected(src);
    assert_eq!(v, RustyfiVersion::V0_1);
    assert_eq!(at(only(&diags)), (3, 10, 3, 11));
}

#[test]
fn analyze_takes_its_argument_literally_with_no_fallback() {
    // A hidden retry under the other generation would make the result depend
    // on something the caller cannot see.
    let src = "@require: basic\nmodule M = struct\n  val a = 1\nend\n";
    assert!(analyze(src, RustyfiVersion::V0_1).is_empty());
    assert!(
        !analyze(src, RustyfiVersion::V0_0).is_empty(),
        "0.0.6 was asked for, so 0.0.6's verdict is what comes back"
    );
}

// ---------------------------------------------------------------------------
// Known-good real files
// ---------------------------------------------------------------------------

/// Both vendored package corpora: 0.0.6 in `dist/`, 0.1 in `dist-v01/`.
///
/// These are the port's own shipped libraries and they compile, so **any**
/// diagnostic here is a false positive — the failure mode that makes a
/// language server worse than no language server. It is also what catches a
/// broken ambiguity re-check, which would have most of `dist-v01` read with
/// the 0.0.6 grammar and lighting up.
#[test]
fn the_vendored_corpora_produce_no_diagnostics() {
    sweep("/../../lib-rustyfi", 70);
}

/// The `rustyfi` crate's own document fixtures, which its test suite compiles
/// end to end, so again any diagnostic is a false positive. Kept alongside the
/// corpora because they are `.saty` *documents* and exercise lexer modes the
/// library packages never enter.
#[test]
fn the_document_fixtures_produce_no_diagnostics() {
    sweep("/../../crates/rustyfi/tests/fixtures", 40);
}

/// Analyse every SATySFi source file under `rel` (relative to this crate) and
/// fail on any diagnostic at all, naming every one.
///
/// `floor` is a minimum file count, so that a filter matching nothing cannot
/// make the sweep pass vacuously.
fn sweep(rel: &str, floor: usize) {
    let root = format!("{}{rel}", env!("CARGO_MANIFEST_DIR"));
    let mut checked = 0usize;
    let mut complaints = Vec::new();
    visit(std::path::Path::new(&root), &mut |path, src| {
        checked += 1;
        let (version, diags) = analyze_detected(src);
        for d in diags {
            complaints.push(format!(
                "{} [as {version}] line {}, char {}: {}",
                path.display(),
                d.line + 1,
                d.character,
                d.message
            ));
        }
    });
    assert!(
        complaints.is_empty(),
        "false positives on {} files that compile:\n{}",
        complaints.len(),
        complaints.join("\n")
    );
    assert!(
        checked >= floor,
        "expected at least {floor} files under {root}, checked only {checked}"
    );
}

/// Call `f` for every SATySFi source file under `dir`, recursively.
fn visit(dir: &std::path::Path, f: &mut impl FnMut(&std::path::Path, &str)) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            visit(&path, f);
            continue;
        }
        if !matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("saty" | "satyh" | "satyg")
        ) {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        f(&path, &src);
    }
}

// ---------------------------------------------------------------------------
// The backtracking budget
// ---------------------------------------------------------------------------

/// The vendored 0.1 library whose truncations trigger the parser's
/// exponential backtracking.
fn pathological_prefix() -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../lib-rustyfi/dist-v01/packages/std-ja.satyh"
    );
    let src = std::fs::read_to_string(path).expect("the vendored 0.1 corpus must be present");
    src[..14_223.min(src.len())].to_string()
}

/// A truncated 0.1 library — an ordinary mid-typing state — must not hang.
///
/// The 0.1 grammar backtracks exponentially on some incomplete inputs, and
/// this prefix takes over ten seconds without `HighWaterStream`'s budget.
///
/// The assertion is on the *message*, not on a stopwatch, so it means the same
/// thing on every machine: reaching the cap must be reported as reaching the
/// cap, not as a claim about the token the parse stopped at. The loose time
/// bound is only a backstop against a regression that removes the budget
/// outright, which would otherwise surface as an unexplained CI timeout.
///
/// If the vendored `std-ja.satyh` is edited and this prefix stops exhausting,
/// re-find one that does (sweep `(200..src.len()).step_by(197)` and time
/// each); do not delete the test.
#[test]
fn a_pathological_prefix_gives_up_instead_of_hanging() {
    let src = pathological_prefix();
    let started = std::time::Instant::now();
    let diags = analyze(&src, RustyfiVersion::V0_1);
    let elapsed = started.elapsed();

    let d = only(&diags);
    assert!(
        d.message.contains("gave up"),
        "expected the budget to stop the parse, got: {}",
        d.message
    );
    assert!(
        elapsed < std::time::Duration::from_secs(30),
        "the budget did not bound the parse: {elapsed:?}"
    );
}

/// The same buffer through the auto-detecting entry point, which is what the
/// server actually calls: the cross-generation re-check must not turn one
/// bounded parse into an unbounded one.
#[test]
fn the_version_recheck_is_bounded_too() {
    let src = pathological_prefix();
    let started = std::time::Instant::now();
    let _ = analyze_auto(&src);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(60),
        "both generations together must still be bounded"
    );
}
