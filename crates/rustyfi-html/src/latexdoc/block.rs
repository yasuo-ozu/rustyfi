//! The block-level `Vec<VertBox>` walker, and the writer it writes through.
//!
//! | `VertBox` | emitted as |
//! |--|--|
//! | a run of consecutive `Line`s | one paragraph, rejoined onto one line |
//! | the same, all fixed-pitch | one `Verbatim`, line breaks and indentation kept |
//! | `Skip`/`ParagTop`/`FramePad` | a blank line (the amount is dropped) |
//! | `ListMark(ListStart{ordered})`/`ItemStart`/… | `itemize`/`enumerate` and `\item` |
//! | `FrameStart`/`FrameEnd` | a paragraph boundary, nothing else |
//! | `ClearPage` | `\clearpage` |
//! | `HookPageBreak` | nothing |
//!
//! ## Why a `Line` boundary disappears
//!
//! Every `VertBox::Line` in the stream is a line the PORT's own paragraph
//! breaker decided on, at the page width the document declared. LaTeX is
//! going to break the paragraph again, at whatever measure this document's
//! `geometry` gives it, so reproducing them would be pointless — and worse
//! than pointless, since a hard line break also fossilizes the port's
//! hyphenation. They are rejoined (`crate::recover::line_join`), which is
//! where authored hyphens have to be told apart from the breaker's.
//!
//! Inside a code block they are the AUTHOR's line breaks and are kept, which
//! is the one thing `crate::recover::is_monospace` is really for.
//!
//! ## Why `ClearPage` survives when the other page furniture does not
//!
//! The input is the PRE-page-break flow, so there are no pages in it and the
//! running heads and folios never existed. `clear-page` is different: it is
//! not page furniture but an authored instruction — "start the next chapter
//! on a fresh page" — and `\clearpage` says exactly that to a typesetter that
//! is doing its own pagination. The Markdown backend drops it because
//! Markdown has no pages at all.

use rustyfi_backend::{ListMarkKind, PureHorzBox, VertBox};

use super::para::{Para, Piece, Rendered};
use super::Ctx;
use crate::recover::{LineJoin, WORD_SPACE_PT};

/// A LaTeX sink that knows what environment it is inside.
///
/// Unlike Markdown, membership of a list is an ENVIRONMENT rather than an
/// indentation, so there is real nesting to track — but the same deferral
/// applies to a code block, and for the same reason: `code-printer` calls
/// `line-break` once per SOURCE LINE, so a thirty-line listing arrives as
/// thirty one-line paragraphs, and fencing them as they come is thirty
/// separate `Verbatim` environments with a `\par` between each pair.
pub(super) struct Writer<'a, 'b> {
    out: String,
    /// One frame per open list. `ordered` picks the environment; `item` is
    /// `Some(false)` for an item that still owes its `\item`, `Some(true)`
    /// once written, `None` between items. ONE stack rather than two kept in
    /// lockstep — the unwind in [`Writer::finish`] has to pop them together,
    /// and two stacks make that an assumption instead of a fact.
    lists: Vec<ListFrame>,
    /// Code content written but not yet wrapped — see the type's doc comment.
    pending_code: Option<String>,
    /// Read for `inline_only` and marked for `fvextra`; interior-mutable, so
    /// a shared reference is all this needs and the flags do not have to be
    /// mirrored and handed back.
    ctx: &'a Ctx<'b>,
}

/// One open `itemize`/`enumerate`.
struct ListFrame {
    ordered: bool,
    item: Option<bool>,
}

impl<'a, 'b> Writer<'a, 'b> {
    fn new(ctx: &'a Ctx<'b>) -> Self {
        Writer {
            out: String::new(),
            lists: Vec::new(),
            pending_code: None,
            ctx,
        }
    }

    /// Write one rendered paragraph.
    fn rendered(&mut self, r: Rendered) {
        if r.code {
            match &mut self.pending_code {
                Some(open) => {
                    open.push('\n');
                    open.push_str(&r.text);
                }
                None => self.pending_code = Some(r.text),
            }
        } else {
            self.flush_code();
            self.block(&r.text);
        }
    }

    /// Close the deferred code block, if one is open. Called by every real
    /// block boundary — a prose paragraph, a table, a frame, a list marker,
    /// and the end of the document.
    fn flush_code(&mut self) {
        let Some(content) = self.pending_code.take() else {
            return;
        };
        if self.ctx.inline_only.get() {
            // No environment may be opened here, so the block's own line
            // structure is what has to go: one `\texttt` of the whole thing.
            // `Not allowed in LR mode` is a fatal error, and a table cell
            // with a code sample in it is common enough (`easytable`'s own
            // manual documents its arguments that way) that dropping the
            // cell instead would be visible.
            let flat = crate::collapse_whitespace(&content);
            self.block(&format!("\\texttt{{{}}}", super::escape::text(&flat)));
            return;
        }
        self.ctx.mark_verbatim();
        self.block(&verbatim(&content));
    }

    /// Write one block, separated from the block above by a blank line.
    ///
    /// A blank line rather than `\par`: they are the same thing to TeX, and
    /// a blank line is what makes the generated file readable — which matters
    /// here, because the whole point of emitting `.tex` rather than a PDF is
    /// that someone is going to open it.
    fn block(&mut self, body: &str) {
        let body = body.trim_end();
        if body.is_empty() {
            return;
        }
        // An `\item` this block is the FIRST content of joins it on the same
        // line. A blank line after `\item` opens a second paragraph inside
        // the item, so the marker sits alone on one line and the text starts
        // a full `\parskip` below it — visibly not a list.
        let joins_item = self.open_item();
        if !self.out.is_empty() && !joins_item && !self.out.ends_with("\n\n") {
            if !self.out.ends_with('\n') {
                self.out.push('\n');
            }
            self.out.push('\n');
        }
        if opens_box(body) {
            self.out.push_str("\\noindent ");
        }
        self.out.push_str(body);
        self.out.push('\n');
    }

    /// Write the `\item` this item still owes, if any, returning whether one
    /// was just written — i.e. whether the caller's block is the first thing
    /// in the item and should follow on the same line.
    ///
    /// Deferred so that an item whose content is several blocks gets exactly
    /// one, and so that an item producing nothing at all still gets one:
    /// dropping it would slip an `enumerate`'s numbering for everything
    /// below.
    fn open_item(&mut self) -> bool {
        let Some(Some(open)) = self.lists.last_mut().map(|f| &mut f.item) else {
            return false;
        };
        if *open {
            return false;
        }
        *open = true;
        if !self.out.ends_with('\n') && !self.out.is_empty() {
            self.out.push('\n');
        }
        self.out.push_str("\\item ");
        true
    }

    fn list_start(&mut self, ordered: bool) {
        self.flush_code();
        if self.ctx.inline_only.get() {
            return;
        }
        self.open_item();
        self.lists.push(ListFrame {
            ordered,
            item: None,
        });
        // LaTeX's own `enumerate`/`itemize` nest four deep and no further
        // (`Too deeply nested`), which is two more levels than any document
        // in the corpus uses.
        let env = if ordered { "enumerate" } else { "itemize" };
        self.push_line(&format!("\\begin{{{env}}}"));
    }

    fn list_end(&mut self) {
        self.flush_code();
        let Some(frame) = self.lists.pop() else {
            return;
        };
        // An `itemize` with no `\item` in it is `Something's wrong--perhaps a
        // missing \item`, a hard error. A list whose every item was empty
        // still has its `\item`s (see `open_item`), so this only fires for a
        // genuinely empty list.
        if self.out.ends_with("\\begin{itemize}\n") || self.out.ends_with("\\begin{enumerate}\n") {
            self.out.push_str("\\item\n");
        }
        let env = if frame.ordered { "enumerate" } else { "itemize" };
        self.push_line(&format!("\\end{{{env}}}"));
    }

    fn item_start(&mut self) {
        self.flush_code();
        if let Some(frame) = self.lists.last_mut() {
            frame.item = Some(false);
        }
    }

    fn item_end(&mut self) {
        self.flush_code();
        // An item that produced nothing still gets its marker.
        self.open_item();
        if let Some(frame) = self.lists.last_mut() {
            frame.item = None;
        }
    }

    fn push_line(&mut self, line: &str) {
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
        self.out.push_str(line);
        self.out.push('\n');
    }

    pub(super) fn finish(mut self) -> String {
        self.flush_code();
        // A list left open by a stream that ended inside one would take the
        // `\end{document}` with it.
        while !self.lists.is_empty() {
            self.list_end();
        }
        self.out
    }
}

/// Does `body` open with a box that fills the measure, so that a paragraph
/// indent in front of it would push it into the margin?
///
/// All three are sized to the measure and all three are unbreakable, so the
/// rule is the same for each: `article`'s 15pt `\parindent` becomes 15pt of
/// overfull `\hbox`. It shows most on a drawing, because the corpus is full
/// of full-measure rules under headings — but a `tabular` released from
/// `Ctx::pending_blocks` and a `minipage` from a wrapping cell are the same
/// shape and were missed while this tested only for a picture.
fn opens_box(body: &str) -> bool {
    ["\\begin{tikzpicture}", "\\begin{tabular}", "\\begin{minipage}"]
        .iter()
        .any(|env| body.starts_with(env))
}

/// A code block, wrapped.
///
/// **`fancyvrb`'s `Verbatim`, not the built-in `verbatim`, and not
/// `listings`.** All three keep the content literal; what separates them is
/// what happens to a line too long for the measure and to a line that is not
/// ASCII.
///
/// - built-in `verbatim` cannot break a long line at all, and `xpath`'s API
///   listing has several that run a long way past the margin;
/// - `listings` can, but it re-tokenizes the content character by character
///   and mishandles multi-byte input — and `latexcmds`' code samples are full
///   of Japanese string literals, which is exactly the case the Markdown
///   backend's own code-block detection exists for;
/// - `fancyvrb` breaks lines (`breaklines`/`breakanywhere`) while leaving the
///   tokenizing to TeX, so a Japanese character in a listing is one character
///   and `luatexja` sets it.
///
/// **The one content a verbatim environment cannot hold is its own end.** A
/// document that discusses this format would type `\end{Verbatim}` in a code
/// sample and the environment would stop there, taking the rest of the file
/// with it. There is no escaping inside a verbatim by definition, so the
/// block falls back to an escaped `\texttt` paragraph with explicit line
/// breaks: uglier, and correct.
fn verbatim(content: &str) -> String {
    const END: &str = "\\end{Verbatim}";
    if content.contains(END) {
        let lines: Vec<String> = content
            .lines()
            .map(|l| {
                // A leading run of spaces is invisible to LaTeX's own
                // spacing; `~` is the fixed-width space that is not.
                let indent = l.len() - l.trim_start_matches(' ').len();
                format!(
                    "{}{}",
                    "~".repeat(indent),
                    super::escape::text(l.trim_start_matches(' '))
                )
            })
            .collect();
        return format!(
            "\\begingroup\\ttfamily\\obeylines\\noindent\n{}\\endgroup",
            lines.join("\\\\\n")
        );
    }
    format!("\\begin{{Verbatim}}\n{content}\n\\end{{Verbatim}}")
}

/// Render a nested block list — a footnote body, a wrapped table cell — as
/// its own fragment.
pub(super) fn render_block(vboxes: &[VertBox], ctx: &Ctx) -> String {
    let mut w = Writer::new(ctx);
    walk_vboxes(&mut w, vboxes, ctx);
    w.finish()
}

/// Walk one flat vertical-box list. Reentrant: an `EmbeddedBlock` recurses
/// into the SAME writer, so its content keeps the environment nesting of
/// whatever list it is inside.
fn walk_vboxes(w: &mut Writer<'_, '_>, vboxes: &[VertBox], ctx: &Ctx) {
    let mut para = Para::default();
    for vb in vboxes {
        match vb {
            VertBox::Line { contents, .. } => {
                // Whether this line ends with an `inline-fil`, which is what
                // says a paragraph is a code block — see `Para::is_code`. Set
                // by a fil and cleared by any real text after it, so it means
                // "the fil is the last content", not merely "there was one".
                let mut fil_terminated = false;
                for (_, bx) in contents {
                    match bx {
                        PureHorzBox::OuterFil => {
                            fil_terminated = true;
                            para.open = true;
                            super::inline::emit_inline(&mut para, bx, ctx);
                        }
                        // The one inline box carrying a whole nested block:
                        // close the paragraph, splice the block in, carry on.
                        PureHorzBox::EmbeddedBlock { block, .. } => {
                            flush_para(w, &mut para, ctx);
                            walk_vboxes(w, block, ctx);
                        }
                        other => {
                            note_heading(&mut para, other, ctx);
                            para.open = true;
                            super::inline::emit_inline(&mut para, other, ctx);
                            if matches!(other, PureHorzBox::InnerString { .. }) {
                                fil_terminated = false;
                                if ctx.mono_run.get() {
                                    para.mono = !para.mixed;
                                    para.has_mono = true;
                                } else {
                                    para.mono = false;
                                    para.mixed = true;
                                }
                            }
                        }
                    }
                }
                if para.open {
                    para.note_line(fil_terminated);
                    end_of_line(&mut para, ctx, fil_terminated);
                }
            }
            // The amount is dropped — LaTeX's own `\parskip` governs the gap
            // between paragraphs at the measure it is setting them to — but
            // the boundary itself is real.
            VertBox::Skip(_) | VertBox::ParagTop(_) | VertBox::FramePad(_) => {
                flush_para(w, &mut para, ctx)
            }
            // An AUTHORED instruction, not page furniture — see this module's
            // doc comment.
            VertBox::ClearPage => {
                flush_para(w, &mut para, ctx);
                w.flush_code();
                w.push_line("\\clearpage");
            }
            VertBox::HookPageBreak(_) => {}
            // A frame's border, padding and decoration are all dropped; what
            // survives is that it ends a paragraph — and, being a real block
            // boundary, closes any open code block.
            VertBox::FrameStart(_) | VertBox::FrameEnd(_) => {
                flush_para(w, &mut para, ctx);
                w.flush_code();
            }
            VertBox::ListMark(kind) => {
                flush_para(w, &mut para, ctx);
                match kind {
                    ListMarkKind::ListStart { ordered } => w.list_start(*ordered),
                    ListMarkKind::ListEnd => w.list_end(),
                    ListMarkKind::ItemStart => w.item_start(),
                    ListMarkKind::ItemEnd => w.item_end(),
                }
            }
        }
    }
    flush_para(w, &mut para, ctx);
}

/// Promote this paragraph to a heading if `bx` is the destination frame of
/// one, recording both the level and the anchor name in one lookup.
///
/// The first match on the paragraph wins and is never un-decided by a later
/// box on the same line.
fn note_heading(para: &mut Para, bx: &PureHorzBox, ctx: &Ctx) {
    if para.heading_level.is_some() {
        return;
    }
    let Some((level, dest)) = crate::recover::find_heading(bx, &ctx.dests, &ctx.outline_by_dest)
    else {
        return;
    };
    para.heading_level = Some(level);
    para.heading_dest = Some(dest.to_string());
    // A heading's anchor is a `\hypertarget`, so it needs the package too,
    // whether or not anything in the document ever LINKS to it. Marking only
    // at `open_link` is `Undefined control sequence` on the first heading of
    // any document with no `\href` in it, which is most of them.
    ctx.mark_hyperref();
}

/// What happens at the boundary between two `Line`s of one paragraph.
///
/// The marker is pushed unconditionally and the rejoin rule applied
/// unconditionally, because whether this paragraph is a code block is not
/// settled until it ends (`Para::is_code`). The one thing decided HERE is
/// whether a word space is implied.
///
/// That one IS decidable now, and it needs BOTH signals. "The last run was
/// fixed-pitch" alone is not enough: a prose sentence whose inline
/// `\texttt{point list}` happens to straddle the line break also ends its
/// line in a fixed-pitch run, and suppressing the space there closes the
/// `\texttt` and reopens it. A line of a code BLOCK additionally ends with an
/// `inline-fil`, which a wrapped prose line never does.
fn end_of_line(para: &mut Para, ctx: &Ctx, fil_terminated: bool) {
    // BEFORE the marker is pushed, because both halves of the rejoin rule
    // read the LAST piece to find the hyphen.
    if ctx.mono_run.get() && fil_terminated {
        ctx.break_hyphen.set(false);
        ctx.reset_flow();
    } else {
        match crate::recover::line_join(ctx.break_hyphen.replace(false), para.ends_with_hyphen()) {
            LineJoin::DropHyphen => para.drop_break_hyphen(),
            // An AUTHORED hyphen the breaker was merely allowed to break
            // after. It stays — deleting it is what once turned
            // `code-printer` into `codeprinter` — but the two halves must not
            // gain a space.
            LineJoin::KeepHyphen => {}
            LineJoin::Space => ctx.note_glue(WORD_SPACE_PT),
        }
    }
    para.pieces.push(Piece::Newline {
        hard: fil_terminated,
    });
}

/// Close the current paragraph, then release anything it queued: the tables
/// it contained (block-level, so they cannot stay inline) come out
/// immediately after it, in the order they were reached.
fn flush_para(w: &mut Writer<'_, '_>, para: &mut Para, ctx: &Ctx) {
    if let Some(rendered) = para.render(ctx.mono_advance.get()) {
        w.rendered(rendered);
    }
    para.clear();
    ctx.mono_advance.set(None);
    // A paragraph boundary is a hard boundary for the inline-flow state: glue
    // recorded at the end of one paragraph must not open the next with a
    // space.
    ctx.reset_flow();
    let pending: Vec<String> = std::mem::take(&mut *ctx.pending_blocks.borrow_mut());
    for block in pending {
        w.flush_code();
        w.block(&block);
    }
}
