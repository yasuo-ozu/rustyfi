//! Comment reflow: `CstOptions::wrap_comments`.
//!
//! # What the option is, and why its default is `false`
//!
//! `lexer.rs:308-316` discards a `%` comment through end of line in every lexer
//! mode and there is no `Token::Comment`, so reflowing one **cannot** change
//! the token stream — provably, not by measurement. What it can do is invent a
//! `%`, and this corpus keeps whole blocks of commented-out code that a wrap
//! would rewrite into something that no longer uncomments cleanly.
//!
//! The census over `lib-rustyfi/dist/packages` + `layout-tests/corpus`
//! (`format_cst::comment`'s module documentation carries it in full):
//!
//! ```text
//!   %-comments in program-area gaps                     2474
//!   ...on a line over the 100-column budget               30   (1.2%)
//!   ...that reflow would bring under it                   23   (0.93%)
//!   ...that are own-line AND pass `comment::is_prose`      3   (0.12%)
//! ```
//!
//! Three in 2474 — **two**, since inline text got a policy. The third sat
//! inside an inline text area (`easytable.saty`'s `}% これは …`), which the
//! census counted as a program-area gap because it really is one: a comment
//! abutting a token in horizontal mode is skipped by `lex_horizontal`'s own
//! `'%'` arm (`lexer.rs:1106-1109`), which re-takes `start`. Reflowing it there
//! is token-safe and is still rewriting somebody's prose, so
//! `build006.rs`'s `Build::inline_depth` refuses — a rule now rather than an
//! accident of the area being opaque. So the option ships **off**, and this
//! file's job is to prove two different things about the two settings:
//!
//! - **off** — nothing whatsoever changes. That is the existing sweeps'
//!   business (`format_cst_slice1.rs`, `format_cst_slice1_v01.rs`), which run
//!   on `CstOptions::default()` and are untouched by this feature.
//! - **on** — the five sweep properties still hold, with property 4 restated
//!   rather than dropped (see [`assert_reflow_only`]), and reflow is a fixpoint
//!   at the boundary width, one column under it and one column over.
//!
//! # Property 4 is the one that has to change, and how
//!
//! `format_cst_slice1.rs`'s property 4 is "the number of content-bearing lines
//! is unchanged, and each is `squeeze`-equal". Reflow **adds** lines, by
//! construction, so that statement is not merely unprovable here — it is false.
//! Dropping it would give up the only check that can see a token migrate across
//! a line boundary, so it is replaced by a strictly stronger one:
//!
//! > the wrap-on output is the wrap-off output with some own-line comment lines
//! > replaced by two or more lines carrying the **same indentation and the same
//! > marker**, whose bodies rejoin — with single spaces — to the original body.
//!
//! That is an exact inverse, and it is what pins the three rules the feature
//! must not break. Two comments joined into one would consume two wrap-off
//! lines for one wrap-on line and fail the alignment; a comment moved across a
//! token would move a non-comment line and fail it; a trailing comment
//! rewritten would change a line that is not an own-line comment at all.

use std::path::{Path, PathBuf};

use rustyfi_lsp::{cst_walk_desync, format_cst, CstOptions, RustyfiVersion};

// ---------------------------------------------------------------------------
// corpus discovery — the same three roots the slice-1 sweeps use
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

/// Comment reflow ON, inline re-wrap OFF.
///
/// **Both flags are now `true` by default**, which turned this file's
/// wrap-on/wrap-off contrast into a comparison of two identical
/// configurations — a test that stops distinguishing rather than failing. So
/// the two option sets here are explicit about BOTH keys: this file is about
/// `wrap_comments` and nothing else, and slice 6 is held OFF on both sides so
/// that a text area moving cannot be mistaken for a comment moving. Slice 6's
/// own sweep is `tests/format_cst_inline_wrap.rs`.
fn wrapping() -> CstOptions {
    CstOptions {
        wrap_comments: true,
        wrap_inline_text: false,
        ..CstOptions::default()
    }
}

/// The baseline: both wraps OFF. **Not `CstOptions::default()` any more** —
/// see [`wrapping`].
fn not_wrapping() -> CstOptions {
    CstOptions {
        wrap_comments: false,
        wrap_inline_text: false,
        ..CstOptions::default()
    }
}

// ---------------------------------------------------------------------------
// the restated property 4
// ---------------------------------------------------------------------------

/// The marker (`%`s plus at most one space) and body of a line that is nothing
/// but an own-line comment, or `None` for any other line.
fn own_line_comment(line: &str) -> Option<(&str, &str, &str)> {
    let body = line.trim_start_matches([' ', '\t']);
    let indent = &line[..line.len() - body.len()];
    if !body.starts_with('%') {
        return None;
    }
    let hashes = body.len() - body.trim_start_matches('%').len();
    let (marker, rest) = match body[hashes..].starts_with(' ') {
        true => body.split_at(hashes + 1),
        false => body.split_at(hashes),
    };
    Some((indent, marker, rest))
}

/// The property this file replaces property 4 with, stated in the module
/// header. Returns how many comments were reflowed.
///
/// It is an alignment rather than a summary, which is the point: a summary
/// (total comment bytes, comment-line count) is satisfied by joining two
/// comments as readily as by splitting one, and "never join two comments" is a
/// rule the task states outright.
fn assert_reflow_only(off: &str, on: &str, what: &str) -> usize {
    let a: Vec<&str> = off.lines().collect();
    let b: Vec<&str> = on.lines().collect();
    let (mut i, mut j, mut reflowed) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            i += 1;
            j += 1;
            continue;
        }
        // The only licensed difference: one own-line comment became several.
        let Some((indent, marker, body)) = own_line_comment(a[i]) else {
            panic!(
                "{what}: wrap-on changed a line that is not an own-line comment\n\
                  off[{i}]: {:?}\n   on[{j}]: {:?}",
                a[i], b[j]
            );
        };
        let mut parts: Vec<&str> = Vec::new();
        while j < b.len() {
            let Some((bi, bm, bb)) = own_line_comment(b[j]) else {
                break;
            };
            // Same indentation and the same marker, or it is a different
            // comment rather than a continuation of this one.
            if bi != indent || bm != marker {
                break;
            }
            parts.push(bb);
            j += 1;
            // Stop as soon as the bodies account for the original: anything
            // more would be swallowing the NEXT comment, which is the join
            // this check exists to forbid.
            if parts.join(" ") == body {
                break;
            }
        }
        assert!(
            parts.len() >= 2,
            "{what}: off[{i}] {:?} was rewritten but not into a reflow of itself \
             ({} continuation line(s))",
            a[i],
            parts.len()
        );
        assert_eq!(
            parts.join(" "),
            body,
            "{what}: the reflow of off[{i}] does not rejoin to the original body"
        );
        i += 1;
        reflowed += 1;
    }
    assert_eq!(
        (i, j),
        (a.len(), b.len()),
        "{what}: the two outputs did not align to their ends"
    );
    reflowed
}

// ---------------------------------------------------------------------------
// the sweep, with wrapping ON
// ---------------------------------------------------------------------------

fn sweep(dirs: &[&str], version: RustyfiVersion, label: &str) -> (usize, usize, usize) {
    let files = corpus(dirs);
    assert!(
        files.len() > 20,
        "expected the bundled corpus, found {} files — is the checkout complete?",
        files.len()
    );
    let on = wrapping();
    let (mut checked, mut files_changed, mut reflowed) = (0, 0, 0);
    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        let what = path.display().to_string();
        let base = format_cst(&src, version, &not_wrapping())
            .unwrap_or_else(|| panic!("{what}: declined with wrapping OFF"));
        let out = format_cst(&src, version, &on).unwrap_or_else(|| {
            panic!(
                "{what}: DECLINED with wrapping ON. A decline is not a \
                 pass — the always-on verifier returns `None`, so it is exactly how a broken \
                 printer hides inside a token-identity sweep."
            )
        });

        // 1. Token identity. Comments emit no tokens, so this can only fail if
        //    a wrapped line stopped being a comment.
        let before = rustyfi_syntax::lex_with_version(&src, version).expect("the input lexes");
        let after = rustyfi_syntax::lex_with_version(&out, version)
            .unwrap_or_else(|_| panic!("{what}: the wrapped text no longer lexes"));
        let bt: Vec<_> = before.iter().map(|a| &a.slot).collect();
        let at: Vec<_> = after.iter().map(|a| &a.slot).collect();
        assert_eq!(bt, at, "{what}: the token stream changed");

        // 2. Text and math areas byte-identical. Reflow never reaches one: an
        //    inline comment lives INSIDE a `Token::Space`/`Break` span
        //    (`trivia.rs`'s module comment), so it is not in a gap at all.
        //
        //    Compared against `base` — the same file with wrapping OFF —
        //    rather than against the SOURCE, because since block text and math
        //    got real layout the source is not the right baseline: a region
        //    differs from `src` for reasons that have nothing to do with
        //    reflow. What this test is actually about is that turning wrapping
        //    on changes no text area, and `base` is the baseline that says so.
        let based = rustyfi_syntax::lex_with_version(&base, version).expect("the base lexes");
        for (x, y) in text_math_regions(&based, &base)
            .iter()
            .zip(text_math_regions(&after, &out).iter())
        {
            assert_eq!(x, y, "{what}: a text/math region was rewritten");
        }

        // 3. Idempotence.
        let twice = format_cst(&out, version, &on)
            .unwrap_or_else(|| panic!("{what}: declined its own wrapped output"));
        assert_eq!(
            twice, out,
            "{what}: reflow is not idempotent — a format-on-save loop"
        );

        // 4, restated.
        reflowed += assert_reflow_only(&base, &out, &what);

        // 5.
        if let Some(d) = cst_walk_desync(&src, version, &on) {
            assert_eq!(d, 0, "{what}: the CST walk drifted {d} time(s)");
        }

        checked += 1;
        files_changed += usize::from(out != base);
    }
    eprintln!(
        "comment reflow, {label}: {checked} files, {files_changed} changed by \
         wrapping alone, {reflowed} comments reflowed"
    );
    (checked, files_changed, reflowed)
}

/// Text/math regions as strings, derived from token kinds — the technique
/// `format_cst_slice1.rs` and `tests/format.rs` use, so a bug in the area fold
/// cannot hide behind the formatter's own idea of where an area is.
fn text_math_regions(atoms: &[rustyfi_syntax::Atom], src: &str) -> Vec<String> {
    use rustyfi_syntax::Token;
    let is_text = |t: &Token| {
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
    };
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut prev = false;
    for a in atoms {
        let t = is_text(&a.slot);
        match (t, prev) {
            (true, true) => out.last_mut().expect("a run is open").1 = a.span.end.byte,
            (true, false) => out.push((a.span.start.byte, a.span.end.byte)),
            _ => {}
        }
        prev = t;
    }
    out.iter().map(|(s, e)| src[*s..*e].to_string()).collect()
}

#[test]
fn wrapping_on_holds_the_five_properties_over_the_v006_corpus() {
    let (checked, changed, reflowed) = sweep(
        &["lib-rustyfi/dist/packages", "layout-tests/corpus"],
        RustyfiVersion::V0_0,
        "0.0.6 corpus",
    );
    assert!(checked > 100, "only {checked} files reached the comparison");
    // Non-vacuity, and the number is deliberately NOT asserted: `CLAUDE.md` on
    // measured numbers in CI. What must hold is that the rule fires at all —
    // a classifier that rejected everything would sweep green otherwise, and
    // "0.12% of comments" is exactly the regime where that goes unnoticed.
    assert!(
        reflowed > 0,
        "no comment was reflowed anywhere in the corpus, so this sweep proves \
         nothing about the wrapping path. Either `comment::is_prose` stopped \
         accepting anything or the builders stopped consulting it."
    );
    assert!(changed > 0, "no file changed under wrapping");
}

#[test]
fn wrapping_on_holds_the_five_properties_over_the_v01_corpus() {
    // No non-vacuity assertion here: whether the 0.1 corpus HAS an over-width
    // prose comment is a fact about 47 third-party files, not about the rule.
    // The 0.0.6 sweep above owns the "it fires" claim.
    sweep(
        &["lib-rustyfi/dist-v01/packages"],
        RustyfiVersion::V0_1,
        "0.1 corpus",
    );
}

// ---------------------------------------------------------------------------
// the default
// ---------------------------------------------------------------------------

/// The DEFAULT reflows, and turning the key off makes the feature inert — on
/// the one shape it would otherwise rewrite.
///
/// This test used to read the other way round (`the_default_reflows_nothing`),
/// and it is here in its inverted form rather than deleted because the thing
/// worth pinning is that the key still *controls* something. A default flip
/// that left a test asserting the old default would have failed loudly; one
/// that left this test comparing two identical option sets would have passed
/// while checking nothing, which is the outcome to avoid.
#[test]
fn the_default_reflows_and_turning_the_key_off_does_not() {
    // The program tail is written at slice 3's own fixpoint (`in` and the
    // body each own a line), so that "nothing moved" is a claim about the
    // COMMENT and not about the line-break rules underneath it.
    let long = format!("% {}\nlet x = 1\nin\nx\n", vec!["alpha"; 30].join(" "));
    let on = format_cst(&long, RustyfiVersion::V0_0, &CstOptions::default()).expect("formats");
    assert_ne!(on, long, "the default no longer reflows a 30-word comment");
    assert!(
        on.lines().filter(|l| l.starts_with('%')).count() >= 2,
        "the default did not actually wrap it: {on:?}"
    );
    // And the key really is what did it.
    let off = format_cst(&long, RustyfiVersion::V0_0, &not_wrapping()).expect("formats");
    assert_eq!(off, long, "`wrap_comments: false` must not touch a comment");
}

// ---------------------------------------------------------------------------
// idempotence at the boundary, which is what a wrap breaks first
// ---------------------------------------------------------------------------

/// One column under the budget, exactly at it, and one over — at three
/// indentations, in both scripts, for both grammars.
///
/// The interesting one is *exactly at*: a formatter that compares `>=` rather
/// than `>` wraps a line that fits, and the wrapped line then fits too, so the
/// second pass leaves it alone and idempotence testing at a random width never
/// sees it. It shows up here as a line that moved when it should not have.
#[test]
fn reflow_is_a_fixpoint_at_the_boundary_width() {
    let opts = wrapping();
    for indent in [0usize, 2, 8] {
        for (word, marker) in [("alpha", "% "), ("あい", "%% ")] {
            for delta in [-1i32, 0, 1] {
                // Build a comment whose LINE is exactly `max_width + delta`
                // columns wide, out of repeated words plus a filler tail.
                let unit = width(word) + 1; // the word and the space before it
                let head = indent + marker.chars().count();
                let n = (opts.max_width - head) / unit;
                let mut body = vec![word; n].join(" ");
                let want = (opts.max_width as i32 + delta) as usize;
                while head + width(&body) < want {
                    body.push('x');
                }
                assert_eq!(
                    head + width(&body),
                    want,
                    "the fixture itself is off by a column"
                );
                let src = format!("{}{marker}{body}\nlet x = 1\nin\nx\n", " ".repeat(indent));
                let once = format_cst(&src, RustyfiVersion::V0_0, &opts)
                    .unwrap_or_else(|| panic!("declined {src:?}"));
                let twice = format_cst(&once, RustyfiVersion::V0_0, &opts)
                    .unwrap_or_else(|| panic!("declined its own output for {src:?}"));
                assert_eq!(
                    twice, once,
                    "not a fixpoint at width {want} (indent {indent}, {marker:?})"
                );
                // At or under the budget nothing may move at all.
                if delta <= 0 {
                    assert_eq!(
                        once,
                        src,
                        "a comment {} the budget was reflowed",
                        if delta == 0 { "exactly at" } else { "under" }
                    );
                } else {
                    assert!(
                        once.lines().count() > src.lines().count(),
                        "a comment one column OVER the budget was not reflowed: {once:?}"
                    );
                    // And every line it produced is inside the budget, which is
                    // what stops the next pass from wrapping again.
                    for l in once.lines().filter(|l| l.trim_start().starts_with('%')) {
                        assert!(
                            width(l) <= opts.max_width,
                            "reflow left {l:?} over the budget"
                        );
                    }
                }
            }
        }
    }
}

/// Display columns, the same table `format_cst::render::width` uses. Duplicated
/// rather than exported: a test that shares the implementation it is checking
/// cannot notice the implementation being wrong.
fn width(s: &str) -> usize {
    s.chars()
        .map(|c| match c as u32 {
            0x1100..=0x115F
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE6F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1F64F
            | 0x1F900..=0x1F9FF
            | 0x20000..=0x2FFFD
            | 0x30000..=0x3FFFD => 2,
            _ => 1,
        })
        .sum()
}

// ---------------------------------------------------------------------------
// the three rules that must hold whatever the policy
// ---------------------------------------------------------------------------

/// Two comments on consecutive lines stay two comments, at every width — the
/// rule reflow is most able to break, because a naive "reflow the comment
/// block" would treat them as one paragraph.
#[test]
fn two_comments_are_never_joined() {
    let opts = wrapping();
    let src = "% alpha beta gamma delta\n% epsilon zeta eta theta\nlet x = 1 in x\n";
    for max_width in [8usize, 12, 20, 40, 100] {
        let o = CstOptions {
            max_width,
            ..opts.clone()
        };
        let out = format_cst(src, RustyfiVersion::V0_0, &o).expect("formats");
        // No output line may carry text from both comments.
        for l in out.lines() {
            let has_a = l.contains("alpha") || l.contains("delta");
            let has_b = l.contains("epsilon") || l.contains("theta");
            assert!(
                !(has_a && has_b),
                "two comments merged at width {max_width}: {l:?}"
            );
        }
        // And the two bodies survive in order, whole.
        let joined: String = out
            .lines()
            .filter(|l| l.trim_start().starts_with('%'))
            .map(|l| l.trim_start().trim_start_matches('%').trim())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            joined, "alpha beta gamma delta epsilon zeta eta theta",
            "at width {max_width}"
        );
    }
}

/// A trailing comment stays trailing: reflowing one would have to start a new
/// line, and a continuation line is not a trailing comment. So it is left
/// alone at any width, however long it is.
#[test]
fn a_trailing_comment_is_never_reflowed() {
    let opts = CstOptions {
        max_width: 20,
        ..wrapping()
    };
    let src = "let x = 1\nin % alpha beta gamma delta epsilon zeta\nx\n";
    let out = format_cst(src, RustyfiVersion::V0_0, &opts).expect("formats");
    assert_eq!(out, src);
}

/// Commented-out code is never wrapped, at any width — the false accept that
/// would corrupt somebody's disabled program. Every one of these is a real
/// corpus line.
#[test]
fn commented_out_code_is_never_reflowed_at_any_width() {
    for body in [
        "%       let () = display-message `insert` in",
        "%        if get-text-width ctx <' get-natural-width ib-title then",
        "%                    xs |> List.map (fun x -> start-path (x, yF) |> line-to (x, yL))",
        "%                let p = XPathCurve.get-point-from-curves-and-pos ci cj (u, v) in",
        "%   | align left | align center | align right",
        "%    rll   rlr            rlr   rr",
        "%     (gen-arctic-item depth)",
        "%       )",
        "% val get-self-intersects : length -> t -> (float * float) list",
        "%% e.g. [if (i < String.length s) then `yes` else `no`]",
    ] {
        let src = format!("{body}\nlet x = 1\nin\nx\n");
        for max_width in [4usize, 10, 20, 40, 100] {
            let opts = CstOptions {
                max_width,
                ..wrapping()
            };
            let out = format_cst(&src, RustyfiVersion::V0_0, &opts).expect("formats");
            // The COMMENT is the claim. At four columns slice 3 breaks the
            // program tail after the `=`, which is its job and not this
            // test's, so the comparison is on the comment lines alone.
            let comment = |t: &str| {
                t.lines()
                    .take_while(|l| l.trim_start().starts_with('%'))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            assert_eq!(
                comment(&out),
                comment(&src),
                "{body:?} was reflowed at width {max_width}"
            );
        }
    }
}

/// An own-line comment keeps the author's indentation through a reflow,
/// including on the lines the reflow invents — slice 1's rule, which exists
/// because `%`-disabled code parked at column 0 was parked there deliberately.
#[test]
fn a_reflowed_comment_keeps_the_authors_indentation_on_every_line() {
    let opts = CstOptions {
        max_width: 24,
        ..wrapping()
    };
    let src = "let f x =\n      % alpha beta gamma delta epsilon\n  1\nin f 2\n";
    let out = format_cst(src, RustyfiVersion::V0_0, &opts).expect("formats");
    let comment_lines: Vec<&str> = out
        .lines()
        .filter(|l| l.trim_start().starts_with('%'))
        .collect();
    assert!(comment_lines.len() >= 2, "{out:?}");
    for l in &comment_lines {
        assert!(
            l.starts_with("      %"),
            "a reflowed line lost the author's six-column indent: {l:?}"
        );
    }
}
