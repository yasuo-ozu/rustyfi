//! Where a parse failure is reported, what it says, and that it arrives at
//! all.
//!
//! Three defects, all of them in what the *compiler* prints:
//!
//! 1. **Mislocated.** `parse_file` took the aggregate error's span, which for
//!    a whole-file rule is the first alternative's — byte 0 for 0.1, the start
//!    of the failing top-level binding for 0.0.6. An error on line 5 was
//!    reported on line 3; a 0.1 library's every error was reported on the
//!    `module` keyword, because a 0.1 library is ONE top-level binding.
//! 2. **A `Debug` dump.** `render_parse_error` was `format!("{err:?}")` over
//!    the whole tree: `Expected { span: Span { start: Loc { line: 3, col: 0,
//!    byte: 33 }, … } }`, four kilobytes of it for a 0.1 file.
//! 3. **Non-terminating.** A chain of `let vN = N in` ending in a `let` with
//!    no right-hand side took 7 ms at 9 lines, 32 ms at 15, and had not
//!    finished after 100 s at 35.
//!
//! The first two are fixed by `AtomStream`'s high-water mark and
//! `parse_error::locate`; the third by `stream::Budget`. The last group in
//! this file is the budget's premise — that no honest file comes near it —
//! measured against the bundled corpus rather than asserted.

use std::path::{Path, PathBuf};

use rustyfi_syntax::stream::{AtomStream, Budget};
use rustyfi_syntax::{ParseFailureKind, ParseFileError};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A 0.0.6 document whose only error is on line 5 (1-based): `let d = in`,
/// a `let` with no right-hand side, four `let`s deep into the body's spine.
fn v006_error_on_line_5() -> &'static str {
    "@require: stdjabook\n\
     let a = 1 in\n\
     let b = 2 in\n\
     let c = 3 in\n\
     let d = in\n\
     document (| title = `t` |) '<\n\
     \x20 +p { x }\n\
     >\n"
}

/// A 0.1 library whose only error is on line 5: `val d = = 4`. The whole file
/// is one `module` binding, which is what used to force every error in it onto
/// line 2.
fn v01_error_on_line_5() -> &'static str {
    "@require: basic\n\
     module M = struct\n\
     \x20 val a = 1\n\
     \x20 val b = 2\n\
     \x20 val d = = 4\n\
     \x20 val e = 5\n\
     end\n"
}

/// A 0.1 library missing its closing `end`: the parse consumes every token
/// there is and then wants one more.
///
/// The one shape in this file whose failure the error TREE locates rather than
/// the high-water mark — the two coincide at the end of the file, and the tree
/// additionally knows *what* was missing. Kept as a named fixture because it
/// is the only cover for `best_failure`'s half of the reducer: with the mark
/// alone, every other fixture here still lands on the right line.
///
/// Finding one took some looking, and the reason is worth recording: the
/// obvious candidates — an unterminated `+p { …`, an unclosed `(`, an unclosed
/// `[` — are all *lexer* failures in this port, because the 0.0.6 lexer is
/// mode-switching and notices the unterminated area itself.
const RUNS_OFF_THE_END_V1: &str = "@require: basic\n\
                                   module M = struct\n\
                                   \x20 val a = 1\n";

/// `n` bindings of `let vN = N in`, then a `let` with no right-hand side —
/// the shape whose backtracking is exponential in `n`.
fn let_chain(n: usize) -> String {
    let mut s = String::from("@require: stdjabook\n");
    for i in 0..n {
        s.push_str(&format!("let v{i} = {i} in\n"));
    }
    s.push_str("let bad = in\n");
    s.push_str("document (| title = `t` |) '<\n  +p { x }\n>\n");
    s
}

fn err_of(src: &str) -> ParseFileError {
    rustyfi_syntax::parse_file(src).expect_err("must not parse")
}

fn err_of_v1(src: &str) -> ParseFileError {
    rustyfi_syntax::parse_file_v1(src).expect_err("must not parse")
}

// ---------------------------------------------------------------------------
// Defect 1 — where the error is reported
// ---------------------------------------------------------------------------

/// The error is four `let`s down the body's spine, and `Vec<TopBinding>` rolls
/// all of them back before failing. Before the high-water mark this reported
/// `line 3, characters 0-3` — the start of the *third* line's `let`, which is
/// perfectly valid code.
#[test]
fn an_error_deep_in_a_0_0_6_document_reports_its_own_line() {
    let e = err_of(v006_error_on_line_5());
    assert_eq!(e.span.start.line, 5, "{e}");
    assert_eq!(e.kind, ParseFailureKind::Syntax, "{e}");
}

/// The same for a 0.1 library, where it is not a matter of degree: the file is
/// one top-level `module` binding, so the aggregate's span was the `module`
/// keyword for *every* error in *every* 0.1 file.
#[test]
fn an_error_in_a_0_1_library_is_not_reported_on_the_module_head() {
    let e = err_of_v1(v01_error_on_line_5());
    assert_eq!(e.span.start.line, 5, "{e}");
    assert_eq!(e.kind, ParseFailureKind::Syntax, "{e}");
}

/// A 0.1 *document* is not one binding, so it exercises the other shape.
#[test]
fn an_error_deep_in_a_0_1_document_reports_its_own_line() {
    let src = "@require: basic\n\
               let a = 1 in\n\
               let b = 2 in\n\
               let c = = 3 in\n\
               a\n";
    let e = err_of_v1(src);
    assert_eq!(e.span.start.line, 4, "{e}");
}

/// The mark must not overshoot either: an error on the *first* construct still
/// reports there, rather than being dragged forward by however far some
/// speculative alternative got.
///
/// This is the control for the test above. A "furthest position reached" rule
/// that counted `peek`s instead of `next`s, or that took the token *after* the
/// mark, would still pass every test that only checks a deep error is not
/// reported shallowly.
#[test]
fn an_error_on_the_first_construct_still_reports_there() {
    let src = "@require: stdjabook\nlet a = ] in\na\n";
    let e = err_of(src);
    assert_eq!(e.span.start.line, 2, "{e}");
    assert_eq!(e.span.start.col, 8, "the `]` itself: {e}");
}

// ---------------------------------------------------------------------------
// Defect 1 — what the error says
// ---------------------------------------------------------------------------

/// No `Debug` dump, in either generation, for any of the shapes above.
///
/// `Loc {` and `Span {` are the two struct names `format!("{err:?}")` emitted;
/// either one appearing means the raw tree is being printed again.
#[test]
fn no_message_contains_a_debug_dump() {
    let mut messages: Vec<String> = vec![
        err_of(v006_error_on_line_5()).to_string(),
        err_of_v1(v01_error_on_line_5()).to_string(),
        err_of("@require: stdjabook\nlet a = ] in\na\n").to_string(),
        err_of("let x = `unterminated").to_string(),
        err_of_v1("module M = struct\n  val a = = 1\nend\n").to_string(),
        // Runs off the end of the file rather than stopping at a token, so
        // this one is located by the error TREE and not by the mark — the
        // other arm of `locate`, and the one that would still print a `Debug`
        // dump if only the mark had been added.
        err_of_v1(RUNS_OFF_THE_END_V1).to_string(),
    ];
    messages.push(gave_up_on(&let_chain(30)).to_string());
    for m in &messages {
        assert!(!m.contains("Loc {"), "Debug dump in: {m}");
        assert!(!m.contains("Span {"), "Debug dump in: {m}");
        assert!(
            m.len() < 400,
            "message is a wall of text ({} B): {m}",
            m.len()
        );
        assert_eq!(m.lines().count(), 1, "message is not one line: {m}");
    }
}

/// The message names what the parser wanted, or quotes what it choked on.
///
/// Without this a diagnostic could satisfy every position assertion above and
/// still say nothing — "parse error" and a line number.
#[test]
fn the_message_says_something_about_the_failure() {
    let e = err_of(v006_error_on_line_5());
    assert!(
        e.message.contains("expected") || e.message.contains("unexpected"),
        "{e}"
    );
    let e = err_of_v1(v01_error_on_line_5());
    assert!(
        e.message.contains("expected") || e.message.contains("unexpected"),
        "{e}"
    );
}

/// When the error tree reached as far as the stream did, its message is
/// preferred, because it names what was missing and the mark cannot.
///
/// This is the other arm of `locate`'s decision, and the only test that covers
/// it: an input that runs off the end of the file rather than stopping at a
/// token it could not use. The mark alone would say "unexpected end of input";
/// the tree says which token would have finished the file.
#[test]
fn a_failure_at_end_of_input_names_the_token_that_would_have_finished_it() {
    let e = err_of_v1(RUNS_OFF_THE_END_V1);
    assert_eq!(e.kind, ParseFailureKind::Syntax, "{e}");
    // Not merely "unexpected end of input", which the mark alone would give:
    // the NAME of the missing token, which only the error tree knows.
    assert_eq!(e.message, "expected 'end'", "{e}");
}

/// A parse that simply runs out — no token to quote, because there is none —
/// says so, rather than pointing wordlessly at the end of the file.
#[test]
fn running_out_of_input_is_reported_as_running_out() {
    let e = err_of("@require: stdjabook\nlet x = 1 in\nlet y =\n");
    assert_eq!(e.kind, ParseFailureKind::Syntax, "{e}");
    assert!(e.message.contains("end of input"), "{e}");
}

// ---------------------------------------------------------------------------
// Lex errors — unchanged, which is the point
// ---------------------------------------------------------------------------

/// Lex failures were never mislocated: they carry a hand-written message and a
/// tight span of their own, and the reducer must pass them through untouched.
///
/// This is the trap this whole change had to avoid, and it is easy to fall
/// into: four attempts to reproduce the span defect accidentally used lex
/// errors and looked fine.
#[test]
fn a_lex_error_keeps_its_own_message_and_span() {
    for (src, line) in [
        ("let x = `unterminated", 1),
        ("@require: stdjabook\nlet x = 1 in\nlet y = `oops\n", 3),
    ] {
        let raw = rustyfi_syntax::lex(src).expect_err("must not lex");
        let e = err_of(src);
        assert_eq!(e.kind, ParseFailureKind::Lex, "{e}");
        assert_eq!(e.span.start.line, line, "{e}");
        // Verbatim: the lexer's own words and its own span, not the reducer's.
        assert_eq!(e.message, raw.msg, "{e}");
        assert_eq!(e.span, raw.span, "{e}");
        assert!(!e.message.is_empty(), "{e}");
    }
}

/// The same source through both generations' entry points, since 0.1 lexes
/// with a different table and used to build its `ParseFileError` by hand.
#[test]
fn a_lex_error_is_a_lex_error_in_0_1_too() {
    let e = err_of_v1("module M = struct\n  val x = `oops\nend\n");
    assert_eq!(e.kind, ParseFailureKind::Lex, "{e}");
    assert_eq!(e.span.start.line, 2, "{e}");
}

// ---------------------------------------------------------------------------
// Defect 2 — the parse terminates
// ---------------------------------------------------------------------------

/// Parse `src`, on its own thread, insisting on an answer within `secs`.
///
/// A plain `#[test]` cannot fail on non-termination — it hangs until the
/// harness is killed, and a CI timeout with no explanation is exactly the
/// failure mode this test exists to prevent. So the parse runs on a thread and
/// the assertion is on the channel. The thread is abandoned rather than
/// joined if it times out; the process exits regardless.
fn parse_within(src: &str, secs: u64) -> ParseFileError {
    let (tx, rx) = std::sync::mpsc::channel();
    let owned = src.to_string();
    std::thread::spawn(move || {
        let _ = tx.send(rustyfi_syntax::parse_file(&owned).err());
    });
    match rx.recv_timeout(std::time::Duration::from_secs(secs)) {
        Ok(Some(e)) => e,
        Ok(None) => panic!("expected a parse failure"),
        Err(_) => panic!("the parse did not terminate within {secs}s"),
    }
}

fn gave_up_on(src: &str) -> ParseFileError {
    parse_within(src, 120)
}

/// The 35-line chain from the bug report: before the budget it was still
/// running after 100 seconds.
///
/// The time bound here is deliberately loose — the assertion that carries the
/// meaning is on the *kind*, which is machine-independent, and the clock is
/// only the backstop for a regression that removes the budget outright.
#[test]
fn a_long_let_chain_terminates_and_says_it_gave_up() {
    let e = parse_within(&let_chain(34), 120);
    assert_eq!(e.kind, ParseFailureKind::GaveUp, "{e}");
    // Reported as a give-up, never as a claim about the source.
    assert!(e.render().starts_with("gave up:"), "{e}");
    assert!(!e.render().contains("parse error"), "{e}");
    // And the position is not a consolation prize: the mark reaches the
    // offending `let bad = in` before the backtracking explodes, so a give-up
    // still names the line the author has to look at. Only the *reason* is
    // less specific than a verdict's.
    assert_eq!(e.span.start.line, 36, "{e}");
}

/// Chains up to the depth the budget can afford reach a real verdict, and the
/// verdict is right. Without this the budget could "fix" the hang by giving up
/// on everything.
///
/// The ceiling is `Budget::FLOOR`, and the cost is ×2 per `let` (see
/// `Budget`), so 15 is a little under it and 16 a little over: this is the
/// deepest chain the floor buys, not a round number. If the floor changes,
/// this list moves with it — but so does nothing else, which is the point of
/// the exponential.
#[test]
fn short_let_chains_still_get_a_real_verdict() {
    for n in [3, 9, 15] {
        let src = let_chain(n);
        let e = parse_within(&src, 60);
        assert_eq!(e.kind, ParseFailureKind::Syntax, "chain of {n}: {e}");
        // `let bad = in` is on the line after the `n` bindings and the header.
        assert_eq!(e.span.start.line, n as u32 + 2, "chain of {n}: {e}");
    }
}

/// A give-up is not a claim that the file is broken, so it must not be
/// reachable by lengthening a file that is *fine*. The budget scales with the
/// input for exactly this reason.
#[test]
fn a_long_valid_let_chain_parses() {
    let mut src = String::from("@require: stdjabook\n");
    for i in 0..400 {
        src.push_str(&format!("let v{i} = {i} in\n"));
    }
    src.push_str("document (| title = `t` |) '<\n  +p { x }\n>\n");
    assert!(rustyfi_syntax::parse_file(&src).is_ok());
}

// ---------------------------------------------------------------------------
// The budget's premise
// ---------------------------------------------------------------------------

/// Serves spent parsing `src` under `v`, and the atom count — the two numbers
/// [`Budget::PER_ATOM`] is the ratio between. `None` if it does not parse,
/// since a failed parse says nothing about what an honest one costs.
fn cost(src: &str, v: rustyfi_syntax::RustyfiVersion) -> Option<(u64, usize)> {
    use syan::parse::Parse;
    let atoms = rustyfi_syntax::lex_with_version(src, v).ok()?;
    let n = atoms.len();
    // Unlimited, so the measurement is of the parse and not of the cap.
    let mut stream = AtomStream::with_budget(atoms, Budget::unlimited());
    let ok = match v {
        rustyfi_syntax::RustyfiVersion::V0_1 => {
            <rustyfi_syntax::cst_v1::FileV1 as Parse<_>>::parse(&mut stream).is_ok()
        }
        _ => <rustyfi_syntax::cst::File as Parse<_>>::parse(&mut stream).is_ok(),
    };
    ok.then(|| (stream.served(), n))
}

fn bundled(sub: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib-rustyfi")
        .join(sub)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if matches!(
            p.extension().and_then(|x| x.to_str()),
            Some("saty" | "satyh" | "satyg")
        ) {
            out.push(p);
        }
    }
}

/// The whole justification for [`Budget::PER_ATOM`], measured rather than
/// asserted: every file in the bundled corpus that parses at all parses well
/// inside the per-atom allowance.
///
/// If this fails, the budget is no longer safe and the number needs raising —
/// but read the worst offender first, because a file that suddenly costs 100×
/// what its neighbours do is a grammar regression, not a reason to widen the
/// cap.
#[test]
fn the_bundled_corpus_stays_far_under_the_per_atom_budget() {
    // Carried per file rather than derived from a position in one list: the
    // two corpora sort into each other (`dist-v01/…` collates before
    // `dist/…`), so an index-based split would silently read every 0.1 package
    // with the 0.0.6 grammar.
    let mut files: Vec<(PathBuf, rustyfi_syntax::RustyfiVersion)> = Vec::new();
    for (sub, v) in [
        ("dist", rustyfi_syntax::RustyfiVersion::V0_0),
        ("dist-v01", rustyfi_syntax::RustyfiVersion::V0_1),
    ] {
        let mut found = Vec::new();
        walk(&bundled(sub), &mut found);
        assert!(
            found.len() > 20,
            "{sub} is missing — is the checkout complete?"
        );
        found.sort();
        files.extend(found.into_iter().map(|p| (p, v)));
    }

    let mut worst = (0f64, String::new(), 0u64, 0usize);
    let mut measured = 0usize;
    for (f, v) in &files {
        let Ok(src) = std::fs::read_to_string(f) else {
            continue;
        };
        let Some((served, atoms)) = cost(&src, *v) else {
            continue;
        };
        measured += 1;
        let ratio = served as f64 / atoms.max(1) as f64;
        if ratio > worst.0 {
            worst = (ratio, f.display().to_string(), served, atoms);
        }
    }

    assert!(measured > 40, "only {measured} bundled files parsed");
    eprintln!(
        "worst serves/atom over {measured} bundled files: {:.1} ({} serves, {} atoms) in {}",
        worst.0, worst.2, worst.3, worst.1
    );
    // A tenth of the allowance, so a corpus that grows a somewhat costlier
    // file does not immediately break the premise.
    let ceiling = Budget::PER_ATOM as f64 / 10.0;
    assert!(
        worst.0 < ceiling,
        "{} costs {:.1} serves/atom, over a tenth of Budget::PER_ATOM ({})",
        worst.1,
        worst.0,
        Budget::PER_ATOM
    );
}

/// The budget is a per-atom allowance, not a ceiling: a bigger file gets a
/// bigger one, which is what keeps a generated file from being refused for
/// being long.
#[test]
fn the_budget_scales_with_the_input() {
    assert_eq!(Budget::for_atoms(0).serves(), Budget::FLOOR);
    let big = Budget::for_atoms(100_000).serves();
    assert_eq!(big, 100_000 * Budget::PER_ATOM);
    assert!(big > Budget::FLOOR);
    // No overflow panic on an absurd input.
    let _ = Budget::for_atoms(usize::MAX);
}
