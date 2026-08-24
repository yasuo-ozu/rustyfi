//! The block-level `Vec<VertBox>` walker, and the line-oriented writer it
//! writes through.
//!
//! | `VertBox` | emitted as |
//! |--|--|
//! | a run of consecutive `Line`s | one paragraph, rejoined into one line |
//! | the same, all fixed-pitch | one ``` fence, line breaks and indentation kept |
//! | `Skip`/`ParagTop`/`FramePad` | a paragraph boundary (the amount is dropped) |
//! | `ListMark(ListStart{ordered})`/`ItemStart`/`ItemEnd`/`ListEnd` | `- `/`1. `, nested by indentation |
//! | `FrameStart`/`FrameEnd` | a paragraph boundary, nothing else |
//! | `ClearPage`, `HookPageBreak` | nothing |
//!
//! ## Why a `Line` boundary disappears
//!
//! Every `VertBox::Line` in the stream is a line the PORT's own paragraph
//! breaker decided on, at the page width the document declared. Reproducing
//! them would hard-wrap the Markdown at a width the reader never chose, and —
//! worse — would fossilize the port's hyphenation. So they are rejoined
//! (`crate::recover::line_join`), which is where authored hyphens have to be
//! told apart from the breaker's.
//!
//! Inside a code block they are the AUTHOR's line breaks and are kept, which
//! is the one thing `crate::recover::is_monospace` is really for.
//!
//! ## Why frames and alignment leave no trace
//!
//! A frame is a box with a border and padding, and Markdown has neither. An
//! `\align-center` is a pair of `inline-fil`s, and Markdown has no alignment.
//! Both are dropped rather than approximated: a blockquote is not a frame,
//! and there is no reason a reader should be told a paragraph was centred
//! when nothing about the text depends on it. The one thing a frame still
//! does here is end the paragraph, which it genuinely does.
//!
//! ## Why a recovered indent is NOT reproduced
//!
//! `block.rs`'s HTML twin turns a paragraph's smallest left offset into a
//! `margin-left`, which is how the third-party `enumitem` list package gets
//! any visible nesting at all. Markdown cannot do that: four leading spaces
//! is an indented CODE BLOCK, and fewer than four is nothing. An `enumitem`
//! list therefore comes out as a sequence of flat paragraphs, each still
//! carrying the bullet glyph the package drew for it — degraded, but not
//! mangled into code.

use rustyfi_backend::{ListMarkKind, PureHorzBox, VertBox};

use super::para::{Para, Piece, Rendered};
use super::Ctx;
use crate::recover::{LineJoin, WORD_SPACE_PT};

/// One open list. `ordered` decides the marker, `next` supplies its number —
/// the document's own numeral is drawn inside a `BulletStart`/`BulletEnd`
/// fence and dropped, so the count is kept here instead.
struct ListState {
    ordered: bool,
    next: usize,
    /// Whether the next `ItemStart` opens this list's FIRST item, which is
    /// the only one that may need a blank line above it.
    first_item: bool,
    /// This list is nested inside an item, so even its first item follows the
    /// parent's text directly with no blank line between.
    tight_start: bool,
}

/// A line-oriented Markdown sink.
///
/// Markdown has no closing tags: a block's membership of a list is expressed
/// by how far it is indented, and its separation from the block above by a
/// blank line. So instead of pushing and popping elements this keeps the
/// current indentation and a marker waiting to be worn by the next block's
/// first line — which is exactly what a `- ` is.
pub(super) struct Writer {
    out: String,
    /// Columns every line of the current block is indented by.
    indent: usize,
    lists: Vec<ListState>,
    /// The indent each open item added, so `ItemEnd` gives back exactly what
    /// `ItemStart` took (an ordered marker's width depends on its number).
    item_widths: Vec<usize>,
    /// The list marker the next block's first line wears in place of the last
    /// columns of its indentation.
    pending_marker: Option<String>,
    /// Skip the blank line that normally separates two blocks. Set by
    /// `ItemStart`, so a list reads as a list rather than as a page of
    /// disconnected bullets.
    suppress_blank: bool,
    /// Code-block content written but not yet fenced.
    ///
    /// A fence is deferred so that consecutive code paragraphs go into ONE of
    /// them. `code.satyh` builds a whole `+code` block as a single paragraph,
    /// but that is a choice, not a rule: `code-printer` calls `line-break`
    /// once per SOURCE LINE, so a thirty-line listing arrives as thirty
    /// one-line paragraphs. Fenced as they came, that is thirty separate code
    /// blocks with a blank line between each pair — unreadable, and not what
    /// the document says. Anything that is genuinely a block boundary (a
    /// frame, a list marker, a prose paragraph, a table) flushes the fence
    /// first, so only paragraphs with nothing whatever between them merge.
    pending_code: Option<String>,
}

impl Writer {
    fn new() -> Self {
        Writer {
            out: String::new(),
            indent: 0,
            lists: Vec::new(),
            item_widths: Vec::new(),
            pending_marker: None,
            suppress_blank: false,
            pending_code: None,
        }
    }

    /// Write one rendered paragraph. Code contents accumulate into a single
    /// deferred fence (see [`Writer::pending_code`]); everything else closes
    /// that fence first and is written straight out.
    fn rendered(&mut self, r: &Rendered) {
        if r.code {
            match &mut self.pending_code {
                Some(open) => {
                    open.push('\n');
                    open.push_str(&r.text);
                }
                None => self.pending_code = Some(r.text.clone()),
            }
        } else {
            self.flush_code();
            self.block(&r.text);
        }
    }

    /// Close the deferred fence, if one is open. Called by every real block
    /// boundary — a prose paragraph, a table, a frame, a list marker, and the
    /// end of the document.
    fn flush_code(&mut self) {
        if let Some(content) = self.pending_code.take() {
            // A fence, not four-space indentation: only a fence keeps its
            // contents out of the surrounding list's indentation arithmetic,
            // and only a fence can hold a blank line without ending the
            // block. No language tag — the box stream records the FACE the
            // code was set in, never what language it is.
            //
            // The fence is as long as it needs to be: a document about
            // Markdown types ``` inside its own code samples, and a
            // three-backtick fence would end there.
            let fence = "`".repeat(fence_len(&content));
            self.block(&format!("{fence}\n{content}\n{fence}"));
        }
    }

    /// Write one block, indented into whatever list encloses it and separated
    /// from the block above.
    fn block(&mut self, body: &str) {
        let body = body.trim_end();
        if body.is_empty() {
            return;
        }
        if !self.out.is_empty() && !self.suppress_blank && !self.out.ends_with("\n\n") {
            self.out.push('\n');
        }
        self.suppress_blank = false;
        let marker = self.pending_marker.take();
        for (i, line) in body.lines().enumerate() {
            if line.is_empty() {
                // A blank line inside a block (between a fence's own lines)
                // carries no indentation: trailing whitespace on an otherwise
                // empty line is invisible and some tools strip it anyway.
                self.out.push('\n');
                continue;
            }
            match (i, &marker) {
                (0, Some(m)) => {
                    self.out
                        .push_str(&" ".repeat(self.indent.saturating_sub(m.chars().count())));
                    self.out.push_str(m);
                }
                _ => self.out.push_str(&" ".repeat(self.indent)),
            }
            self.out.push_str(line);
            self.out.push('\n');
        }
    }

    fn list_start(&mut self, ordered: bool) {
        self.flush_code();
        let nested = !self.lists.is_empty();
        self.lists.push(ListState {
            ordered,
            next: 1,
            first_item: true,
            tight_start: nested,
        });
    }

    fn list_end(&mut self) {
        self.flush_code();
        self.lists.pop();
    }

    fn item_start(&mut self) {
        self.flush_code();
        let (marker, tight) = match self.lists.last_mut() {
            Some(list) => {
                let marker = if list.ordered {
                    let n = list.next;
                    list.next += 1;
                    format!("{n}. ")
                } else {
                    "- ".to_string()
                };
                let tight = !list.first_item || list.tight_start;
                list.first_item = false;
                (marker, tight)
            }
            // An unmatched `ItemStart` — should not happen, the markers are
            // always stdlib-paired — degrades to a bullet rather than
            // panicking.
            None => ("- ".to_string(), false),
        };
        let width = marker.chars().count();
        self.indent += width;
        self.item_widths.push(width);
        self.pending_marker = Some(marker);
        self.suppress_blank = tight;
    }

    fn item_end(&mut self) {
        self.flush_code();
        // The marker is still waiting, so this item produced no block at all
        // — its content was a bullet glyph and nothing else, or a drawing
        // below the size threshold. Write the marker alone: an empty entry is
        // truthful, and dropping it would slip an ordered list's numbering
        // for everything below.
        if let Some(marker) = self.pending_marker.take() {
            if !self.out.is_empty() && !self.suppress_blank && !self.out.ends_with("\n\n") {
                self.out.push('\n');
            }
            self.suppress_blank = false;
            self.out
                .push_str(&" ".repeat(self.indent.saturating_sub(marker.chars().count())));
            self.out.push_str(marker.trim_end());
            self.out.push('\n');
        }
        self.indent -= self.item_widths.pop().unwrap_or(0);
    }

    pub(super) fn finish(mut self) -> String {
        self.flush_code();
        self.out
    }
}

/// How many backticks a fence needs: three, unless the code itself opens a
/// line with a run that long or longer.
fn fence_len(content: &str) -> usize {
    let longest = content
        .lines()
        .map(|line| line.trim_start().chars().take_while(|c| *c == '`').count())
        .max()
        .unwrap_or(0);
    longest.max(2) + 1
}

/// Render a nested block list — a footnote body — as its own document
/// fragment, with no inherited list indentation.
pub(super) fn render_block(vboxes: &[VertBox], ctx: &Ctx) -> String {
    let mut w = Writer::new();
    walk_vboxes(&mut w, vboxes, ctx);
    w.finish()
}

/// Walk one flat vertical-box list. Reentrant: an `EmbeddedBlock` recurses
/// into the SAME writer, so its content keeps the indentation of whatever
/// list it is inside.
pub(super) fn walk_vboxes(w: &mut Writer, vboxes: &[VertBox], ctx: &Ctx) {
    let mut para = Para::default();
    for vb in vboxes {
        match vb {
            VertBox::Line { contents, .. } => {
                // Whether this line ends with an `inline-fil`, which is what
                // says a paragraph is a code block — see `Para::is_code`. Set
                // by a fil and cleared by any real text after it, so it means
                // "the fil is the last content", not merely "there was one".
                // Glue, markers and the huge `inline-skip` the taken
                // forced-break splices in all ride AFTER the fil and must not
                // clear it.
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
                            if para.heading_level.is_none() {
                                para.heading_level = crate::recover::find_heading_level(
                                    other,
                                    &ctx.dests,
                                    &ctx.outline_by_dest,
                                );
                            }
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
            // The amount is dropped — Markdown has one paragraph separator
            // and it is a blank line — but the boundary itself is real.
            VertBox::Skip(_) | VertBox::ParagTop(_) | VertBox::FramePad(_) => {
                flush_para(w, &mut para, ctx)
            }
            // Pagination is meaningless once the document is one continuous
            // flow, and a `---` rule here would turn the paragraph above it
            // into a setext heading.
            VertBox::ClearPage => {
                flush_para(w, &mut para, ctx);
                w.flush_code();
            }
            VertBox::HookPageBreak(_) => {}
            // A frame's border, padding and decoration are all dropped; what
            // survives is that it ends a paragraph — and, being a real block
            // boundary, closes any open code fence.
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

/// What happens at the boundary between two `Line`s of one paragraph.
///
/// The marker is pushed unconditionally and the rejoin rule applied
/// unconditionally, because whether this paragraph is a code block is not
/// settled until it ends (`Para::is_code`) — the two answers are recorded
/// side by side and one of them is read at render time. The one thing decided
/// HERE is whether a word space is implied.
///
/// That one IS decidable now, and it needs BOTH signals. "The last run was
/// fixed-pitch" alone is not enough: a prose sentence whose inline
/// `` `point list` `` happens to straddle the line break also ends its line
/// in a fixed-pitch run, and suppressing the space there closes the code span
/// and reopens it — ``` `point``list` ```. A line of a code BLOCK
/// additionally ends with an `inline-fil`, which a wrapped prose line never
/// does, so the pair tells them apart.
fn end_of_line(para: &mut Para, ctx: &Ctx, fil_terminated: bool) {
    // BEFORE the marker is pushed, because both halves of the rejoin rule
    // read the LAST piece to find the hyphen. Pushing first hides it behind
    // the marker, `line_join`'s answer is then applied to nothing, and the
    // breaker's hyphen survives into the text — `graph-ics`, from `xpath`'s
    // one over-long API line.
    if ctx.mono_run.get() && fil_terminated {
        // A code block's line: the break is the author's, and no word space
        // stands in for it.
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
    para.pieces.push(Piece::Newline { hard: fil_terminated });
}

/// Close the current paragraph, then release anything it queued: the tables
/// it contained (block-level in Markdown, so they cannot stay inline) come
/// out immediately after it, in the order they were reached.
fn flush_para(w: &mut Writer, para: &mut Para, ctx: &Ctx) {
    if let Some(rendered) = para.render(ctx.mono_advance.get(), false) {
        w.rendered(&rendered);
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
