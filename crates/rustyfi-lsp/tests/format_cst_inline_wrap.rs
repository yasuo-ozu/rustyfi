//! Slice 6: **gap-level re-wrapping of inline text** — `CstOptions::wrap_inline_text`.
//!
//! The corpus sweep in `format_cst_slice1.rs` holds the five properties with
//! this feature on, and its property 1 carries the relaxation that licenses it
//! (`Space` <-> `Break`, at a gap the predicate clears, and nothing else). What
//! is here is what a sweep over 209 files cannot say:
//!
//! - the **shapes** the predicate turns on, each pinned against its control, so
//!   a broken predicate fails a named test rather than a whole corpus;
//! - **idempotence at the boundary width** — exactly at the budget, one column
//!   under and one over — which is where a greedy fill stops being a fixpoint
//!   and which a sweep at one fixed width never visits;
//! - the **user's own example**, at two widths, because "230 columns comes back
//!   unchanged" was the report and this is the answer to it.
//!
//! Two things this file deliberately does NOT own. The measurement that
//! licenses the predicate is `docs/plans/formatter-cst/README.md`'s rule 3 and
//! is re-verified end to end, against real compiles and real PDF bytes, by
//! `crates/rustyfi/tests/ws_inline_rewrap.rs` — a formatter test cannot check
//! that a whitespace edit is invisible to a typesetter. And the classifier's
//! ranges are pinned against `rustyfi-backend`'s own `char_script` there too,
//! for the same reason.

use rustyfi_lsp::{format_cst, CstOptions, RustyfiVersion};

fn at(max_width: usize) -> CstOptions {
    CstOptions {
        max_width,
        ..CstOptions::default()
    }
}

fn off() -> CstOptions {
    CstOptions {
        wrap_inline_text: false,
        ..CstOptions::default()
    }
}

fn fmt(src: &str, opts: &CstOptions) -> String {
    format_cst(src, RustyfiVersion::V0_0, opts)
        .unwrap_or_else(|| panic!("DECLINED, which is how a broken printer hides: {src:?}"))
}

fn fmt01(src: &str, opts: &CstOptions) -> String {
    format_cst(src, RustyfiVersion::V0_1, opts)
        .unwrap_or_else(|| panic!("DECLINED (0.1): {src:?}"))
}

/// Format, then format the output, and return the first — panicking if the two
/// differ. Every expectation in this file goes through it, because a greedy
/// fill that is right once and different the second time is the failure mode
/// the whole design is arranged around.
fn stable(src: &str, opts: &CstOptions) -> String {
    let once = fmt(src, opts);
    let twice = fmt(&once, opts);
    assert_eq!(
        twice, once,
        "not idempotent at width {}\n  in   : {src:?}\n  once : {once:?}\n  twice: {twice:?}",
        opts.max_width
    );
    once
}

// ---------------------------------------------------------------------------
// the shape the corpus caught and the fixtures did not
// ---------------------------------------------------------------------------

/// **`layout-tests/corpus/azmath/doc/azmath.saty`, token 523**, reduced.
///
/// The corpus sweep caught a `Break` becoming a `Space` here before any
/// fixture did, and the first question was whether it was a predicate bug: a
/// join between two CJK characters is exactly the meaning change this feature
/// exists to prevent. It is not one. The source is
///
/// ```text
///     … を用いて別行立て数式を記述します。
///     \SATySFi; は\LaTeX;と異なり、…
/// ```
///
/// and the codepoint after the gap is not `\` and not `S`: it is **nothing at
/// all**, because `\SATySFi` lexes to `Token::HorzCmd` and a command ENDS the
/// elaborated text run (`README.md` rule 3). One side of the gap is CJK and
/// the other is a run boundary, so the gap is absorbed and the join is free.
///
/// Which is the finding an area-level rule would have thrown away: this
/// paragraph is 100% Japanese and every one of its gaps that abuts a command,
/// an inline `${…}` or a `` `literal` `` re-wraps.
#[test]
fn a_cjk_paragraph_re_wraps_at_the_gaps_that_abut_a_command() {
    let src = "let t = {を用いて別行立て数式を記述します。\n\\SATySFi;と異なり} in t\n";
    let want = "let t = {を用いて別行立て数式を記述します。 \\SATySFi;と異なり}\nin\nt\n";
    assert_eq!(stable(src, &at(100)), want, "a command edge did not absorb the gap");
    // The CONTROL, and it is one character different: put a CJK character
    // where the command was and the same gap freezes at any width. Note the
    // continuation line is still RE-INDENTED to the area's depth — that is
    // slice 4, and rule 1 of the measurement says indentation is free
    // everywhere (123 of 123). Frozen means "the break stays a break", not
    // "the bytes are copied".
    let control = "let t = {を用いて別行立て数式を記述します。\nそして異なり} in t\n";
    let frozen = "let t = {を用いて別行立て数式を記述します。\n  そして異なり}\nin\nt\n";
    assert_eq!(stable(control, &at(200)), frozen, "a CJK/CJK gap was joined");
    assert_eq!(stable(frozen, &at(200)), frozen, "a CJK/CJK gap was joined");
    // The same three run-enders, each in the same position.
    for (src, want, what) in [
        (
            "let t = {記述します。\n${x + y}と異なり} in t\n",
            "let t = {記述します。 ${x + y}と異なり}\nin\nt\n",
            "an inline math escape",
        ),
        (
            "let t = {記述します。\n`code`と異なり} in t\n",
            "let t = {記述します。 `code`と異なり}\nin\nt\n",
            "a backtick literal",
        ),
        (
            "let a = {x} in let t = {記述します。\n#a;と異なり} in t\n",
            "let a = {x}\nin\nlet t = {記述します。 #a;と異なり} in\nt\n",
            "a `#var;` embed",
        ),
        (
            // A group EDGE, reached the only way inline text has one: the
            // `}` that closes a command's argument. (A bare `{ … }` inside
            // horizontal mode is not inline-text syntax — `InlineElem` has no
            // nested-group variant — so there is no other shape to test. The
            // run in FRONT of any `{` is not a whitespace token at all: the
            // lexer swallows it into `BHorzGrp`, `lexer.rs:1112-1147`.)
            "let t = {記述し\\emph{ます}\nと異なり} in t\n",
            "let t = {記述し\\emph{ます} と異なり}\nin\nt\n",
            "a command argument's closing group edge",
        ),
    ] {
        assert_eq!(stable(src, &at(100)), want, "{what} did not end the run");
    }
}

/// The rest of rule 3's counterexamples: characters that are measurably unsafe
/// and are neither Han nor Kana, so a `Script`-based test would re-wrap them.
///
/// Each was measured at 3.96 pt of displaced ink. `Ａ` U+FF21 is the sharpest —
/// its Unicode Script is **Latin**.
#[test]
fn the_non_han_non_kana_characters_freeze_too() {
    for (left, right, what) in [
        ("あ、", "「か」", "an ideographic comma and a corner bracket"),
        ("語。", "々次", "an ideographic full stop and the iteration mark"),
        ("あ：", "！か", "fullwidth colon and exclamation mark"),
        ("（あ）", "・か", "fullwidth parentheses and the katakana middle dot"),
        ("あ\u{3000}", "\u{3000}か", "the ideographic SPACE, U+3000, which is a `Zs`"),
        ("Ａ", "Ｂ", "FULLWIDTH LATIN A and B, whose Script is Latin"),
    ] {
        // The continuation line is re-indented (slice 4, rule 1: indentation
        // is free everywhere); what must not happen is the BREAK going away.
        let src = format!("let t = {{{left}\n  {right}}}\nin\nt\n");
        assert_eq!(stable(&src, &at(200)), src, "{what}: joined a frozen gap");
        // And the other direction: a space between the same two must not
        // become a break, however narrow the budget.
        let flat = format!("let t = {{{left} {right}}}\nin\nt\n");
        assert_eq!(stable(&flat, &at(8)), flat, "{what}: split a frozen gap");
    }
    // The control, so the fixture above is not passing because nothing ever
    // wraps: swap in two Latin letters and both directions move.
    let src = "let t = {ab\ncd}\nin\nt\n";
    assert_eq!(stable(src, &at(200)), "let t = {ab cd}\nin\nt\n");
    assert_eq!(stable("let t = {ab cd}\nin\nt\n", &at(11)), "let t = {ab\n  cd}\nin\nt\n");
    // Hangul, Thai, Lao, Greek, Cyrillic and emoji are SAFE — this port routes
    // them through `OtherScript`, so the UAX#14 framing does not apply.
    for (s, what) in [
        ("한국", "hangul"),
        ("ไทย", "thai"),
        ("ລາວ", "lao"),
        ("αβγ", "greek"),
        ("Жук", "cyrillic"),
        ("🙂🙂", "emoji"),
    ] {
        let src = format!("let t = {{{s}\n{s}}}\nin\nt\n");
        let want = format!("let t = {{{s} {s}}}\nin\nt\n");
        assert_eq!(stable(&src, &at(200)), want, "{what} must be safe");
    }
}

/// The escaped-space veto: `\ ` joins the run, so the run's last SIGNIFICANT
/// character decides — and the veto is looked THROUGH rather than taken at
/// face value.
///
/// `\ ` lexes to `Char(" ")`, so the character immediately before the gap is a
/// literal space and the obvious reading makes the gap Latin-adjacent and
/// free. It is not: `ws_inline_rewrap.rs`'s R19 compiles both spellings and
/// they DIFFER. That case is what corrected the implementation — it was
/// written the obvious way first, and the fixture below asserted the wrong
/// answer until a compile said so.
#[test]
fn an_escaped_space_is_looked_through_to_the_character_behind_it() {
    // Frozen: `本` decides, not the escape.
    let src = "let t = {日本\\ \n  語です}\nin\nt\n";
    assert_eq!(stable(src, &at(200)), src, "joined a gap behind an escaped space");
    // Without the escape, the same two sides freeze too — so the escape
    // licenses nothing rather than changing the answer.
    let control = "let t = {日本\n  語です}\nin\nt\n";
    assert_eq!(stable(control, &at(200)), control);
    // And it is not a blanket freeze: with only ONE side CJK the gap is free,
    // in both arrangements (R20 and R21).
    assert_eq!(
        stable("let t = {alpha\\ \n語です} in t\n", &at(100)),
        "let t = {alpha\\  語です}\nin\nt\n"
    );
    assert_eq!(
        stable("let t = {日本\\ \nalpha beta} in t\n", &at(100)),
        "let t = {日本\\  alpha beta}\nin\nt\n"
    );
}

/// The two freezes that are about the run's own bytes rather than its
/// neighbours: a `%` comment inside it, and a blank line inside it.
#[test]
fn a_comment_or_a_blank_line_inside_a_run_freezes_it_whatever_the_script() {
    // The comment lives INSIDE the `Space` token's span, so filling the gap
    // would delete it. (Its leading run still collapses to one space, which is
    // slice 4's `keep_first_space` and not slice 6.)
    let src = "let t = {alpha  % note\n  beta gamma} in t\n";
    let out = stable(src, &at(200));
    assert!(out.contains("% note\n"), "the comment moved or vanished: {out:?}");
    assert!(!out.contains("alpha % note beta"), "the gap was filled: {out:?}");
    // A blank line is one `Break` token, so the typesetter cannot tell it from
    // a single newline — but the author wrote a paragraph break and no width
    // budget asked for it to go.
    let blank = "let t = {alpha\n\n  beta gamma} in t\n";
    let out = stable(blank, &at(200));
    assert!(out.contains("alpha\n\n"), "a blank line inside a run was filled: {out:?}");
}

// ---------------------------------------------------------------------------
// idempotence at the boundary, which is what a greedy fill breaks first
// ---------------------------------------------------------------------------

/// One column under the budget, exactly at it, and one over — in Latin, in
/// CJK, and in mixed text where a frozen gap sits mid-paragraph.
///
/// *Exactly at* is the interesting one, and it is the same trap the comment
/// wrapper documents: a fill that compares `>=` rather than `>` breaks a line
/// that fits, the broken line then fits too, and idempotence testing at a
/// random width never sees it. Here it shows up as a second pass that differs.
///
/// The mixed case is the one this feature adds: a frozen gap next to a
/// reflowed one is where "the second pass makes the same decisions" is least
/// obvious, because the frozen gap's bytes are read from the input while the
/// reflowed one's are computed.
#[test]
fn the_fill_is_a_fixpoint_at_the_boundary_width() {
    for (body, what) in [
        ("alpha beta gamma delta epsilon zeta eta theta iota kappa", "latin"),
        (
            "日本語 と英語 が混ざる 文章 です alpha beta gamma delta epsilon",
            "cjk words separated by gaps that each abut a run boundary",
        ),
        (
            "alpha beta 日本語\nです gamma delta epsilon zeta eta theta iota",
            "mixed, with a FROZEN gap mid-paragraph",
        ),
        (
            "alpha \\emph{beta} gamma ${x + y} delta `lit` epsilon zeta eta",
            "mixed with commands, math and a literal",
        ),
    ] {
        let src = format!("let t = {{\n  {body}\n}}\nin\nt\n");
        // Sweep every width in a band around the natural one, so "exactly at"
        // is hit for every one of the line's break points rather than for one
        // chosen by hand.
        for max_width in 8usize..=80 {
            let opts = at(max_width);
            let once = fmt(&src, &opts);
            let twice = fmt(&once, &opts);
            assert_eq!(
                twice, once,
                "{what}: not a fixpoint at width {max_width}\n  once : {once:?}\n  twice: {twice:?}"
            );
            // And a THIRD pass, because a two-cycle would pass the check above
            // only if the cycle length divides one — it does not, but the
            // cheap way to say so is to look.
            let thrice = fmt(&twice, &opts);
            assert_eq!(thrice, twice, "{what}: a two-cycle at width {max_width}");
        }
    }
}

/// The budget is EXACT: a line that fills it to the last column stays whole,
/// and one column less breaks it.
///
/// A mutation that spends `max_width - col - 2` instead of
/// `max_width - col - 1` — the classic fence-post — survived every other test
/// in this file and both corpus sweeps, because breaking one column EARLY
/// still satisfies "no line exceeds the budget", is still idempotent, and
/// still changes no token. The only thing that catches it is an expectation
/// about where the break lands, so this test asserts the fill is right up
/// against the fence from both sides.
///
/// `  alpha beta gamma` is 18 columns. At 18 it fits and must not break; at 17
/// it must. An off-by-one in either direction fails one of the two.
#[test]
fn the_fill_uses_every_column_of_the_budget_and_not_one_more() {
    let src = "let t = {\n  alpha beta gamma\n}\nin\nt\n";
    let whole = "let t = {\n  alpha beta gamma\n}\nin\nt\n";
    let broken = "let t = {\n  alpha beta\n  gamma\n}\nin\nt\n";
    assert_eq!(
        stable(src, &at(18)),
        whole,
        "an 18-column line broke at a budget of 18 — the fill is spending one \
         column too few"
    );
    assert_eq!(
        stable(src, &at(17)),
        broken,
        "an 18-column line survived a budget of 17 — the fill is spending one \
         column too many"
    );
    // The same fence one word further along, so the answer is not an accident
    // of this one line's width. `  alpha beta` is 12 columns.
    let two = "let t = {\n  alpha beta\n}\nin\nt\n";
    assert_eq!(stable(two, &at(12)), two, "a 12-column line broke at a budget of 12");
    assert_eq!(
        stable(two, &at(11)),
        "let t = {\n  alpha\n  beta\n}\nin\nt\n",
        "a 12-column line survived a budget of 11"
    );
    // And in CJK, where every character is two columns and a fence-post error
    // is twice as visible. `  ab 日本` is 2 + 2 + 1 + 4 = 9 columns.
    let cjk = "let t = {\n  ab 日本\n}\nin\nt\n";
    assert_eq!(stable(cjk, &at(9)), cjk, "a 9-column CJK line broke at a budget of 9");
    assert_eq!(
        stable(cjk, &at(8)),
        "let t = {\n  ab\n  日本\n}\nin\nt\n",
        "a 9-column CJK line survived a budget of 8"
    );
}

/// The mixed case in full, spelled out — because "it is a fixpoint" says
/// nothing about whether the frozen gap stayed frozen.
#[test]
fn a_frozen_gap_mid_paragraph_stays_exactly_where_the_author_put_it() {
    let src = "let t = {\n  alpha beta 日本語\n  です gamma delta epsilon zeta eta theta\n} in t\n";
    for max_width in [20usize, 40, 60, 200] {
        let out = stable(src, &at(max_width));
        // The frozen gap is a newline, and it is still a newline: the two CJK
        // characters are never on the same line.
        assert!(
            out.contains("日本語\n") && out.contains("です"),
            "the CJK gap was joined at width {max_width}: {out:?}"
        );
        for l in out.lines() {
            assert!(
                !(l.contains("日本語") && l.contains("です")),
                "at width {max_width}, a line carries both sides of a frozen gap: {l:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// the user's example
// ---------------------------------------------------------------------------

/// The report: a 230-column `+p { … }` comes back unchanged at any
/// `max_width`, because inline text got re-INDENTATION only.
#[test]
fn the_users_230_column_paragraph_wraps_at_100_and_at_60() {
    let src = "@require: stdjabook\ndocument (| title = {T}; author = {A}; show-title = true; \
               show-toc = false |) '<\n  +p {\n    Hello from WebAssembly. This document was \
               typeset in your browser, by the same Rust code the command-line rustyfi runs, \
               and everything below is one ordinary SATySFi document rather than a feature \
               demonstration bolted together.\n  }\n>\n";
    // The regression this feature is for: with the key off, the 230-column
    // line comes back at 230 columns.
    let unwrapped = fmt(src, &off());
    assert!(
        unwrapped.lines().any(| l| l.chars().count() > 200),
        "the control does not reproduce the report, so this test proves nothing"
    );
    for max_width in [100usize, 60] {
        let out = stable(src, &at(max_width));
        // The prose lines only. The `document (| … |) '<` line is program
        // text, is 78 columns wide, and slice 3 is what would break it —
        // measuring it here would make this a test of a slice that has not
        // landed.
        let widest = out
            .lines()
            .filter(| l| l.starts_with("    ") && !l.contains('{') && !l.contains('}'))
            .map(| l| l.chars().count())
            .max()
            .unwrap_or(0);
        assert!(widest > 0, "no prose line was found at all:\n{out}");
        assert!(
            widest <= max_width,
            "a prose line is {widest} columns at a budget of {max_width}:\n{out}"
        );
        // Every continuation line takes the AREA's indentation, which is
        // slice 4's, and the `+p {` / `}` frame is untouched.
        assert!(out.contains("\n  +p {\n"), "{out}");
        assert!(out.contains("\n  }\n"), "{out}");
        for l in out.lines().filter(| l| l.contains("browser") || l.contains("together")) {
            assert!(l.starts_with("    "), "continuation line not at the area's indent: {l:?}");
        }
    }
    // Exact bytes at 100, so the shape is pinned and not just the width.
    assert_eq!(
        stable(src, &at(100)),
        "@require: stdjabook\ndocument (| title = {T}; author = {A}; show-title = true; \
         show-toc = false |) '<\n  +p {\n    Hello from WebAssembly. This document was typeset \
         in your browser, by the same Rust code the\n    command-line rustyfi runs, and \
         everything below is one ordinary SATySFi document rather than a\n    feature \
         demonstration bolted together.\n  }\n>\n"
    );
}

// ---------------------------------------------------------------------------
// the key, and the other grammar
// ---------------------------------------------------------------------------

/// The key controls the feature, and the default has it on.
#[test]
fn the_key_is_what_does_it_and_the_default_is_on() {
    let src = "let t = {\n  alpha\n  beta\n  gamma\n}\nin\nt\n";
    assert_eq!(fmt(src, &CstOptions::default()), "let t = {\n  alpha beta gamma\n}\nin\nt\n");
    assert_eq!(fmt(src, &off()), src, "`wrap_inline_text: false` must be inert");
    assert!(CstOptions::default().wrap_inline_text, "the default is on");
}

/// The 0.1 grammar runs the same predicate through the same module.
///
/// The measurement behind rule 3 was taken on the **0.0.6** path only, which is
/// recorded here rather than in a commit message because it is the reason this
/// test exists: the 0.1 lexer and elaborator are a separate road to the same
/// tokens, so the behaviour is pinned rather than inferred.
/// `crates/rustyfi/tests/ws_inline_rewrap.rs` closes the measurement gap by
/// compiling 0.1 fixtures end to end.
#[test]
fn the_0_1_grammar_wraps_and_freezes_by_the_same_rule() {
    let lib = |body: &str| format!("module M = struct\n{body}end\n");
    let src = lib("  val t = {\n    alpha\n    beta\n    gamma\n  }\n");
    assert_eq!(
        fmt01(&src, &CstOptions::default()),
        lib("  val t = {\n    alpha beta gamma\n  }\n")
    );
    // Frozen, at a width that would join anything it could.
    let cjk = lib("  val t = {\n    日本語の文章を\n    書きます\n  }\n");
    assert_eq!(fmt01(&cjk, &at(200)), cjk, "0.1 joined a frozen gap");
    // And the command edge, which is the azmath shape on the other grammar.
    let cmd = lib("  val t = {\n    記述します。\n    \\emph{と異なり}\n  }\n");
    assert_eq!(
        fmt01(&cmd, &CstOptions::default()),
        lib("  val t = {\n    記述します。 \\emph{と異なり}\n  }\n")
    );
    // Idempotence on the 0.1 path too, across the same band of widths.
    for max_width in 8usize..=60 {
        let opts = at(max_width);
        let once = fmt01(&src, &opts);
        assert_eq!(fmt01(&once, &opts), once, "0.1 not a fixpoint at width {max_width}");
    }
}
