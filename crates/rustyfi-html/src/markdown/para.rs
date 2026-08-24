//! The paragraph under construction, and how it becomes Markdown.
//!
//! ## Why a paragraph is not built as a string
//!
//! The HTML backend appends to a `String` as it walks, because HTML can say
//! everything locally: a run knows it is monospace and writes `<code>` there
//! and then. Markdown cannot. Whether a monospace run is `` `inline code` ``
//! or a line inside a ``` fence depends on whether the REST of the paragraph
//! is monospace too, which is not known until the paragraph ends — and the
//! two spellings escape their content in opposite ways (a code span's content
//! is literal; prose has every `*` and `_` escaped).
//!
//! So a paragraph accumulates [`Piece`]s, and [`Para::render`] decides once,
//! at the end, which of the two it is. The same deferral is what lets a
//! code block's INDENTATION survive: a leading gap arrives as a width in
//! points ([`Piece::Gap`]), and only at the end is the fixed-pitch advance
//! known that turns it back into a count of spaces.
//!
//! ## Escaping happens here, not at the point of emission
//!
//! [`Piece::Text`] holds the document's own characters, unescaped. Markdown
//! this backend GENERATED — a link's brackets, an emphasis delimiter, an
//! image reference — is a different kind ([`Piece::Markup`] and friends) and
//! is never escaped. Keeping the two apart in the buffer, rather than
//! escaping eagerly and hoping nothing escapes the escaper, is what makes it
//! impossible to double-escape a generated delimiter or to leave a document's
//! own asterisk bare.

use super::escape;

/// One fragment of a paragraph under construction.
pub(super) enum Piece {
    /// The document's own characters. `mono` records that the run was set in
    /// a fixed-pitch face, which decides both whether this is code and — if
    /// the whole paragraph is — how the gaps around it are measured.
    Text { s: String, mono: bool },
    /// A horizontal gap inside fixed-pitch text, in points.
    ///
    /// Not stored as spaces, because how many spaces it is cannot be known
    /// until the paragraph's own character advance is (see
    /// [`Para::render`]). This is where a `+code` block's indentation lives:
    /// `code.satyh` emits it as `inline-skip (charwid *' float i)`, an
    /// `inline-skip` of exactly `i` character widths, and the HTML backend
    /// loses it because glue collapses. A fence keeps whitespace, so here it
    /// is recovered exactly.
    Gap(f64),
    /// A `VertBox::Line` boundary.
    ///
    /// `hard` says the line ended with an `inline-fil`, i.e. that the break
    /// is one the AUTHOR wrote — `code.satyh` ends every source line that
    /// way. A line that ran out of room and was broken by the paragraph
    /// breaker instead ends with a hyphenation point or a word space, and is
    /// SOFT: even inside a code block it is rejoined, because reproducing it
    /// would fossilize a wrap at a page width the reader never chose. It is
    /// the one place a code block still gets the prose treatment, and the
    /// distinction is free — the fil is right there in the stream.
    ///
    /// A soft break writes nothing at all: the word space that stands in for
    /// it comes from the glue rule, which is the only thing that knows
    /// whether there was one (between two CJK characters, or across a
    /// hyphenation point, there was not).
    ///
    /// Emitted at every boundary rather than only where it will be used,
    /// because whether this paragraph is a code block is not decided until it
    /// ends — see [`Para::is_code`].
    Newline { hard: bool },
    /// Markdown this backend generated. `plain` is what to write instead when
    /// the paragraph turns out to be a code block, where the markup would be
    /// literal text.
    Markup { md: String, plain: String },
    /// An equation as LaTeX (`--katex`), still undelimited.
    ///
    /// Kept apart from [`Piece::Markup`] for one reason: whether it is written
    /// `$…$` or `$$…$$` is not a property of the equation but of the
    /// PARAGRAPH around it, and is therefore not knowable when the box is
    /// walked. An equation that is the whole of its paragraph was DISPLAYED —
    /// nothing else can be alone in a block — and every renderer that
    /// understands `$$` sets it centred, on its own line, with big operators
    /// carrying their limits above and below. An equation with prose beside it
    /// was inline. Deciding at render time, where the whole paragraph is
    /// visible, is the same deferral this module exists for; see
    /// [`Para::sole_math`].
    ///
    /// `plain` is the reading-order text, for a code fence — where `$x^2$`
    /// would be literal characters rather than an equation.
    Math { latex: String, plain: String },
    /// An equation as an inline `<svg>` (`--svg-math`, `--svg-outline-math`),
    /// in BOTH the shapes it may take.
    ///
    /// Two strings rather than one for the same reason [`Piece::Math`] holds
    /// undelimited LaTeX: whether the drawing may be broken across lines is a
    /// property of the PARAGRAPH, not of the equation, and is not known when
    /// the box is walked. `inline` is one line, safe anywhere; `block` is
    /// indented one element per line and is only legal where the drawing is
    /// its own HTML block. `crate::mathsvg::Wrap` has the CommonMark argument
    /// for why the distinction is forced rather than chosen.
    ///
    /// Both are built up front. The alternative — keeping the glyphs and
    /// rendering at flush time — would put a lifetime on [`Para`] and thread
    /// it through the whole block walker to save re-running a font lookup a
    /// few dozen times per document.
    MathSvg {
        inline: String,
        block: String,
        plain: String,
    },
    /// An equation as MathML Core (`--mathml`), as the CHILDREN of a `<math>`
    /// element that has not been written yet.
    ///
    /// The same deferral [`Piece::Math`] makes, for the same reason and one
    /// more. Whether the element says `display="block"` or `display="inline"`
    /// is a property of the PARAGRAPH — and it is not decoration: in block
    /// display a browser sets `math-style: normal`, which puts a big
    /// operator's limits above and below at full size. And whether the element
    /// may be broken across lines is the [`Piece::MathSvg`] question, with the
    /// same CommonMark answer.
    ///
    /// The body is ONE LINE in both shapes — `crate::mathml`'s doc comment has
    /// the measurement behind that, and the short version is that
    /// [`Para::render`]'s `collapse_spaces` folds a pretty-printed element back
    /// onto one line anyway, so emitting newlines only converts them into
    /// whitespace text nodes inside the MathML tree.
    ///
    /// `approx` is `crate::mathml::Approx::Approx` when this box drew ink the
    /// recovery could not account for; it decides the class on the `<math>`
    /// and combines with the other boxes' verdicts when they merge.
    MathMl {
        body: String,
        plain: String,
        approx: crate::mathml::Approx,
    },
    /// An emphasis delimiter. Kept distinct from [`Piece::Markup`] because a
    /// Markdown delimiter may not sit against the whitespace inside its own
    /// span (`* text *` is not emphasis) — see [`Para::render`], which moves
    /// the space out from under it.
    EmphOpen(&'static str),
    EmphClose(&'static str),
    /// A link's two halves. Kept distinct so an EMPTY link can be dropped
    /// rather than emitted as `[](…)`: a `\href` whose content is a drawn
    /// bullet or an unrecoverable graphic really does end up with nothing
    /// between the brackets.
    LinkOpen(String),
    LinkClose,
}

/// A finished paragraph: the Markdown it became, and whether that Markdown is
/// the CONTENTS of a code fence rather than a block in its own right.
pub(super) struct Rendered {
    pub(super) text: String,
    pub(super) code: bool,
}

/// A paragraph being accumulated. The block walker owns one; table cells and
/// footnote bodies each get their own.
#[derive(Default)]
pub(super) struct Para {
    pub(super) pieces: Vec<Piece>,
    /// Whether anything at all has been started here — distinguishes "no
    /// paragraph yet" (nothing to flush) from "a paragraph so far containing
    /// only spacing".
    pub(super) open: bool,
    /// The outline level of the destination frame found on this paragraph's
    /// lines, if any — see `crate::recover::find_heading_level`. First match
    /// wins and is never un-decided by a later box on the same line.
    pub(super) heading_level: Option<i64>,
    /// Every text run so far was fixed-pitch, and there was at least one.
    pub(super) mono: bool,
    /// A proportional run has appeared, which disqualifies [`Para::mono`] for
    /// good.
    pub(super) mixed: bool,
    /// At least one run was fixed-pitch, whatever the others were.
    pub(super) has_mono: bool,
    /// `VertBox::Line`s seen, and how many of them ended with an
    /// `inline-fil` — see [`Para::is_code`], where the pair is the test.
    pub(super) lines: usize,
    pub(super) fil_lines: usize,
}

impl Para {
    /// Append document text, merging into the previous piece when it is text
    /// of the same kind. Merging matters: the box stream splits a Japanese
    /// phrase into one run per character and a hyphenatable English word into
    /// one per chunk, and a code span per chunk would be unreadable.
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

    pub(super) fn push_markup(&mut self, md: impl Into<String>, plain: impl Into<String>) {
        self.pieces.push(Piece::Markup {
            md: md.into(),
            plain: plain.into(),
        });
    }

    /// Reset for the next paragraph. `heading_level` and the monospace flags
    /// are per-paragraph facts and go with it.
    pub(super) fn clear(&mut self) {
        self.pieces.clear();
        self.open = false;
        self.heading_level = None;
        self.mono = false;
        self.mixed = false;
        self.has_mono = false;
        self.lines = 0;
        self.fil_lines = 0;
    }

    /// Is this paragraph a code block — one whose line breaks are the
    /// AUTHOR's and whose whitespace is significant?
    ///
    /// The obvious test, "every run is fixed-pitch", is the one the HTML
    /// backend uses and it is not enough. A `+code` block containing any
    /// Japanese at all fails it: a fixed-pitch Latin face has no CJK glyphs,
    /// so SATySFi sets those characters in the document's own gothic/mincho
    /// face, and the paragraph reads as MIXED. In `latexcmds`' manual, whose
    /// code samples are full of Japanese string literals, that was most of
    /// the code blocks in the document — each one arriving as a row of
    /// disconnected inline code spans.
    ///
    /// The reliable signal is structural, and it comes from how `code.satyh`
    /// builds the block: ONE `line-break` over a sequence of
    /// `inline-skip ++ line ++ inline-fil ++ discretionary`, one per source
    /// line. So EVERY line of a code block ends with an `inline-fil`. A
    /// justified prose paragraph ends only its LAST line that way — that fil
    /// is how `read-inline ctx {..} ++ inline-fil` fills the final line — so
    /// the two are told apart by counting, not by guessing.
    ///
    /// A single line cannot be told apart this way (one line is always
    /// "all its lines"), so a one-line paragraph falls back to the
    /// all-fixed-pitch test — which is exactly right for it, and is kept as
    /// an alternative at every length: an all-fixed-pitch paragraph is
    /// unambiguously code whatever its fils say.
    ///
    /// The count is a MAJORITY, not "all", because a code line too long for
    /// the measure is broken by the paragraph breaker like any other, and
    /// that line ends at a hyphenation point rather than at its fil. One
    /// overflowing line in `xpath`'s API listing was enough to make the whole
    /// block prose under an "all" test.
    pub(super) fn is_code(&self) -> bool {
        self.mono || (self.lines >= 2 && self.has_mono && self.fil_lines * 2 > self.lines)
    }

    /// Record that a `VertBox::Line` just ended, and whether it ended with an
    /// `inline-fil`.
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

    /// This paragraph as Markdown, or `None` when it holds nothing a reader
    /// would see.
    ///
    /// `advance` is the fixed-pitch character width (pt) observed in this
    /// document, used to turn a [`Piece::Gap`] back into a count of spaces
    /// inside a code block.
    ///
    /// A code block comes back WITHOUT its fence, flagged
    /// [`Rendered::code`]: the writer keeps consecutive ones together in a
    /// single fence, which it cannot do once they are already wrapped.
    /// `in_cell` says this paragraph is a table cell's content rather than a
    /// block of its own, which decides whether a lone equation may be
    /// upgraded to a display block or pretty-printed — see
    /// [`Para::display_math`].
    pub(super) fn render(&self, advance: Option<f64>, in_cell: bool) -> Option<Rendered> {
        if !self.open {
            return None;
        }
        let code = self.is_code();
        let body = if code {
            self.render_code(advance)
        } else {
            self.render_prose(in_cell)
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
            Some(level) => {
                let depth = crate::recover::heading_depth(level) as usize;
                // The `#` prefix means the content is no longer in the
                // leading position, so it needs no `escape::line_start`.
                format!("{} {}", "#".repeat(depth), one_line)
            }
            None => escape::line_start(&one_line),
        };
        Some(Rendered { text, code: false })
    }

    /// Is this paragraph nothing but ONE equation — i.e. was the equation
    /// DISPLAYED rather than set inside a sentence?
    ///
    /// Nothing in the box stream says "display style" directly: a displayed
    /// equation is a `line-break` over an inline sequence like any other, and
    /// what makes it displayed is that the sequence holds nothing else. So
    /// this is the signal, and it is exact rather than a heuristic — a
    /// paragraph whose only ink is one math box cannot have been part of a
    /// sentence.
    ///
    /// Everything without ink is ignored: the `inline-fil`s that centre the
    /// equation, the glue around it, the line boundary, and an emphasis or
    /// link wrapper that contributes no characters of its own.
    ///
    /// **Several math pieces still count as one displayed equation**, and that
    /// is the case worth explaining. A formula is not one box: `latexcmds`'
    /// Schrödinger equation reaches this backend as FOUR, because each
    /// `\underset`-style construction splits the run. They are pieces of one
    /// equation, so they are written into one `$$…$$` rather than four — four
    /// separate display blocks would be four centred lines where the document
    /// has one, and four inline `$…$` would leave a displayed equation set in
    /// the middle of a paragraph.
    ///
    /// Returns the pieces' LaTeX in order, or `None` when the paragraph holds
    /// anything else. A LINK anywhere declines: its brackets have to be
    /// written around the content, and a display block cannot be a link's
    /// text.
    ///
    /// `in_cell` declines outright. A table cell whose only content is an
    /// equation is not a displayed equation — it is a cell, and the paragraph
    /// machinery simply has no other content to look at. Upgrading it puts a
    /// centred `$$…$$` block inside a `|` row, which every renderer sets as a
    /// full-width display block that breaks the table; five rows of
    /// `easytable`'s own manual are this shape. "Alone in its block" only
    /// means "displayed" when the block is a paragraph.
    fn display_math(&self, in_cell: bool) -> Option<Vec<&str>> {
        if in_cell {
            return None;
        }
        let mut maths = Vec::new();
        for piece in &self.pieces {
            match piece {
                Piece::Math { latex, .. } => maths.push(latex.as_str()),
                Piece::Text { s, .. } if !s.trim().is_empty() => return None,
                Piece::Markup { md, .. } if !md.trim().is_empty() => return None,
                Piece::MathSvg { .. } | Piece::MathMl { .. } | Piece::LinkOpen(_) => {
                    return None
                }
                _ => {}
            }
        }
        (!maths.is_empty()).then_some(maths)
    }

    /// [`Para::display_math`]'s question for `--mathml`: is this paragraph
    /// nothing but MathML equations, and if so what are all their children and
    /// the combined verdict on their ink?
    ///
    /// Several boxes still make ONE displayed equation, exactly as they do for
    /// `--katex` — `latexcmds`' Schrödinger equation reaches this backend as
    /// four boxes — so the children are concatenated into a single `<math
    /// display="block">` rather than each getting an element. Four block
    /// elements would be four centred lines where the document has one.
    ///
    /// A cell declines for [`Para::display_math`]'s reason. It is not about
    /// line breaks here — the element is one line either way — but about what
    /// `display="block"` MEANS: a browser makes it a block-level box, which
    /// inside a `|` row is a full-width band that breaks the table, exactly as
    /// a `$$…$$` does.
    fn display_math_ml(&self, in_cell: bool) -> Option<(String, crate::mathml::Approx)> {
        if in_cell {
            return None;
        }
        let mut body = String::new();
        let mut any = false;
        let mut approx = crate::mathml::Approx::Exact;
        for piece in &self.pieces {
            match piece {
                Piece::MathMl {
                    body: b, approx: a, ..
                } => {
                    body.push_str(b);
                    approx = approx.or(*a);
                    any = true;
                }
                Piece::Text { s, .. } if !s.trim().is_empty() => return None,
                Piece::Markup { md, .. } if !md.trim().is_empty() => return None,
                Piece::MathSvg { .. } | Piece::Math { .. } | Piece::LinkOpen(_) => return None,
                _ => {}
            }
        }
        any.then_some((body, approx))
    }

    /// Is this paragraph one drawn equation and nothing else — i.e. may its
    /// `<svg>` be pretty-printed?
    ///
    /// Same question as [`Para::display_math`] asks for LaTeX, and the same
    /// answer for a cell: a drawing inside a `|` row must stay on one line,
    /// because a table cell is not its own HTML block and a newline inside one
    /// ends the row.
    fn display_svg(&self, in_cell: bool) -> bool {
        if in_cell {
            return false;
        }
        let mut svgs = 0usize;
        for piece in &self.pieces {
            match piece {
                Piece::MathSvg { .. } => svgs += 1,
                Piece::Text { s, .. } if !s.trim().is_empty() => return false,
                Piece::Markup { md, .. } if !md.trim().is_empty() => return false,
                Piece::Math { .. } | Piece::MathMl { .. } | Piece::LinkOpen(_) => return false,
                _ => {}
            }
        }
        svgs == 1
    }

    /// The paragraph as one flowing line of prose.
    fn render_prose(&self, in_cell: bool) -> String {
        // A paragraph that is nothing but equations was a DISPLAYED one — see
        // [`Para::display_math`]. Written whole and returned here, before the
        // inline walk, because the pieces join into a single `$$…$$` rather
        // than each getting delimiters of its own.
        if let Some(maths) = self.display_math(in_cell) {
            return format!("$${}$$", maths.join(" "));
        }
        // The same question for `--mathml`. `display="block"` is not
        // decoration: a browser sets `math-style: normal` for it, which puts a
        // big operator's limits above and below at full size and sets a
        // fraction at display proportions. See [`Para::display_math_ml`].
        if let Some((body, approx)) = self.display_math_ml(in_cell) {
            return format!(
                "{}{body}{}",
                crate::mathml::open_tag(true, approx),
                crate::mathml::CLOSE_TAG,
            );
        }
        // The same question for a drawn equation: alone in its own block, the
        // `<svg>` may be broken across lines and indented.
        let pretty_svg = self.display_svg(in_cell);
        let mut out = String::new();
        // An emphasis delimiter waiting for the first non-space character, so
        // it never ends up leaning against a space.
        let mut pending_open: Option<&'static str> = None;
        // Where each open link's content began, so an empty one can be
        // withdrawn. Nested links are illegal in Markdown; the inner one is
        // dropped rather than emitted.
        let mut links: Vec<(usize, String)> = Vec::new();
        let mut i = 0;
        while i < self.pieces.len() {
            // A maximal run of fixed-pitch text becomes ONE code span, so a
            // `\code{foo bar}` is `` `foo bar` `` and not two spans with a
            // bare space between them.
            let end = self.mono_run_end(i);
            if end > i {
                let (lead, body, trail) = mono_run_text(&self.pieces[i..end]);
                if lead {
                    push_text(&mut out, &mut pending_open, " ");
                }
                if !body.is_empty() {
                    push_markup(&mut out, &mut pending_open, &escape::code_span(&body));
                }
                if trail {
                    out.push(' ');
                }
                i = end;
                continue;
            }
            match &self.pieces[i] {
                Piece::Text { s, .. } => {
                    push_text(&mut out, &mut pending_open, &escape::inline(s))
                }
                // Outside a code block a gap is just a word space; its exact
                // width is the line breaker's business, not the reader's.
                Piece::Gap(_) => push_text(&mut out, &mut pending_open, " "),
                // Nothing: the word space that replaces a rejoined line break
                // was already pushed as text by the glue rule, which is where
                // it has to be decided, because between two CJK characters
                // there must not be one.
                Piece::Newline { .. } => {}
                Piece::Markup { md, .. } => push_markup(&mut out, &mut pending_open, md),
                // `$…$` inline, `$$…$$` displayed — the delimiters GitHub,
                // Pandoc, VS Code and Typora all read. (The HTML backend uses
                // `\(…\)`/`\[…\]` instead, because KaTeX's `auto-render` and
                // MathJax enable those by default and `$…$` by configuration;
                // `crate::latex`'s module comment has the argument.)
                // Inline by construction: a paragraph whose only ink is
                // equations returned above, so reaching here means there is
                // prose beside this one.
                // An equation drawn as an `<svg>`. Pretty-printed only when
                // it is the whole paragraph — see `crate::mathsvg::Wrap`, and
                // note the raw markup is never escaped.
                Piece::MathSvg { inline, block, .. } => push_markup(
                    &mut out,
                    &mut pending_open,
                    if pretty_svg { block } else { inline },
                ),
                // MathML set inside a sentence. Inline by construction: a
                // paragraph whose only ink is equations returned above. One
                // line, like every other shape this mode writes — a Markdown
                // paragraph is one line, and a renderer with `breaks: true`
                // puts a `<br>` at every newline, including the ones inside a
                // `<math>`.
                Piece::MathMl { body, approx, .. } => {
                    let markup = format!(
                        "{}{body}{}",
                        crate::mathml::open_tag(false, *approx),
                        crate::mathml::CLOSE_TAG,
                    );
                    push_markup(&mut out, &mut pending_open, &markup);
                }
                Piece::Math { latex, .. } => {
                    // Two equations may sit side by side with nothing between
                    // them — one construction routinely produces several math
                    // boxes in a row, and `latexcmds`' Schrödinger equation is
                    // five. Written flush, the closing `$` of one and the
                    // opening `$` of the next form a literal `$$`, which every
                    // renderer that understands display math reads as one.
                    // Measured: `$h$$\frac{1}{2m}…` swallowed the whole
                    // formula into a display block that never closed.
                    if out.ends_with('$') {
                        out.push(' ');
                    }
                    push_markup(&mut out, &mut pending_open, &format!("${latex}$"));
                }
                Piece::EmphOpen(delim) => {
                    // Two delimiters in a row would open and immediately
                    // reopen; flush the first as ordinary markup.
                    if let Some(prev) = pending_open.take() {
                        out.push_str(prev);
                    }
                    pending_open = Some(delim);
                }
                Piece::EmphClose(delim) => {
                    // Nothing was written since the opener: an empty `**` pair
                    // is not emphasis, it is four literal asterisks.
                    if pending_open.take().is_none() {
                        // The closer may not lean against a space either, so
                        // the space moves out from under it.
                        let keep = out.trim_end_matches(' ').len();
                        let spaces = out.len() - keep;
                        out.truncate(keep);
                        out.push_str(delim);
                        for _ in 0..spaces.min(1) {
                            out.push(' ');
                        }
                    }
                }
                Piece::LinkOpen(url) => {
                    if links.is_empty() {
                        push_markup(&mut out, &mut pending_open, "[");
                        links.push((out.len(), url.clone()));
                    } else {
                        // Nested: the inner link contributes its text only.
                        links.push((usize::MAX, String::new()));
                    }
                }
                Piece::LinkClose => {
                    if let Some((start, url)) = links.pop() {
                        if start != usize::MAX {
                            close_link(&mut out, start, &url);
                        }
                    }
                }
            }
            i += 1;
        }
        if let Some(delim) = pending_open {
            out.push_str(delim);
        }
        // An unterminated link would swallow the rest of the document.
        for (start, _) in links.into_iter().rev() {
            if start != usize::MAX && start >= 1 {
                out.remove(start - 1);
            }
        }
        out
    }

    /// The paragraph as the literal contents of a fence: the document's own
    /// characters, its own line breaks, and its own indentation.
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
                // line ran out of room, not one the author wrote. Rejoined
                // even here — see [`Piece::Newline`].
                Piece::Newline { hard: false } => {}
                Piece::Newline { hard: true } => {
                    // Trailing spaces on a code line are invisible noise, and
                    // the line-end `inline-fil` reliably produces some.
                    while out.ends_with(' ') {
                        out.pop();
                    }
                    out.push('\n');
                }
                // Inside a fence there is no markup, only text — so a link
                // contributes its URL-free text, an image its label, and an
                // equation its characters rather than its LaTeX.
                Piece::Markup { plain, .. }
                | Piece::Math { plain, .. }
                | Piece::MathSvg { plain, .. }
                | Piece::MathMl { plain, .. } => out.push_str(plain),
                Piece::EmphOpen(_)
                | Piece::EmphClose(_)
                | Piece::LinkOpen(_)
                | Piece::LinkClose => {}
            }
        }
        out
    }

    /// The end of the maximal run of fixed-pitch pieces starting at `i`, or
    /// `i` itself when the piece there is not one.
    ///
    /// Gaps and rejoined line breaks are absorbed into the run, so a code
    /// span the box stream split across a line boundary — `` `point list` ``
    /// broken after `point` — comes back as ONE span. A run of nothing but
    /// spacing is not code and is declined, or a stray gap between two
    /// ordinary words would come out in backticks.
    fn mono_run_end(&self, i: usize) -> usize {
        let is_run_piece = |p: Option<&Piece>| {
            matches!(
                p,
                Some(Piece::Text { mono: true, .. }) | Some(Piece::Gap(_))
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
        let has_ink = self.pieces[i..end].iter().any(
            |p| matches!(p, Piece::Text { s, .. } if !s.trim().is_empty()),
        );
        if has_ink {
            end
        } else {
            i
        }
    }
}

/// A fixed-pitch run's text, plus whether it carried whitespace on either
/// edge that belongs OUTSIDE the code span.
///
/// The edges matter: a space inside the backticks prints, so `` use `x` now ``
/// would come out as ``use ` x ` now``. Whitespace at either end is therefore
/// lifted out and re-emitted as an ordinary word space.
fn mono_run_text(run: &[Piece]) -> (bool, String, bool) {
    let mut body = String::new();
    for piece in run {
        match piece {
            Piece::Text { s, .. } => body.push_str(s),
            Piece::Gap(_) => body.push(' '),
            // Nothing, for the same reason as in prose: the word space that
            // replaces a rejoined line break comes from the GLUE, which is
            // the only thing that knows whether there was one. A line broken
            // mid-word — by hyphenation, or between two CJK characters — has
            // no space to restore, and putting one here would print it
            // inside the backticks.
            Piece::Newline { .. } => {}
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
/// count. With no advance measured (a base-14 render, where no font file
/// says whether a face is fixed-pitch at all) it degrades to one space, which
/// keeps the words apart and loses only the indentation.
fn gap_spaces(pt: f64, advance: Option<f64>) -> usize {
    match advance {
        Some(a) if a > 0.0 => (pt / a).round().max(0.0) as usize,
        // No measurement: any positive gap is one space.
        _ => usize::from(pt > 0.0),
    }
}

/// Append generated Markdown, releasing any emphasis delimiter waiting for a
/// non-space character first.
fn push_markup(out: &mut String, pending: &mut Option<&'static str>, md: &str) {
    if let Some(delim) = pending.take() {
        out.push_str(delim);
    }
    out.push_str(md);
}

/// Append already-escaped text. A delimiter waiting to open moves to AFTER
/// any leading whitespace, since `* text*` is not emphasis in any renderer.
fn push_text(out: &mut String, pending: &mut Option<&'static str>, s: &str) {
    match pending.take() {
        None => out.push_str(s),
        Some(delim) => {
            let lead = s.len() - s.trim_start_matches(' ').len();
            out.push_str(&s[..lead]);
            if lead == s.len() {
                // All space: keep waiting for real content.
                *pending = Some(delim);
            } else {
                out.push_str(delim);
                out.push_str(&s[lead..]);
            }
        }
    }
}

/// Close a link whose `[` sits at `start - 1` and whose content runs from
/// `start` to the end of `out`.
///
/// The word space before a link is PENDING when the link opens — the glue
/// rule cannot settle it until the character that follows is known, and that
/// character arrives after the `[` has been written. Left there it lands
/// inside the brackets, and `**bold** [here](…)` comes out as
/// `**bold**[ here](…)`: a link whose text begins with a space, which renders
/// with the gap in the wrong place and the wrong thing underlined. Both edges
/// are moved out, and a link that turns out to hold nothing but spacing is
/// withdrawn entirely along with its bracket.
fn close_link(out: &mut String, start: usize, url: &str) {
    let content = &out[start..];
    if content.trim().is_empty() {
        out.truncate(start - 1);
        return;
    }
    let lead = content.len() - content.trim_start_matches(' ').len();
    if lead > 0 {
        // Move the `[` to after the leading spaces. It is one ASCII byte, so
        // the arithmetic is in bytes and characters alike.
        out.remove(start - 1);
        out.insert(start - 1 + lead, '[');
    }
    let trail = out.len() - out.trim_end_matches(' ').len();
    out.truncate(out.len() - trail);
    out.push_str(&format!("]({})", link_url(url)));
    for _ in 0..trail.min(1) {
        out.push(' ');
    }
}

/// A URL inside `(…)`. Spaces and parentheses would end the destination
/// early, so a URL carrying either goes in angle brackets, which is
/// CommonMark's own escape hatch for exactly this.
fn link_url(url: &str) -> String {
    if url.contains([' ', '(', ')']) {
        format!("<{}>", url.replace('<', "%3C").replace('>', "%3E"))
    } else {
        url.to_string()
    }
}

/// Collapse the runs of spaces a rejoined paragraph accumulates — a line
/// boundary and the glue on either side of it each contribute one — into
/// single spaces, and drop them at the edges.
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
        let rendered = para.render(None, false).expect("some content");
        assert!(!rendered.code, "expected prose, got code");
        rendered.text
    }

    #[test]
    fn document_text_is_escaped_and_generated_markdown_is_not() {
        let out = prose(vec![
            Piece::EmphOpen("**"),
            Piece::Text {
                s: "2*3".into(),
                mono: false,
            },
            Piece::EmphClose("**"),
        ]);
        assert_eq!(out, "**2\\*3**");
    }

    /// `* text *` is not emphasis in any renderer, so the spaces move out
    /// from under both delimiters.
    #[test]
    fn an_emphasis_delimiter_never_leans_on_a_space() {
        let out = prose(vec![
            Piece::Text {
                s: "a ".into(),
                mono: false,
            },
            Piece::EmphOpen("*"),
            Piece::Text {
                s: " b ".into(),
                mono: false,
            },
            Piece::EmphClose("*"),
            Piece::Text {
                s: "c".into(),
                mono: false,
            },
        ]);
        assert_eq!(out, "a *b* c");
    }

    #[test]
    fn an_empty_emphasis_pair_is_not_four_asterisks() {
        let out = prose(vec![
            Piece::Text {
                s: "a".into(),
                mono: false,
            },
            Piece::EmphOpen("**"),
            Piece::EmphClose("**"),
            Piece::Text {
                s: "b".into(),
                mono: false,
            },
        ]);
        assert_eq!(out, "ab");
    }

    /// A `\href` whose content is a drawn bullet leaves nothing between the
    /// brackets; `[](url)` renders as literal text in most renderers and as
    /// an invisible link in the rest.
    #[test]
    fn an_empty_link_is_withdrawn_entirely() {
        let out = prose(vec![
            Piece::Text {
                s: "see ".into(),
                mono: false,
            },
            Piece::LinkOpen("http://x/".into()),
            Piece::LinkClose,
        ]);
        assert_eq!(out, "see");
    }

    /// A line boundary is the PORT's wrapping decision; the space that stands
    /// in for it comes from the glue rule (which is where a CJK pair can
    /// suppress it), so the marker itself must add nothing or every rejoin
    /// would double-space.
    #[test]
    fn a_rejoined_line_break_adds_no_space_of_its_own() {
        let out = prose(vec![
            Piece::Text {
                s: "研".into(),
                mono: false,
            },
            Piece::Newline { hard: true },
            Piece::Text {
                s: "究".into(),
                mono: false,
            },
        ]);
        assert_eq!(out, "研究");
    }

    /// The all-fixed-pitch test is the FALLBACK, used only for a paragraph of
    /// one line. A multi-line one is decided structurally, because a `+code`
    /// block containing Japanese is not all fixed-pitch — the Latin face has
    /// no CJK glyphs, so those characters are set in the document's own face.
    #[test]
    fn a_multi_line_code_block_is_recognised_by_its_fils_not_its_font() {
        let mixed_code = Para {
            open: true,
            has_mono: true,
            mixed: true,
            lines: 3,
            fil_lines: 3,
            ..Para::default()
        };
        assert!(mixed_code.is_code(), "every line ended with an inline-fil");
        // Ordinary justified prose: only its LAST line carries a fil.
        let prose = Para {
            open: true,
            has_mono: true,
            lines: 3,
            fil_lines: 1,
            ..Para::default()
        };
        assert!(!prose.is_code());
        // A two-line prose paragraph is the case a `>= lines - 1` test would
        // have got wrong, so it is pinned separately.
        let two_line_prose = Para {
            open: true,
            has_mono: true,
            lines: 2,
            fil_lines: 1,
            ..Para::default()
        };
        assert!(!two_line_prose.is_code());
        // Fils everywhere but no fixed-pitch run anywhere is a centred block,
        // not code.
        let centred = Para {
            open: true,
            lines: 3,
            fil_lines: 3,
            ..Para::default()
        };
        assert!(!centred.is_code());
    }

    #[test]
    fn a_link_url_with_a_space_goes_in_angle_brackets() {
        let out = prose(vec![
            Piece::LinkOpen("http://x/a b".into()),
            Piece::Text {
                s: "t".into(),
                mono: false,
            },
            Piece::LinkClose,
        ]);
        assert_eq!(out, "[t](<http://x/a b>)");
    }

    /// One code span for the whole run, not one per chunk the box stream
    /// happened to split the word into.
    #[test]
    fn adjacent_fixed_pitch_runs_become_one_code_span() {
        let out = prose(vec![
            Piece::Text {
                s: "foo".into(),
                mono: true,
            },
            Piece::Gap(3.0),
            Piece::Text {
                s: "bar".into(),
                mono: true,
            },
        ]);
        assert_eq!(out, "`foo bar`");
    }

    /// The gap on the outside edge of a fixed-pitch run is the surrounding
    /// prose's word space; inside the backticks it would print.
    #[test]
    fn a_gap_on_the_edge_of_a_code_span_stays_outside_it() {
        let out = prose(vec![
            Piece::Text {
                s: "use".into(),
                mono: false,
            },
            Piece::Gap(3.0),
            Piece::Text {
                s: "x".into(),
                mono: true,
            },
            Piece::Gap(3.0),
            Piece::Text {
                s: "now".into(),
                mono: false,
            },
        ]);
        assert_eq!(out, "use `x` now");
    }

    /// The whole point of deferring: an all-monospace paragraph is a code
    /// block, and its indentation is recovered from the gap widths against
    /// the measured character advance.
    #[test]
    fn an_all_monospace_paragraph_becomes_a_fence_that_keeps_its_indentation() {
        let para = Para {
            pieces: vec![
                Piece::Text {
                    s: "if x:".into(),
                    mono: true,
                },
                Piece::Newline { hard: true },
                Piece::Gap(12.0),
                Piece::Text {
                    s: "return".into(),
                    mono: true,
                },
            ],
            open: true,
            mono: true,
            ..Para::default()
        };
        let rendered = para.render(Some(3.0), false).unwrap();
        assert!(rendered.code);
        assert_eq!(rendered.text, "if x:\n    return");
    }

    /// Nothing inside a fence is markup, so a document's own asterisks are
    /// NOT escaped there — the escaping is chosen per paragraph, which is
    /// exactly what the two-pass buffer exists for.
    #[test]
    fn a_code_block_does_not_escape_its_contents() {
        let para = Para {
            pieces: vec![Piece::Text {
                s: "a * b_c".into(),
                mono: true,
            }],
            open: true,
            mono: true,
            ..Para::default()
        };
        assert_eq!(para.render(Some(3.0), false).unwrap().text, "a * b_c");
    }

    #[test]
    fn a_heading_is_written_at_its_outline_depth() {
        let para = Para {
            pieces: vec![Piece::Text {
                s: "Introduction".into(),
                mono: false,
            }],
            open: true,
            heading_level: Some(1),
            ..Para::default()
        };
        assert_eq!(para.render(None, false).unwrap().text, "## Introduction");
    }

    /// The gap arithmetic is a division, not a guess: `code.satyh` sizes its
    /// indent in exact multiples of the character advance.
    #[test]
    fn gap_spaces_recovers_the_source_column_count() {
        assert_eq!(gap_spaces(15.0, Some(5.0)), 3);
        assert_eq!(gap_spaces(0.0, Some(5.0)), 0);
        // No font file to measure: any gap is one space.
        assert_eq!(gap_spaces(15.0, None), 1);
        assert_eq!(gap_spaces(0.0, None), 0);
    }
}
