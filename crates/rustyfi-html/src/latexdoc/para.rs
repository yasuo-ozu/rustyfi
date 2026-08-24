//! The paragraph under construction, and how it becomes LaTeX.
//!
//! ## Why a paragraph is not built as a string
//!
//! Exactly the reason the Markdown backend gives, and it bites harder here.
//! Whether a fixed-pitch run is `\texttt{…}` or a line inside a
//! `\begin{verbatim}` depends on whether the REST of the paragraph is
//! fixed-pitch too, which is not known until the paragraph ends
//! ([`Para::is_code`]) — and the two spellings escape their content in
//! OPPOSITE directions. `\texttt{100%}` needs the `%` escaped or the rest of
//! the line vanishes; a `verbatim` line needs it left exactly as it is, since
//! escaping there would print a literal backslash.
//!
//! So a paragraph accumulates [`Piece`]s and [`Para::render`] decides once,
//! at the end, which of the two it is. The same deferral is what lets a code
//! block's INDENTATION survive: a leading gap arrives as a width in points
//! ([`Piece::Gap`]), and only at the end is the fixed-pitch advance known
//! that turns it back into a count of columns.
//!
//! ## Escaping happens here, not at the point of emission
//!
//! [`Piece::Text`] holds the document's own characters, unescaped. LaTeX this
//! backend GENERATED — a `\href{`, an `\emph{`, a `$…$` — is a different kind
//! ([`Piece::Markup`] and friends) and is never escaped. Keeping the two
//! apart in the buffer is what makes it impossible to double-escape a
//! generated brace or to leave a document's own `%` bare.

use std::fmt::Write as _;

use super::escape;

/// Where a link points.
pub(super) enum LinkTarget {
    /// An external URI: `\href{…}{…}`.
    Uri(String),
    /// A `\ref` to a destination the document registered:
    /// `\hyperlink{…}{…}`. The Markdown backend can only write these as
    /// plain text, because Markdown has no document-wide anchor scheme;
    /// LaTeX does, so a cross-reference stays a link.
    Goto(String),
}

/// One fragment of a paragraph under construction.
pub(super) enum Piece {
    /// The document's own characters. `mono` records that the run was set in
    /// a fixed-pitch face, which decides both whether this is code and — if
    /// the whole paragraph is — how the gaps around it are measured.
    Text { s: String, mono: bool },
    /// A horizontal gap inside fixed-pitch text, in points.
    ///
    /// Not stored as spaces, because how many spaces it is cannot be known
    /// until the paragraph's own character advance is (see [`Para::render`]).
    /// This is where a `+code` block's indentation lives: `code.satyh` emits
    /// it as `inline-skip (charwid *' float i)`, an `inline-skip` of exactly
    /// `i` character widths.
    Gap(f64),
    /// A `VertBox::Line` boundary. `hard` says the line ended with an
    /// `inline-fil`, i.e. that the break is one the AUTHOR wrote —
    /// `code.satyh` ends every source line that way. A SOFT break is the
    /// paragraph breaker's own decision at a measure the reader never chose,
    /// and is rejoined even inside a code block.
    ///
    /// A soft break writes nothing at all: the word space that stands in for
    /// it comes from the glue rule, which is the only thing that knows
    /// whether there was one (between two CJK characters, or across a
    /// hyphenation point, there was not).
    Newline { hard: bool },
    /// LaTeX this backend generated. `plain` is what to write instead when
    /// the paragraph turns out to be a code block, where the markup would be
    /// printed literally.
    Markup { tex: String, plain: String },
    /// An emphasis or link wrapper. Kept as OPEN/CLOSE pairs rather than
    /// written inline so that an empty one can be withdrawn — a `\href`
    /// whose content is a drawn bullet really does end up with nothing
    /// between the braces, and `\href{u}{}` is an invisible, unclickable
    /// link.
    Open(&'static str),
    Close,
    LinkOpen(LinkTarget),
}

/// A finished paragraph: the LaTeX it became, and whether that LaTeX is the
/// CONTENTS of a `verbatim` rather than a block in its own right.
pub(super) struct Rendered {
    pub(super) text: String,
    pub(super) code: bool,
}

/// A paragraph being accumulated. The block walker owns one; table cells,
/// footnote bodies and `draw-text` labels each get their own.
#[derive(Default)]
pub(super) struct Para {
    pub(super) pieces: Vec<Piece>,
    /// Whether anything at all has been started here — distinguishes "no
    /// paragraph yet" (nothing to flush) from "a paragraph so far containing
    /// only spacing".
    pub(super) open: bool,
    /// The outline level of the destination frame found on this paragraph's
    /// lines, if any — see `crate::recover::find_heading_level`.
    pub(super) heading_level: Option<i64>,
    /// The destination name that heading was registered under, so the
    /// heading can carry a `\hypertarget` a `\ref` elsewhere can reach.
    pub(super) heading_dest: Option<String>,
    /// Every text run so far was fixed-pitch, and there was at least one.
    pub(super) mono: bool,
    /// A proportional run has appeared, which disqualifies [`Para::mono`].
    pub(super) mixed: bool,
    /// At least one run was fixed-pitch, whatever the others were.
    pub(super) has_mono: bool,
    /// `VertBox::Line`s seen, and how many ended with an `inline-fil` — see
    /// [`Para::is_code`], where the pair is the test.
    pub(super) lines: usize,
    pub(super) fil_lines: usize,
}

impl Para {
    /// Append document text, merging into the previous piece when it is text
    /// of the same kind. Merging matters: the box stream splits a Japanese
    /// phrase into one run per character and a hyphenatable English word into
    /// one per chunk, and a `\texttt{}` per chunk would be unreadable.
    pub(super) fn push_text(&mut self, s: &str, mono: bool) {
        if s.is_empty() {
            return;
        }
        if let Some(Piece::Text { s: last, mono: m }) = self.pieces.last_mut() {
            if *m == mono {
                last.push_str(s);
                return;
            }
        }
        self.pieces.push(Piece::Text {
            s: s.to_string(),
            mono,
        });
    }

    pub(super) fn push_markup(&mut self, tex: impl Into<String>, plain: impl Into<String>) {
        self.pieces.push(Piece::Markup {
            tex: tex.into(),
            plain: plain.into(),
        });
    }

    /// Reset for the next paragraph.
    pub(super) fn clear(&mut self) {
        self.pieces.clear();
        self.open = false;
        self.heading_level = None;
        self.heading_dest = None;
        self.mono = false;
        self.mixed = false;
        self.has_mono = false;
        self.lines = 0;
        self.fil_lines = 0;
    }

    /// Is this paragraph a code block — one whose line breaks are the
    /// AUTHOR's and whose whitespace is significant?
    ///
    /// The obvious test, "every run is fixed-pitch", is not enough, and this
    /// is the Markdown backend's finding rather than a fresh one: a `+code`
    /// block containing any Japanese fails it, because a fixed-pitch Latin
    /// face has no CJK glyphs and SATySFi sets those characters in the
    /// document's own gothic/mincho face, so the paragraph reads as MIXED. In
    /// `latexcmds`' manual, whose code samples are full of Japanese string
    /// literals, that is most of the code blocks in the document.
    ///
    /// The reliable signal is structural: `code.satyh` builds a block as ONE
    /// `line-break` over a sequence of
    /// `inline-skip ++ line ++ inline-fil ++ discretionary`, one per source
    /// line, so EVERY line of a code block ends with an `inline-fil`. A
    /// justified prose paragraph ends only its LAST line that way.
    ///
    /// A single line cannot be told apart this way, so a one-line paragraph
    /// falls back to the all-fixed-pitch test. The count is a MAJORITY rather
    /// than "all", because a code line too long for the measure is broken by
    /// the paragraph breaker like any other and ends at a hyphenation point
    /// instead of at its fil.
    pub(super) fn is_code(&self) -> bool {
        self.mono || (self.lines >= 2 && self.has_mono && self.fil_lines * 2 > self.lines)
    }

    pub(super) fn note_line(&mut self, ended_with_fil: bool) {
        self.lines += 1;
        if ended_with_fil {
            self.fil_lines += 1;
        }
    }

    /// Does the last text written end in a hyphen? Asked at a line boundary,
    /// where `crate::recover::line_join` needs to know whether there is a
    /// hyphen at all before deciding whose it is.
    pub(super) fn ends_with_hyphen(&self) -> bool {
        match self.pieces.last() {
            Some(Piece::Text { s, .. }) => {
                s.chars().next_back().is_some_and(crate::recover::is_hyphen)
            }
            _ => false,
        }
    }

    /// Delete the hyphen the LINE BREAKER inserted at the end of the line
    /// just closed, so the word it split comes back together.
    pub(super) fn drop_break_hyphen(&mut self) {
        if let Some(Piece::Text { s, .. }) = self.pieces.last_mut() {
            if s.chars().next_back().is_some_and(crate::recover::is_hyphen) {
                s.pop();
            }
        }
    }

    /// This paragraph as LaTeX, or `None` when it holds nothing a reader
    /// would see.
    ///
    /// `advance` is the fixed-pitch character width (pt) observed in this
    /// document, used to turn a [`Piece::Gap`] back into a column count
    /// inside a code block.
    ///
    /// A code block comes back WITHOUT its `verbatim` wrapper, flagged
    /// [`Rendered::code`]: the writer keeps consecutive ones together in a
    /// single environment, which it cannot do once they are already wrapped.
    pub(super) fn render(&self, advance: Option<f64>) -> Option<Rendered> {
        if !self.open {
            return None;
        }
        let code = self.is_code();
        let body = if code {
            self.render_code(advance)
        } else {
            self.render_prose()
        };
        let trimmed = body.trim_matches(|c| c == ' ' || c == '\n');
        if trimmed.is_empty() {
            return None;
        }
        if code {
            return Some(Rendered {
                text: trimmed.to_string(),
                code: true,
            });
        }
        let one_line = collapse_spaces(trimmed);
        let text = match self.heading_level {
            Some(level) => heading(crate::recover::heading_depth(level), &one_line, self.heading_dest.as_deref()),
            None => one_line,
        };
        Some(Rendered { text, code: false })
    }

    /// The paragraph as flowing LaTeX prose.
    fn render_prose(&self) -> String {
        let mut out = String::new();
        // How many wrappers are open, so an unterminated one can be closed
        // rather than swallowing the rest of the document.
        let mut open: Vec<(usize, usize)> = Vec::new();
        let mut i = 0;
        while i < self.pieces.len() {
            // A maximal run of fixed-pitch text becomes ONE `\texttt{…}`, so
            // a `\code{foo bar}` is `\texttt{foo bar}` and not two boxes with
            // a bare space between them.
            let end = self.mono_run_end(i);
            if end > i {
                let (lead, body, trail) = mono_run_text(&self.pieces[i..end]);
                if lead {
                    out.push(' ');
                }
                if !body.is_empty() {
                    let _ = write!(out, "\\texttt{{{}}}", escape::text(&body));
                }
                if trail {
                    out.push(' ');
                }
                i = end;
                continue;
            }
            match &self.pieces[i] {
                Piece::Text { s, .. } => out.push_str(&escape::text(s)),
                // Outside a code block a gap is just a word space; its exact
                // width is the line breaker's business, not the reader's.
                Piece::Gap(_) => out.push(' '),
                // Nothing: the word space that replaces a rejoined line break
                // was already pushed as text by the glue rule, which is where
                // it has to be decided, because between two CJK characters
                // there must not be one.
                Piece::Newline { .. } => {}
                Piece::Markup { tex, .. } => out.push_str(tex),
                Piece::Open(cmd) => {
                    let cmd_at = out.len();
                    out.push_str(cmd);
                    open.push((cmd_at, out.len()));
                }
                Piece::LinkOpen(target) => {
                    let cmd_at = out.len();
                    match target {
                        LinkTarget::Uri(u) => {
                            let _ = write!(out, "\\href{{{}}}{{", escape::url(u));
                        }
                        LinkTarget::Goto(name) => {
                            let _ = write!(out, "\\hyperlink{{{}}}{{", escape::dest_name(name));
                        }
                    }
                    open.push((cmd_at, out.len()));
                }
                Piece::Close => match open.pop() {
                    // Nothing but spacing between the braces: withdraw the
                    // whole wrapper. `\href{u}{}` is an invisible link and
                    // `\emph{}` is a stray, and a `\href` whose content was a
                    // drawn bullet produces exactly that.
                    Some((cmd_at, body_at)) if out[body_at..].trim().is_empty() => {
                        out.truncate(cmd_at);
                    }
                    Some((cmd_at, body_at)) => close_wrapper(&mut out, cmd_at, body_at),
                    // An unmatched close — the marker pairs are stdlib-paired
                    // so this should not happen; a stray `}` would be a hard
                    // error, so it is dropped.
                    None => {}
                },
            }
            i += 1;
        }
        // Whatever is still open closes here rather than at the end of the
        // document.
        for _ in 0..open.len() {
            out.push('}');
        }
        out
    }

    /// The paragraph as the literal contents of a `verbatim`: the document's
    /// own characters, its own line breaks, and its own indentation.
    fn render_code(&self, advance: Option<f64>) -> String {
        let mut out = String::new();
        for piece in &self.pieces {
            match piece {
                Piece::Text { s, .. } => out.push_str(s),
                Piece::Gap(pt) => {
                    for _ in 0..gap_spaces(*pt, advance) {
                        out.push(' ');
                    }
                }
                // A SOFT break is one the paragraph breaker made because the
                // line ran out of room, not one the author wrote.
                Piece::Newline { hard: false } => {}
                Piece::Newline { hard: true } => {
                    // Trailing spaces on a code line are invisible noise, and
                    // the line-end `inline-fil` reliably produces some.
                    while out.ends_with(' ') {
                        out.pop();
                    }
                    out.push('\n');
                }
                // Inside a `verbatim` there is no markup, only characters —
                // so a link contributes its text and an image its label.
                Piece::Markup { plain, .. } => out.push_str(plain),
                Piece::Open(_) | Piece::Close | Piece::LinkOpen(_) => {}
            }
        }
        out
    }

    /// The end of the maximal run of fixed-pitch pieces starting at `i`, or
    /// `i` itself when the piece there is not one.
    ///
    /// Gaps and rejoined line breaks are absorbed into the run, so a
    /// `\texttt` the box stream split across a line boundary comes back as
    /// ONE. A run of nothing but spacing is not code and is declined, or a
    /// stray gap between two ordinary words would come out in a typewriter
    /// face.
    fn mono_run_end(&self, i: usize) -> usize {
        let is_run_piece = |p: Option<&Piece>| {
            matches!(
                p,
                Some(Piece::Text { mono: true, .. })
                    | Some(Piece::Gap(_))
                    | Some(Piece::Newline { .. })
            )
        };
        if !is_run_piece(self.pieces.get(i)) {
            return i;
        }
        let mut end = i;
        while is_run_piece(self.pieces.get(end)) {
            end += 1;
        }
        let has_ink = self.pieces[i..end]
            .iter()
            .any(|p| matches!(p, Piece::Text { s, .. } if !s.trim().is_empty()));
        if has_ink {
            end
        } else {
            i
        }
    }
}

/// Close the wrapper whose opening command ends at `start`, moving any space
/// that ended up inside the braces to the OUTSIDE of them.
///
/// The word space before a wrapper is PENDING when the wrapper opens — the
/// glue rule cannot settle it until the character that follows is known, and
/// that character arrives after `\href{url}{` has been written. Left where it
/// falls, `a \href{u}{link}` comes out as `a\href{u}{ link}`: the space moves
/// from between the words to inside the link, so the underline starts a space
/// early and the word before it runs into the link text. The Markdown backend
/// has the same problem and the same fix (`close_link`); it shows up here on
/// the first `\href` of the plain-text fixture.
fn close_wrapper(out: &mut String, cmd_at: usize, body_at: usize) {
    let lead = out[body_at..].len() - out[body_at..].trim_start_matches(' ').len();
    if lead > 0 {
        // Swap the opening command and the run of spaces after it. Spaces are
        // one ASCII byte, so this arithmetic is the same in bytes and
        // characters.
        let cmd = out[cmd_at..body_at].to_string();
        out.replace_range(cmd_at..body_at + lead, &format!("{}{cmd}", " ".repeat(lead)));
    }
    let trail = out.len() - out.trim_end_matches(' ').len();
    out.truncate(out.len() - trail);
    out.push('}');
    for _ in 0..trail.min(1) {
        out.push(' ');
    }
}

/// A heading at `depth`, with the anchor a `\ref` elsewhere in the document
/// reaches it by.
///
/// **Starred forms, always.** The document has already typeset its own
/// numbering into the title — `stdjabook`'s `section-scheme` writes
/// `1. Introduction`, numbering it from its own counter — so an unstarred
/// `\section` would number it a second time and produce `1 1. Introduction`.
/// It also means no entry is added to a `\tableofcontents`, which is right
/// for the same reason: a document that wants a contents page TYPESETS one,
/// and it is already in the flow.
///
/// LaTeX's own hierarchy runs out at `\subparagraph`, so depths 5 and 6
/// share it. That is one level of flattening at a depth `article` does not
/// really have either.
fn heading(depth: u8, body: &str, dest: Option<&str>) -> String {
    let cmd = match depth {
        1 => "\\section*",
        2 => "\\subsection*",
        3 => "\\subsubsection*",
        4 => "\\paragraph*",
        _ => "\\subparagraph*",
    };
    match dest {
        // `\hypertarget` around the TITLE rather than before it: an anchor on
        // its own line before a sectioning command is a paragraph, which
        // `\section*` would then have to break out of.
        Some(name) => format!(
            "{cmd}{{\\hypertarget{{{}}}{{}}{body}}}",
            escape::dest_name(name)
        ),
        None => format!("{cmd}{{{body}}}"),
    }
}

/// A fixed-pitch run's text, plus whether it carried whitespace on either
/// edge that belongs OUTSIDE the `\texttt{}`.
///
/// The edges matter: a space inside the braces is set in the typewriter face
/// at typewriter width, so `use \texttt{ x } now` has visibly wrong spacing.
fn mono_run_text(run: &[Piece]) -> (bool, String, bool) {
    let mut body = String::new();
    for piece in run {
        match piece {
            Piece::Text { s, .. } => body.push_str(s),
            Piece::Gap(_) => body.push(' '),
            // Nothing, for the same reason as in prose: the word space that
            // replaces a rejoined line break comes from the GLUE, which is
            // the only thing that knows whether there was one.
            _ => {}
        }
    }
    let trimmed = body.trim();
    let lead = trimmed.len() < body.trim_end().len();
    let trail = trimmed.len() < body.trim_start().len();
    (lead, trimmed.to_string(), trail)
}

/// How many spaces a gap of `pt` points is, in a document whose fixed-pitch
/// character advances `advance` points.
///
/// `code.satyh` sizes both the leading indent and every inter-word space in
/// exact multiples of that advance (`set-space-ratio (charwid /' fontsize)`),
/// so this division is not an estimate — it recovers the source's own column
/// count. With no advance measured (a base-14 render, where no font file says
/// whether a face is fixed-pitch at all) it degrades to one space.
fn gap_spaces(pt: f64, advance: Option<f64>) -> usize {
    match advance {
        Some(a) if a > 0.0 => (pt / a).round().max(0.0) as usize,
        _ => usize::from(pt > 0.0),
    }
}

/// Collapse the runs of spaces a rejoined paragraph accumulates — a line
/// boundary and the glue on either side of it each contribute one — into
/// single spaces, and drop them at the edges.
///
/// This is not only tidiness. A BLANK line inside what this backend is about
/// to write as one paragraph starts a new one in LaTeX, and a line beginning
/// with spaces after a `\\` is a different thing again; folding the whole
/// paragraph onto one line makes both unreachable.
fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for c in s.chars() {
        let c = if c == '\n' { ' ' } else { c };
        if c == ' ' {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prose(pieces: Vec<Piece>) -> String {
        let para = Para {
            pieces,
            open: true,
            ..Para::default()
        };
        let rendered = para.render(None).expect("some content");
        assert!(!rendered.code, "expected prose, got code");
        rendered.text
    }

    fn t(s: &str) -> Piece {
        Piece::Text {
            s: s.into(),
            mono: false,
        }
    }

    #[test]
    fn document_text_is_escaped_and_generated_latex_is_not() {
        let out = prose(vec![Piece::Open("\\emph{"), t("50% off"), Piece::Close]);
        assert_eq!(out, "\\emph{50\\% off}");
    }

    /// A `\href` whose content is a drawn bullet leaves nothing between the
    /// braces, and an empty `\href` is an invisible unclickable link.
    #[test]
    fn an_empty_wrapper_is_withdrawn_entirely() {
        let out = prose(vec![
            t("see "),
            Piece::LinkOpen(LinkTarget::Uri("http://x/".into())),
            Piece::Close,
        ]);
        assert_eq!(out, "see");
    }

    /// An unterminated wrapper would swallow the rest of the document, so it
    /// closes at the end of its own paragraph.
    #[test]
    fn an_unclosed_wrapper_is_closed_at_the_paragraph_end() {
        let out = prose(vec![Piece::Open("\\emph{"), t("a")]);
        assert_eq!(out, "\\emph{a}");
    }

    /// A `\ref` is a real link here, which is the one thing this backend can
    /// say and the Markdown one cannot.
    #[test]
    fn a_cross_reference_becomes_a_hyperlink() {
        let out = prose(vec![
            Piece::LinkOpen(LinkTarget::Goto("sec:intro".into())),
            t("Section 1"),
            Piece::Close,
        ]);
        assert_eq!(out, "\\hyperlink{rustyfi:sec:intro}{Section 1}");
    }

    /// One `\texttt` for the whole run, not one per chunk the box stream
    /// happened to split the word into — and the surrounding word spaces stay
    /// outside it, where they are set in the body face.
    #[test]
    fn adjacent_fixed_pitch_runs_become_one_texttt_without_swallowing_spaces() {
        let out = prose(vec![
            t("use"),
            Piece::Gap(3.0),
            Piece::Text {
                s: "foo bar".into(),
                mono: true,
            },
            Piece::Gap(3.0),
            t("now"),
        ]);
        assert_eq!(out, "use \\texttt{foo bar} now");
    }

    /// The whole point of deferring: an all-monospace paragraph is a
    /// `verbatim`, its indentation recovered from the gap widths against the
    /// measured character advance, and its contents NOT escaped.
    #[test]
    fn an_all_monospace_paragraph_becomes_verbatim_content_kept_literal() {
        let para = Para {
            pieces: vec![
                Piece::Text {
                    s: "if x > 0: # 100%".into(),
                    mono: true,
                },
                Piece::Newline { hard: true },
                Piece::Gap(12.0),
                Piece::Text {
                    s: "return \\y".into(),
                    mono: true,
                },
            ],
            open: true,
            mono: true,
            ..Para::default()
        };
        let rendered = para.render(Some(3.0)).unwrap();
        assert!(rendered.code);
        assert_eq!(rendered.text, "if x > 0: # 100%\n    return \\y");
    }

    #[test]
    fn a_heading_is_written_at_its_outline_depth_with_an_anchor() {
        let para = Para {
            pieces: vec![t("1. Introduction")],
            open: true,
            heading_level: Some(0),
            heading_dest: Some("sec:intro".into()),
            ..Para::default()
        };
        assert_eq!(
            para.render(None).unwrap().text,
            "\\section*{\\hypertarget{rustyfi:sec:intro}{}1. Introduction}"
        );
        // Starred, always: the document typeset `1.` itself, and an
        // unstarred `\section` would number it again.
        let para = Para {
            pieces: vec![t("Detail")],
            open: true,
            heading_level: Some(1),
            ..Para::default()
        };
        assert_eq!(para.render(None).unwrap().text, "\\subsection*{Detail}");
    }

    /// A blank line inside what this writes as one paragraph would start a
    /// second one in LaTeX.
    #[test]
    fn a_rejoined_paragraph_is_one_line_with_no_double_spaces() {
        let out = prose(vec![
            t("a "),
            Piece::Newline { hard: false },
            t(" b"),
            Piece::Gap(3.0),
            Piece::Gap(3.0),
            t("c"),
        ]);
        assert_eq!(out, "a b c");
    }

    #[test]
    fn gap_spaces_recovers_the_source_column_count() {
        assert_eq!(gap_spaces(15.0, Some(5.0)), 3);
        assert_eq!(gap_spaces(0.0, Some(5.0)), 0);
        assert_eq!(gap_spaces(15.0, None), 1);
        assert_eq!(gap_spaces(0.0, None), 0);
    }
}
