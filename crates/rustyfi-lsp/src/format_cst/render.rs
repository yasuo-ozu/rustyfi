//! The renderer: Lindig's strict rendering of a Wadler `Doc`.
//!
//! Two functions, as in the paper. `fits` asks whether a flat rendering of what
//! remains on the current line stays inside the budget; `best` walks the
//! document with an explicit work stack, choosing a mode per group. The stack is
//! explicit rather than recursive because a SATySFi file's `let … in` chain
//! nests to the right one level per binding (`cst.rs:767-782`) and
//! `stdja.satyh:272-289` is eleven deep — a recursive renderer is a stack-depth
//! question on real input, not a hypothetical one.
//!
//! Widths are **unicode display columns**, not bytes and not `char`s: this
//! corpus is Japanese and an East Asian Wide character occupies two columns, so
//! a byte or `char` count is wrong by a factor of two on prose. rustfmt reaches
//! the same conclusion via `unicode_str_width`.

use super::doc::{Doc, Mode};

/// Display width of `s` in columns.
///
/// East Asian Wide and Fullwidth count 2, everything else 1. A tab is counted
/// at `tab_width` from the current column by the caller, not here — a tab's
/// width is a function of *where* it is, which a per-string function cannot know.
pub(crate) fn width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

fn char_width(c: char) -> usize {
    // The two East Asian Width classes that are two columns wide. Ranges from
    // UAX #11; this is deliberately a small hand-rolled table rather than a new
    // dependency, because `rustyfi-lsp`'s analysis half promises nothing outside
    // `rustyfi-syntax` (`lib.rs:8-22`) and the playground links it into wasm.
    match c as u32 {
        0x1100..=0x115F           // Hangul Jamo initial consonants
        | 0x2E80..=0x303E         // CJK radicals, Kangxi, CJK symbols
        | 0x3041..=0x33FF         // Hiragana, Katakana, Bopomofo, compatibility
        | 0x3400..=0x4DBF         // CJK ext A
        | 0x4E00..=0x9FFF         // CJK unified
        | 0xA000..=0xA4CF         // Yi
        | 0xAC00..=0xD7A3         // Hangul syllables
        | 0xF900..=0xFAFF         // CJK compatibility ideographs
        | 0xFE30..=0xFE6F         // CJK compatibility forms
        | 0xFF00..=0xFF60         // Fullwidth forms
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F       // Emoji (wide in practice)
        | 0x1F900..=0x1F9FF
        | 0x20000..=0x2FFFD       // CJK ext B..
        | 0x30000..=0x3FFFD => 2,
        _ => 1,
    }
}

/// A rendering decision for one group, resolved before its contents are walked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fit {
    Flat,
    Broken,
}

pub(crate) struct Options {
    /// The column budget. Lines may still exceed it: a `Verbatim` area has no
    /// legal break point, and in SATySFi that is normal rather than a bug — one
    /// line of Japanese prose or one URL can be hundreds of columns wide. That
    /// is why there is no `error_on_line_overflow`.
    pub(crate) max_width: usize,
    /// Columns per indentation step.
    pub(crate) indent: usize,
    /// What a line terminator is, for this file.
    pub(crate) newline: &'static str,
    /// How many consecutive blank lines survive.
    pub(crate) max_blank_lines: usize,
}

/// Render `doc`.
pub(crate) fn render(doc: &Doc<'_>, opts: &Options) -> String {
    let mut out = String::new();
    // Work items are (indent, fit, doc). `fit` is the enclosing group's resolved
    // mode, which is what a `Line` consults.
    let mut work: Vec<(usize, Fit, &Doc<'_>)> = vec![(0, Fit::Broken, doc)];
    // Column reached on the current line.
    let mut col = 0usize;
    // Blank lines emitted since the last content byte, so the cap is applied to
    // the FINAL line structure rather than to anything the builder guessed.
    let mut pending_blanks = 0usize;
    // Whether anything has been written at all, so a leading blank line can be
    // dropped: a file does not begin with a section break.
    let mut wrote_content = false;
    // Indentation owed to the current line, written by the next content byte.
    //
    // LAZY rather than eager, and that is required rather than tidier — slice
    // 0's eager version was correct only because it never emitted a `Nest` and
    // never a `BlankLine` after one. With both, eager indentation writes the
    // indent of a line that turns out to be *blank* (trailing whitespace) and
    // then fails to write one for the line the content actually lands on
    // (`flush_blanks` emits bare terminators). See
    // `a_blank_line_between_two_indented_lines_indents_the_second`.
    let mut pending_indent: Option<usize> = None;

    while let Some((ind, fit, d)) = work.pop() {
        match d {
            Doc::Nil => {}
            Doc::Concat(parts) => {
                // Reversed, because the stack pops last-first.
                for p in parts.iter().rev() {
                    work.push((ind, fit, p));
                }
            }
            Doc::Nest(n, inner) => {
                let next = (ind as i32 + n).max(0) as usize;
                work.push((next, fit, inner));
            }
            Doc::Group(mode, inner) => {
                let chosen = match mode {
                    Mode::Break => Fit::Broken,
                    Mode::Auto | Mode::Fill => {
                        match inner.forces_break()
                            || !fits(inner, &work, opts.max_width.saturating_sub(col))
                        {
                            true => Fit::Broken,
                            false => Fit::Flat,
                        }
                    }
                };
                work.push((ind, chosen, inner));
            }
            Doc::Token { text, .. } | Doc::Verbatim(text) => {
                if text.is_empty() {
                    continue;
                }
                open_line(
                    &mut out,
                    &mut pending_blanks,
                    &mut pending_indent,
                    wrote_content,
                    opts,
                );
                out.push_str(text);
                wrote_content = true;
                // A multi-line copied range resets the column to whatever its
                // own last line reached — the enclosing layout continues from
                // there, which is why `first_line`/`multiline` is what `fits`
                // reads about a slice rather than its total width.
                col = match text.rfind('\n') {
                    Some(i) => width(&text[i + 1..]),
                    None => col + width(text),
                };
                // A copied range that ENDS in a terminator owes the next line
                // an indent like any other newline. The case is not
                // hypothetical: `lex_header` swallows a header's line break
                // into the token (`lexer.rs:915-933`), so every `@require:`
                // reaches here as a `Doc::Token` ending in `\n`.
                if text.ends_with('\n') || text.ends_with('\r') {
                    pending_indent = Some(ind);
                    col = ind;
                }
            }
            Doc::Line | Doc::SoftLine => match fit {
                Fit::Flat => {
                    if matches!(d, Doc::Line) {
                        open_line(
                            &mut out,
                            &mut pending_blanks,
                            &mut pending_indent,
                            wrote_content,
                            opts,
                        );
                        out.push(' ');
                        col += 1;
                    }
                }
                Fit::Broken => newline(&mut out, &mut col, ind, opts, &mut pending_indent),
            },
            Doc::VerbatimIndent(text) => {
                // Replaces the indent this line is owed. Honoured only while
                // one is still owed, or while the line is still empty (the
                // start of the file owes none); mid-line this would be
                // intra-line spacing and the builder emits `Verbatim` there.
                flush_blanks(&mut out, &mut pending_blanks, wrote_content, opts);
                if pending_indent.take().is_some() || col == 0 {
                    out.push_str(text);
                    col = width(text);
                }
            }
            Doc::HardLine => newline(&mut out, &mut col, ind, opts, &mut pending_indent),
            Doc::FillLine => {
                // Greedy fill: the space costs one column, so what follows it
                // has `max_width - col - 1` to land in. `fits_ahead` measures
                // the REST OF THE WORK STACK up to the next break
                // opportunity, which is the one thing `fits` cannot do — it
                // takes a subtree, and a fill point's chunk is not a subtree
                // of anything.
                let budget = opts.max_width.saturating_sub(col + 1);
                match fits_ahead(&work, budget) {
                    true => {
                        open_line(
                            &mut out,
                            &mut pending_blanks,
                            &mut pending_indent,
                            wrote_content,
                            opts,
                        );
                        out.push(' ');
                        wrote_content = true;
                        col += 1;
                    }
                    false => newline(&mut out, &mut col, ind, opts, &mut pending_indent),
                }
            }
            Doc::BlankLine => {
                // Counted, not emitted. `flush_blanks` decides how many survive,
                // once it knows there is content after them.
                pending_blanks += 1;
            }
        }
    }
    finish(&mut out, opts);
    out
}

/// The file's last line: no trailing whitespace, no trailing blank lines, and
/// **exactly one** terminator.
///
/// Two of the four normalisations every LSP client expects
/// (`trimTrailingWhitespace` / `insertFinalNewline` + `trimFinalNewlines`); the
/// other two — the per-line trim and the blank-line cap — the builder and
/// [`flush_blanks`] already do. They live *here*, after the whole document has
/// been laid out, for the reason `format.rs:466-483` records from doing it the
/// other way round: the lex-based formatter added its final newline AFTER the
/// blank-line cap had run, so a whitespace-only last line became a blank line
/// that only the NEXT format capped away, and two consecutive saves of an
/// untouched file produced two different files. Trimming first and terminating
/// second cannot reach that state — the bytes this writes are the bytes the
/// second pass reads back.
///
/// # The two things it must not do
///
/// - **Add a second newline to a file whose last token is a header.**
///   `lex_header` swallows the line's terminator INTO the token
///   (`lexer.rs:915-933`), so `@require: x\n` reaches here as a `Doc::Token`
///   that already ends the line. Trimming before appending is what makes the
///   two cases one: the header's own terminator is removed and the same one put
///   back, so the output is byte-identical and format-on-save is idempotent.
///   Judging by "does the output end in a newline" alone is what made the
///   lex-based formatter append a blank line every save (`format.rs:536-556`).
/// - **Split a CRLF, or leave a lone `\r`.** The trim takes whole terminators
///   because it strips every trailing `\r` and `\n` there is, and the one
///   written back is `opts.newline` — the file's own.
///
/// Trimming can only ever remove program-area gap whitespace or a header's
/// swallowed terminator: no other token kind in either grammar ends in
/// whitespace (every text, math, literal and block area closes with its own
/// delimiter), so this never reaches inside a token's meaning. A rendering that
/// is *entirely* whitespace loses it and gains no terminator — an empty file
/// stays empty rather than becoming a blank line.
fn finish(out: &mut String, opts: &Options) {
    let keep = out.trim_end_matches([' ', '\t', '\n', '\r']).len();
    out.truncate(keep);
    if !out.is_empty() {
        out.push_str(opts.newline);
    }
}

fn newline(
    out: &mut String,
    col: &mut usize,
    ind: usize,
    opts: &Options,
    pending_indent: &mut Option<usize>,
) {
    out.push_str(opts.newline);
    // The indent is OWED, not written: the line may turn out to be blank (a
    // `BlankLine` follows), and an indent on a blank line is trailing
    // whitespace. `col` is set anyway so it stays truthful for `fits`.
    *pending_indent = Some(ind);
    *col = ind;
}

/// Start the line the next content byte lands on: the blank lines that survive
/// the cap, then the indent owed to whatever line that leaves us on.
fn open_line(
    out: &mut String,
    pending_blanks: &mut usize,
    pending_indent: &mut Option<usize>,
    wrote_content: bool,
    opts: &Options,
) {
    flush_blanks(out, pending_blanks, wrote_content, opts);
    if let Some(ind) = pending_indent.take() {
        for _ in 0..ind {
            out.push(' ');
        }
    }
}

/// Emit the blank lines that survive the cap, now that content is known to
/// follow them.
fn flush_blanks(out: &mut String, pending: &mut usize, wrote_content: bool, opts: &Options) {
    if *pending == 0 {
        return;
    }
    // Leading blank lines are dropped whatever the cap says.
    let n = match wrote_content {
        true => (*pending).min(opts.max_blank_lines),
        false => 0,
    };
    for _ in 0..n {
        out.push_str(opts.newline);
    }
    *pending = 0;
}

/// Does a flat rendering of `doc`, **plus whatever shares its line
/// afterwards**, fit in `budget` columns?
///
/// Stops at the first newline, the first break opportunity after the group, or
/// the first overrun, so it is O(width) rather than O(document) — the property
/// that makes the strict renderer linear.
///
/// # Why it reads the continuation, and not only the group
///
/// Lindig's `fits` measures the group alone, and on this grammar that is off by
/// exactly the trailing token that shares the line. `stdja.satyh:199` is the
/// exhibit:
///
/// ```text
///   start-path (x, y +' thk *' 0.5) |> line-to (x +' wid, y +' thk *' 0.5) |> terminate-path in
/// ```
///
/// The `|>` chain is 98 columns and fits; the ` in` that the enclosing `let …
/// in` spine puts after it does not, and the chain has already committed to
/// flat by then. Measuring the group and then continuing through the work
/// stack until the next place a line may end is what prettier does, and it is
/// the same linear cost — the continuation is bounded by the same budget.
///
/// A group in the continuation is measured FLAT. That is an approximation and
/// it errs towards breaking, which is the direction that cannot produce a line
/// over budget.
fn fits(doc: &Doc<'_>, after: &[(usize, Fit, &Doc<'_>)], budget: usize) -> bool {
    let mut work: Vec<&Doc<'_>> = vec![doc];
    let mut used = 0usize;
    // Index into `after`, walked from its end (the stack's top) once `work`
    // drains. `None` until then, so a `Line` inside the group counts as one
    // flat column while a `Line` after it ends the line and the measurement.
    let mut ahead: Option<usize> = None;
    loop {
        let d = match work.pop() {
            Some(d) => d,
            None => match ahead.unwrap_or(after.len()).checked_sub(1) {
                None => return true,
                Some(j) => {
                    ahead = Some(j);
                    after[j].2
                }
            },
        };
        let past = ahead.is_some();
        match d {
            // Every kind of break opportunity ends the chunk once the group
            // itself has been measured.
            Doc::Line | Doc::SoftLine | Doc::FillLine | Doc::HardLine | Doc::BlankLine
                if past =>
            {
                return true
            }
            Doc::Nil | Doc::SoftLine | Doc::BlankLine => {}
            Doc::VerbatimIndent(text) => used += width(text),
            Doc::Concat(parts) => work.extend(parts.iter().rev()),
            Doc::Nest(_, inner) | Doc::Group(_, inner) => work.push(inner),
            Doc::Token { text, .. } | Doc::Verbatim(text) => {
                // A multiline range never fits flat: there is no flat rendering
                // of it, because its newlines are content. Past the group it
                // ENDS the line rather than failing it — only the part before
                // its terminator shares this one.
                if let Some(i) = text.find(['\n', '\r']) {
                    return past && used + width(&text[..i]) <= budget;
                }
                used += width(text);
                if used > budget {
                    return false;
                }
            }
            Doc::Line | Doc::FillLine => {
                used += 1;
                if used > budget {
                    return false;
                }
            }
            // A hard line inside the group means the group is not flat.
            Doc::HardLine => return false,
        }
    }
}

/// Does the content from the top of the work stack to the next break
/// opportunity fit in `budget` columns?
///
/// [`fits`]'s counterpart for [`Doc::FillLine`], and it has to read the STACK
/// rather than a subtree: a fill point's chunk is "everything up to the next
/// place a break is allowed", which crosses out of whatever `Concat` the
/// point sits in and is not a subtree of anything. The stack is popped from
/// its end, so "ahead" is `work` in reverse, and each item is expanded into a
/// local stack that must be drained before the next one is taken.
///
/// Stops at the first break opportunity, the first newline inside a copied
/// range, or the first overrun — so it costs O(the chunk's width), not
/// O(what is left of the document), and the strict renderer stays linear.
///
/// A chunk that reaches the end of the document with nothing over budget
/// answers `true`: there is no more content to overflow the line.
fn fits_ahead(work: &[(usize, Fit, &Doc<'_>)], budget: usize) -> bool {
    let mut used = 0usize;
    let mut local: Vec<&Doc<'_>> = Vec::new();
    let mut i = work.len();
    loop {
        let d = match local.pop() {
            Some(d) => d,
            None => match i.checked_sub(1) {
                None => return true,
                Some(j) => {
                    i = j;
                    work[j].2
                }
            },
        };
        match d {
            // Every kind of break opportunity ends the chunk. `BlankLine`
            // included: it is a line ending that has not been counted yet.
            Doc::FillLine | Doc::HardLine | Doc::BlankLine | Doc::Line | Doc::SoftLine => {
                return true
            }
            Doc::Nil => {}
            Doc::Concat(parts) => local.extend(parts.iter().rev()),
            Doc::Nest(_, inner) | Doc::Group(_, inner) => local.push(inner),
            Doc::VerbatimIndent(text) => used += width(text),
            Doc::Token { text, .. } | Doc::Verbatim(text) => {
                // A copied range that ends the line ends the chunk with it,
                // and only the part before its terminator is on this line.
                if let Some(j) = text.find(['\n', '\r']) {
                    return used + width(&text[..j]) <= budget;
                }
                used += width(text);
                if used > budget {
                    return false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Note the trailing `\n` on every expectation: [`render`] renders a
    //! WHOLE FILE, and [`finish`] terminates one. These strings gained it when
    //! the final-newline rule landed; the layout each of them is actually about
    //! is everything to the left of it.
    use super::*;

    fn tok(s: &str) -> Doc<'_> {
        Doc::Token { text: s, atom: 0 }
    }

    fn opts(max_width: usize) -> Options {
        Options {
            max_width,
            indent: 2,
            newline: "\n",
            max_blank_lines: 2,
        }
    }

    #[test]
    fn cjk_counts_two_columns() {
        assert_eq!(width("abc"), 3);
        assert_eq!(width("日本語"), 6);
        // Mixed, which is what every real document is.
        assert_eq!(width("a日b"), 4);
    }

    #[test]
    fn an_auto_group_stays_flat_when_it_fits_and_breaks_when_it_does_not() {
        let doc = Doc::Group(
            Mode::Auto,
            Box::new(Doc::concat(vec![tok("a"), Doc::Line, tok("b")])),
        );
        assert_eq!(render(&doc, &opts(80)), "a b\n");
        assert_eq!(render(&doc, &opts(2)), "a\nb\n");
    }

    #[test]
    fn a_break_group_breaks_at_any_width() {
        let doc = Doc::Group(
            Mode::Break,
            Box::new(Doc::concat(vec![tok("a"), Doc::Line, tok("b")])),
        );
        assert_eq!(render(&doc, &opts(1000)), "a\nb\n");
    }

    #[test]
    fn nest_indents_only_the_broken_arm() {
        let doc = Doc::Group(
            Mode::Auto,
            Box::new(Doc::Nest(
                2,
                Box::new(Doc::concat(vec![tok("a"), Doc::Line, tok("b")])),
            )),
        );
        assert_eq!(render(&doc, &opts(80)), "a b\n");
        assert_eq!(render(&doc, &opts(2)), "a\n  b\n");
    }

    #[test]
    fn a_multiline_verbatim_forces_its_group_open() {
        // The area-boundary case: a text area's newlines are content, so there
        // is no flat rendering of it and the enclosing group cannot be flat.
        let doc = Doc::Group(
            Mode::Auto,
            Box::new(Doc::concat(vec![
                tok("f"),
                Doc::Line,
                Doc::Verbatim("{a\nb}"),
            ])),
        );
        assert_eq!(render(&doc, &opts(1000)), "f\n{a\nb}\n");
    }

    #[test]
    fn blank_lines_are_capped_against_the_final_structure() {
        let doc = Doc::concat(vec![
            tok("a"),
            Doc::HardLine,
            Doc::BlankLine,
            Doc::BlankLine,
            Doc::BlankLine,
            Doc::BlankLine,
            tok("b"),
        ]);
        // Four requested, two survive: one terminator from the HardLine, then
        // two blank lines, then `b`.
        assert_eq!(render(&doc, &opts(80)), "a\n\n\nb\n");
    }

    /// The bug the lazy indent exists for: with eager indentation this wrote
    /// the indent of the *blank* line (trailing whitespace) and none for the
    /// line `b` landed on.
    #[test]
    fn a_blank_line_between_two_indented_lines_indents_the_second() {
        let doc = Doc::Nest(
            2,
            Box::new(Doc::concat(vec![
                Doc::HardLine,
                tok("a"),
                Doc::HardLine,
                Doc::BlankLine,
                tok("b"),
            ])),
        );
        assert_eq!(render(&doc, &opts(80)), "\n  a\n\n  b\n");
    }

    /// A `Doc::Token` whose own bytes end in a terminator — which is what a
    /// header is — owes the next line its indent too.
    #[test]
    fn a_token_ending_in_a_terminator_still_indents_the_next_line() {
        let doc = Doc::Nest(
            2,
            Box::new(Doc::concat(vec![tok("@require: x\n"), tok("b")])),
        );
        assert_eq!(render(&doc, &opts(80)), "@require: x\n  b\n");
    }

    #[test]
    fn leading_blank_lines_are_dropped_whatever_the_cap_says() {
        let doc = Doc::concat(vec![Doc::BlankLine, Doc::BlankLine, tok("a")]);
        assert_eq!(render(&doc, &opts(80)), "a\n");
    }

    #[test]
    fn every_invented_terminator_honours_the_files_own_newline() {
        // NOT reachable from slice 0's corpus sweep: the identity builder emits
        // no `Line`/`HardLine`/`BlankLine`, so every newline in its output is a
        // COPIED byte and `opts.newline` is never consulted. Found by mutating
        // `newline` to a hard-coded "\n" and watching the sweep stay green.
        // From slice 1 the renderer invents every program-area terminator, and
        // then this is the whole of CRLF correctness.
        let doc = Doc::concat(vec![tok("a"), Doc::HardLine, Doc::BlankLine, tok("b")]);
        let crlf = Options {
            newline: "\r\n",
            ..opts(80)
        };
        assert_eq!(render(&doc, &crlf), "a\r\n\r\nb\r\n");
        // No LONE `\r` and no LONE `\n` survives anywhere — the fusion class
        // that produced four separate bugs in the lex-based formatter, all of
        // them from a CRLF split across a token-span boundary.
        //
        // Note what this must NOT assert: `!out.contains("\n\r")`. Any two
        // consecutive CRLFs contain `\n\r` at their boundary, so that check
        // fails on correct output — it was the first thing written here and it
        // was wrong, not the renderer.
        let out = render(&doc, &crlf);
        let bytes = out.as_bytes();
        for (i, b) in bytes.iter().enumerate() {
            match b {
                b'\r' => assert_eq!(bytes.get(i + 1), Some(&b'\n'), "lone CR at {i} in {out:?}"),
                b'\n' => assert_eq!(
                    i.checked_sub(1).and_then(|j| bytes.get(j)),
                    Some(&b'\r'),
                    "lone LF at {i} in {out:?}"
                ),
                _ => {}
            }
        }
    }

    #[test]
    fn the_column_continues_from_a_multiline_ranges_last_line() {
        // After a copied range ending in "cd", the column is 2, not the range's
        // total width — otherwise every fit decision after a text area is wrong.
        let doc = Doc::concat(vec![Doc::Verbatim("aaaaaaaaaa\ncd"), tok("!")]);
        assert_eq!(render(&doc, &opts(80)), "aaaaaaaaaa\ncd!\n");
    }

    /// [`finish`], on its own, and on the shapes that made the lex-based
    /// formatter non-idempotent.
    #[test]
    fn the_last_line_is_trimmed_and_terminated_exactly_once() {
        let case = |doc: &Doc<'_>| render(doc, &opts(80));
        // No terminator at all: one is invented.
        assert_eq!(case(&tok("a")), "a\n");
        // A token that already ends the line — a header — keeps ITS OWN
        // terminator and gains none, which is the whole of format-on-save
        // idempotence for a buffer whose last token is a header.
        assert_eq!(case(&tok("@require: x\n")), "@require: x\n");
        // Trailing whitespace on the last line goes, and so do trailing blank
        // lines: the cap never sees them, because no content follows.
        assert_eq!(
            case(&Doc::concat(vec![tok("a"), Doc::Verbatim("   ")])),
            "a\n"
        );
        assert_eq!(
            case(&Doc::concat(vec![
                tok("a"),
                Doc::HardLine,
                Doc::BlankLine,
                Doc::BlankLine,
            ])),
            "a\n"
        );
        // And it is a fixpoint at the level that matters: rendering something
        // that already ends in exactly one terminator does not add a second.
        assert_eq!(case(&tok("a\n")), "a\n");
        // A document that renders to nothing but whitespace stays empty rather
        // than becoming a blank line.
        assert_eq!(case(&Doc::Verbatim("   ")), "");
        assert_eq!(case(&Doc::Nil), "");
        // CRLF: whole terminators only, and the file's own written back.
        let crlf = Options {
            newline: "\r\n",
            ..opts(80)
        };
        assert_eq!(render(&tok("a"), &crlf), "a\r\n");
        assert_eq!(render(&tok("@require: x\r\n"), &crlf), "@require: x\r\n");
    }
}
