//! Document formatting, as a protocol-free function over source text.
//!
//! No LSP types and no I/O, for [`crate::features`]' reason: the browser
//! playground builds this crate for `wasm32-unknown-unknown` with
//! `--no-default-features`, and a "Format" button that only exists behind a
//! JSON-RPC loop cannot be reused there. [`crate::server`] turns the result
//! into a `textDocument/formatting` reply and does nothing else.
//!
//! # The constraint that decides the whole design
//!
//! **In SATySFi, whitespace is usually content.** The language has four
//! lexical areas ([`Area`]) and only one of them — program text — throws
//! whitespace away. Inside inline text `{ … }`, block text `'< … >` and math
//! `${ … }` the lexer *tokenises* it: a run of spaces becomes
//! `Token::Space` and a run containing a newline becomes `Token::Break`,
//! both of which reach the evaluator as inter-word glue and line-break
//! opportunities, and the CJK adjacency rules in `linebreak.rs` make some of
//! them decide where a line ends. Re-wrapping a paragraph does not tidy a
//! document, it re-typesets it.
//!
//! So the invariant this module exists to keep is:
//!
//! > **No byte inside a text or math area is ever altered, and no byte inside
//! > any token's span is ever altered.** Only the whitespace *between* two
//! > program-mode tokens is rewritten.
//!
//! It is kept by construction rather than by care. [`program_gaps`] replays
//! the lexer's own mode stack ([`crate::area`]) and yields only the byte ranges
//! that (a) lie strictly between two tokens and (b) are in [`Area::Program`]
//! at that point; every other byte of the file is copied across verbatim. A
//! gap that turns out to hold anything but whitespace and `%` comments — which
//! would mean the replay had drifted from the lexer — makes the whole format
//! decline rather than guess.
//!
//! # What is normalised
//!
//! In program-area whitespace only, and every one of these is *information-free
//! by construction* — the lexer skips the bytes, so nothing downstream can tell
//! the difference:
//!
//! - **trailing whitespace** at the end of a line;
//! - **the final newline**: exactly one, and trailing blank lines removed;
//! - **leading blank lines** at the top of the file, removed;
//! - **runs of blank lines**, capped ([`FormatOptions::max_blank_lines`],
//!   default 2);
//! - **tabs in indentation**, expanded to the column they occupy at the
//!   client's `tabSize` (only when the client asked for spaces).
//!
//! Four of the five are LSP's own `FormattingOptions` members —
//! `trimTrailingWhitespace`, `insertFinalNewline` / `trimFinalNewlines`, and
//! the required `tabSize` / `insertSpaces` — and are honoured as such. That
//! overlap is not a coincidence: those are the normalisations the protocol
//! treats as universal precisely because they are the ones no language can
//! attach meaning to. The blank-line cap is the one rule with no option behind
//! it, and the only aesthetic judgement in the module; see
//! [`FormatOptions::max_blank_lines`] for the number and why it is that number.
//!
//! # What is deliberately left alone, and why
//!
//! **Indentation is preserved verbatim, and lines are never joined or split.**
//! This is the significant limitation and it is a decision, not a stub. Two
//! reasons, both measured against real SATySFi rather than assumed — 99 files,
//! 24 111 lines: everything under `lib-rustyfi/dist/packages` plus every
//! third-party `.satyh` under `layout-tests/corpus`:
//!
//! - **Re-indentation from bracket nesting would be worse than the input.**
//!   Real SATySFi indents *continuations* by hand, and the hand choice carries
//!   information a bracket counter cannot recover. From `itemize.satyh`:
//!
//!   ```text
//!   let ib-parent =
//!     embed-block-top ctx (…) (fun ctx ->
//!       form-paragraph (ctx |> set-paragraph-margin item-gap item-gap)
//!         (read-inline ctx parent ++ inline-fil)
//!     )
//!   in
//!   ```
//!
//!   The last argument line is indented one step past its function with no
//!   bracket opened in between. Reproducing that needs expression-level
//!   layout — a `rustfmt`, not a whitespace pass — and a bracket-only rule
//!   would flatten it on every such line in the corpus.
//! - **Runs of two or more spaces are overwhelmingly deliberate alignment.**
//!   1 380 of those 24 111 lines carry an interior multi-space run, and
//!   sampling them finds `val font-cjk-gothic   : string * float * float`,
//!   `text-height   : length;`, `let-math \mu       = greek-lowercase …`,
//!   `| f init []        = init` and the two-space gap before an end-of-line
//!   comment — alignment in every case. Collapsing them to one space is what
//!   a naive formatter does and it destroys 5.7% of the corpus's lines for no
//!   gain.
//!
//! Also left alone, for the same "say only what is provable" rule the rest of
//! this crate follows: **whitespace is never inserted where the author wrote
//! none**. `let x=1` stays. Inserting a space is safe only between two tokens
//! that could not have merged, and the payoff (a construct almost nobody
//! writes) does not justify a rule whose failure mode is a silently
//! re-tokenised file.
//!
//! # When the formatter declines
//!
//! [`format`] returns `None` for a buffer that does not **lex**. It does not
//! require a *parse*: the areas come from the token stream, so a file using a
//! construct this port's grammar has not implemented still formats correctly,
//! and refusing there would be refusing for a reason unrelated to the answer.
//! A file that does not lex has no reliable area map at all — the mode stack
//! is what failed — so there is nothing honest to do with it.

use rustyfi_syntax::{Atom, RustyfiVersion, Token};

use crate::area::{Area, AreaStack};

/// What the client asked for, in LSP's own vocabulary.
///
/// [`Default`] is what a client that sends nothing gets, and matches the
/// behaviour every editor defaults to: expand tabs at four columns, trim
/// trailing whitespace, end the file with exactly one newline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatOptions {
    /// LSP `FormattingOptions.tabSize`: how many columns a tab advances.
    /// Only read when [`Self::insert_spaces`] is set; `0` is treated as `1`.
    pub tab_size: usize,
    /// LSP `FormattingOptions.insertSpaces`. When set, a tab in program-area
    /// indentation is replaced by the spaces it stood for. When clear,
    /// indentation is left exactly as written — this formatter never converts
    /// spaces *to* tabs, because a run of spaces may be alignment (see the
    /// module comment) and a tab cannot express alignment at an unknown tab
    /// size.
    pub insert_spaces: bool,
    /// LSP `FormattingOptions.trimTrailingWhitespace`.
    pub trim_trailing_whitespace: bool,
    /// LSP `FormattingOptions.insertFinalNewline`.
    pub insert_final_newline: bool,
    /// LSP `FormattingOptions.trimFinalNewlines`.
    pub trim_final_newlines: bool,
    /// How many consecutive blank lines to keep in program text.
    ///
    /// The one rule here with no LSP option behind it and the one aesthetic
    /// judgement in the module, so it is stated as a number rather than
    /// hidden. Two, because the bundled corpus uses a two-blank-line gap as a
    /// section break (`itemize.satyh`) and only 12 lines in 24 111 exceed it —
    /// the cap removes noise without overruling anybody's paragraphing.
    /// Leading blank lines are dropped whatever this says: a file does not
    /// begin with a section break.
    pub max_blank_lines: usize,
}

impl Default for FormatOptions {
    fn default() -> Self {
        FormatOptions {
            tab_size: 4,
            insert_spaces: true,
            trim_trailing_whitespace: true,
            insert_final_newline: true,
            trim_final_newlines: true,
            max_blank_lines: 2,
        }
    }
}

/// Format `source` under an explicitly chosen generation.
///
/// `None` means *declined*, and is not the same as "no changes": a buffer that
/// does not lex has no area map, and a buffer whose area map disagrees with
/// its own bytes (see the module comment) is one this code has misread. An
/// unchanged buffer comes back as `Some(source.to_string())`.
///
/// The generation matters less here than anywhere else in the crate — the two
/// lexers agree about every delimiter this fold reads — but it is taken
/// explicitly all the same, because 0.1 reserves words 0.0.6 does not
/// (`lexer.rs`'s version-gated keyword table) and a buffer that lexes under
/// one generation may not under the other. Use [`format_auto`] to have the
/// generation chosen from the text, exactly as [`crate::analyze_auto`] does.
pub fn format(source: &str, version: RustyfiVersion, opts: &FormatOptions) -> Option<String> {
    // An empty buffer has no program area to normalise and no line to end.
    // Giving it a newline would be inventing a line the author has not started.
    if source.is_empty() {
        return Some(String::new());
    }
    let atoms = rustyfi_syntax::lex_with_version(source, version).ok()?;
    let newline = dominant_newline(source);

    let mut out = String::with_capacity(source.len());
    let mut cursor = 0usize;
    for (start, end) in program_gaps(&atoms, source.len()) {
        out.push_str(&source[cursor..start]);
        out.push_str(&rewrite_gap(
            &source[start..end],
            Where {
                // A gap normally continues the line its preceding token sits
                // on. `@require:`/`@import:`/`@stage:` are the exception, and
                // it is a real one rather than a hypothetical: `lex_header`
                // consumes the rest of the line *and its line break* into the
                // one token (`lexer.rs:915-921`), so the gap after a header
                // starts at column 0 and its first terminator ends a BLANK
                // line, not the header's. Read off the byte before the gap so
                // that any future token with the same shape is handled too.
                at_line_start: start == 0 || source[..start].ends_with(['\n', '\r']),
                at_file_start: start == 0,
                at_file_end: end == source.len(),
            },
            newline,
            opts,
        )?);
        cursor = end;
    }
    out.push_str(&source[cursor..]);
    Some(out)
}

/// [`format`], choosing the generation from the buffer itself.
///
/// The same ladder [`crate::analyze_auto`] applies, through the same function
/// ([`crate::detect_version`]), so the formatter and the diagnostics cannot
/// disagree about which grammar a file is written in.
pub fn format_auto(source: &str, opts: &FormatOptions) -> Option<String> {
    format(source, crate::detect_version(source), opts)
}

/// The byte ranges between tokens that are in program text.
///
/// Half-open `[start, end)`, in order, non-overlapping, and never inside a
/// token's own span — which is what keeps a `Token::Char` run, a
/// `Token::Literal` body and the whitespace the lexer folded into a
/// `Token::Space` out of reach.
///
/// One zero-length range is deliberately included: the gap in front of
/// `Token::Eoi`, which is empty exactly when the file ends without a trailing
/// newline — the case [`FormatOptions::insert_final_newline`] exists for.
fn program_gaps(atoms: &[Atom], len: usize) -> Vec<(usize, usize)> {
    let mut stack = AreaStack::new();
    let mut out = Vec::new();
    let mut cursor = 0usize;
    for a in atoms {
        let start = a.span.start.byte.max(cursor);
        let at_eoi = a.slot == Token::Eoi;
        if (start > cursor || at_eoi) && stack.current() == Area::Program {
            out.push((cursor, start));
        }
        stack.advance(&a.slot);
        cursor = a.span.end.byte.max(cursor);
    }
    // A lex that succeeded ends with `Eoi` at the end of the file, so the loop
    // has already covered the tail. Kept for the shape's sake: a caller that
    // ever hands over a token stream without one still gets a total answer
    // instead of a silently dropped tail.
    if cursor < len && stack.current() == Area::Program {
        out.push((cursor, len));
    }
    out
}

/// One line of a program-area gap: indentation (or inter-token space), an
/// optional `%` comment, and the line terminator that ends it.
///
/// A gap is exactly a sequence of these, because program mode skips only three
/// things — spaces, line breaks, and `%` to end of line (`lexer.rs`'s
/// `lex_program` and `skip_spaces`).
struct Line<'a> {
    /// Spaces and tabs at the start of this line of the gap.
    lead: &'a str,
    /// `%` through to just before the terminator, if the line carries one.
    comment: Option<&'a str>,
    /// `"\r\n"`, `"\n"`, `"\r"`, or `""` for the last line of the gap.
    term: &'a str,
}

/// Split a program-area gap into lines, or `None` if it holds anything the
/// lexer would not have skipped.
///
/// The `None` case should be unreachable — a program-mode gap is whitespace
/// and comments by construction — which is exactly why it is checked instead
/// of assumed: reaching it means [`crate::area`]'s replay has drifted from the
/// lexer, and the one thing a formatter must not do when it has misread a file
/// is edit it anyway.
fn split_gap(gap: &str) -> Option<Vec<Line<'_>>> {
    let mut out = Vec::new();
    let mut rest = gap;
    loop {
        let lead_end = rest
            .find(|c: char| c != ' ' && c != '\t')
            .unwrap_or(rest.len());
        let (lead, tail) = rest.split_at(lead_end);

        let (comment, tail) = match tail.starts_with('%') {
            true => {
                let end = tail.find(['\n', '\r']).unwrap_or(tail.len());
                (Some(&tail[..end]), &tail[end..])
            }
            false => (None, tail),
        };

        let (term, tail) = if let Some(t) = tail.strip_prefix("\r\n") {
            ("\r\n", t)
        } else if let Some(t) = tail.strip_prefix('\n') {
            ("\n", t)
        } else if let Some(t) = tail.strip_prefix('\r') {
            ("\r", t)
        } else {
            ("", tail)
        };

        // Nothing but whitespace, comments and terminators may appear here.
        if term.is_empty() && !tail.is_empty() {
            return None;
        }

        out.push(Line {
            lead,
            comment,
            term,
        });
        if term.is_empty() {
            return Some(out);
        }
        rest = tail;
    }
}

/// Where in the file a gap sits. The same `"   \n"` is trailing whitespace
/// after a token, a blank line, or the end of the file depending only on this.
#[derive(Debug, Clone, Copy)]
struct Where {
    /// The gap begins at column 0, so its first line is a line of its own
    /// rather than the tail of the preceding token's line.
    at_line_start: bool,
    /// The gap begins at byte 0. Blank lines here are the file's leading ones
    /// and none of them stay, whatever [`FormatOptions::max_blank_lines`] says.
    at_file_start: bool,
    /// The gap ends at the end of the file, so its last line is whitespace
    /// after the final token rather than the indentation of the next one.
    at_file_end: bool,
}

/// Rewrite one program-area gap.
fn rewrite_gap(gap: &str, at: Where, newline: &str, opts: &FormatOptions) -> Option<String> {
    let Where {
        at_line_start,
        at_file_start,
        at_file_end,
    } = at;
    let lines = split_gap(gap)?;
    let last = lines.len() - 1;

    // What each line of the gap contributes, terminator excluded.
    let rendered: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            // The first line of a gap continues the line the previous token is
            // on, so its leading run is *inter-token* space — possibly hand
            // alignment before an end-of-line comment — rather than
            // indentation. Every later line of the gap starts at a column the
            // author chose as indentation, and so does the first when the gap
            // itself begins a line.
            let is_indentation = i > 0 || at_line_start;
            let lead = match is_indentation {
                true => normalise_indent(line.lead, opts),
                false => line.lead.to_string(),
            };
            match line.comment {
                // A comment keeps its own text; only what trails it goes.
                Some(c) => format!("{lead}{}", rtrim(c)),
                // The last line of the gap has no terminator, so its leading
                // run is the indentation of the token that follows — unless
                // there is no such token, in which case it is whitespace at
                // the end of the file.
                None if i == last && !at_file_end => lead,
                // Everything else is trailing whitespace or a blank line.
                None if !opts.trim_trailing_whitespace => line.lead.to_string(),
                None => String::new(),
            }
        })
        .collect();

    let mut out = String::new();
    let mut blank_run = 0usize;
    let mut seen_content = false;
    for (i, (line, text)) in lines.iter().zip(&rendered).enumerate() {
        // A *blank line* is an empty line that ends. The first line of a gap
        // is not one even when it renders empty: it is the tail of the line
        // the previous token sits on, and its terminator ends that line rather
        // than an empty one — unless the gap started a line of its own.
        let is_blank = text.is_empty() && !line.term.is_empty() && (i > 0 || at_line_start);
        if is_blank {
            blank_run += 1;
            // Nothing has been written yet and the gap starts the file, so
            // this run is the file's leading blank lines. None of them stay.
            let cap = match at_file_start && !seen_content {
                true => 0,
                false => opts.max_blank_lines,
            };
            if blank_run > cap {
                continue;
            }
        } else {
            blank_run = 0;
            seen_content |= !text.is_empty();
        }
        out.push_str(text);
        out.push_str(line.term);
    }

    if at_file_end {
        if opts.trim_final_newlines {
            // Leave exactly one terminator: strip while what remains still
            // ends in one.
            loop {
                let keep = match strip_terminator(&out) {
                    Some(shorter) if ends_with_terminator(shorter) => shorter.len(),
                    _ => break,
                };
                out.truncate(keep);
            }
        }
        if opts.insert_final_newline && !ends_with_terminator(&out) {
            out.push_str(newline);
        }
    }
    Some(out)
}

/// Replace tabs in an indentation run by the spaces they stood for.
///
/// Column-based rather than a flat `tab_size` spaces each, so a mixed
/// `"\t  \t"` lands where the author saw it land. Returns the run untouched
/// when the client wants tabs, or when there are none — which is what keeps a
/// three-space indent at three spaces: this expands tabs, it does not
/// re-indent.
fn normalise_indent(lead: &str, opts: &FormatOptions) -> String {
    if !opts.insert_spaces || !lead.contains('\t') {
        return lead.to_string();
    }
    let tab = opts.tab_size.max(1);
    let mut col = 0usize;
    for c in lead.chars() {
        col = match c {
            '\t' => col + tab - (col % tab),
            _ => col + 1,
        };
    }
    " ".repeat(col)
}

fn rtrim(s: &str) -> &str {
    s.trim_end_matches([' ', '\t'])
}

fn ends_with_terminator(s: &str) -> bool {
    s.ends_with('\n') || s.ends_with('\r')
}

fn strip_terminator(s: &str) -> Option<&str> {
    s.strip_suffix("\r\n")
        .or_else(|| s.strip_suffix('\n'))
        .or_else(|| s.strip_suffix('\r'))
}

/// Which line terminator to write when one has to be *invented* — which
/// happens in exactly one place, the final newline of a file that ends without
/// one.
///
/// Taken from the first terminator in the file rather than from a majority
/// vote: every terminator already in the text is copied through untouched, so
/// this only has to avoid being the odd one out in a file that is consistent,
/// and a file that is not consistent has no right answer to find.
fn dominant_newline(source: &str) -> &'static str {
    match source.find(['\n', '\r']) {
        Some(i) if source[i..].starts_with("\r\n") => "\r\n",
        Some(i) if source[i..].starts_with('\r') => "\r",
        _ => "\n",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(src: &str) -> String {
        format(src, RustyfiVersion::V0_0, &FormatOptions::default()).expect("a formattable buffer")
    }

    #[test]
    fn a_gap_splits_into_indentation_comments_and_terminators() {
        let lines = split_gap("  % one\n\t\n  ").expect("a well-formed gap");
        assert_eq!(lines.len(), 3);
        assert_eq!((lines[0].lead, lines[0].comment), ("  ", Some("% one")));
        assert_eq!((lines[1].lead, lines[1].comment), ("\t", None));
        assert_eq!((lines[2].lead, lines[2].term), ("  ", ""));
    }

    #[test]
    fn a_gap_holding_anything_the_lexer_would_not_skip_is_refused() {
        // Unreachable through `program_gaps`; the point is that reaching it
        // declines rather than editing a file this code has misread.
        assert!(split_gap("  let  ").is_none());
    }

    #[test]
    fn tabs_expand_to_the_column_they_occupied() {
        let opts = FormatOptions {
            tab_size: 4,
            ..FormatOptions::default()
        };
        assert_eq!(normalise_indent("\t", &opts), "    ");
        assert_eq!(normalise_indent("\t\t", &opts), "        ");
        // A tab after two spaces advances to the next stop, not by four.
        assert_eq!(normalise_indent("  \t", &opts), "    ");
        // Spaces alone are returned untouched: this is not a re-indenter.
        assert_eq!(normalise_indent("   ", &opts), "   ");
    }

    #[test]
    fn tabs_survive_a_client_that_asked_for_tabs() {
        let opts = FormatOptions {
            insert_spaces: false,
            ..FormatOptions::default()
        };
        assert_eq!(normalise_indent("\t\t", &opts), "\t\t");
    }

    #[test]
    fn interior_space_is_never_touched_because_it_is_usually_alignment() {
        let src = "let alpha   = 1\nlet b       = 2\n";
        assert_eq!(fmt(src), src);
    }

    #[test]
    fn a_missing_final_newline_is_added_and_extra_ones_removed() {
        assert_eq!(fmt("let x = 1"), "let x = 1\n");
        assert_eq!(fmt("let x = 1\n\n\n\n"), "let x = 1\n");
    }

    #[test]
    fn a_crlf_file_keeps_crlf_when_a_newline_has_to_be_invented() {
        assert_eq!(fmt("let x = 1\r\nlet y = 2"), "let x = 1\r\nlet y = 2\r\n");
    }
}
