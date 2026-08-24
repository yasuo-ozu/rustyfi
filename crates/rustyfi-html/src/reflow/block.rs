//! The block-level `Vec<VertBox>` walker ("Block level"). Unlike the PDF
//! writer's `Page`/`PlacedLine` walk (already-placed, absolutely positioned),
//! this walks the FLAT pre-page-break list with a single linear pass, using a
//! small amount of local state (a "current paragraph" text accumulator and a
//! "pending top margin" carried from the last `Skip`) instead of a `Page`'s
//! already-resolved geometry — there is no x/y here at all, only document order.
//!
//! | `VertBox` | emitted as |
//! |--|--|
//! | a run of consecutive `Line`s | one `<p class="para">` (flowing text) |
//! | `Skip(len)` | closes the current paragraph; `len` becomes the NEXT block element's `margin-top` |
//! | `FrameStart`/`FrameEnd` | a real nested `<div class="frame">` (push/pop) |
//! | `ClearPage` | a soft `<hr class="clearpage">` separator (pagination is meaningless once reflowed) |
//! | `HookPageBreak` | dropped (no reflow meaning) |
//! | `ListMark(ListStart{ordered})`/`ListEnd`/`ItemStart`/`ItemEnd` | S4: real nested `<ul>`/`<ol>`/`<li>` (push/pop, tracked by `list_stack` below) |
//!
//! `PureHorzBox::EmbeddedBlock` (an INLINE box that carries a whole nested
//! `Vec<VertBox>`) is handled HERE, not in `inline.rs`, because splicing it
//! in requires the same "flush the open paragraph, emit a block-level
//! element, resume" dance as `Skip`/`FrameStart` — `inline.rs` only ever
//! appends to an already-open paragraph's text, it never closes one.

use std::fmt::Write as _;

use rustyfi_backend::{ListMarkKind, PureHorzBox, VertBox};

use super::{inline, structure, Ctx};
// The inter-word space standing in for a rejoined line break, and which of
// the three things a line boundary is — both shared with the Markdown
// backend, which feeds the same values to the same rules
// (`crate::recover`).
use crate::recover::{LineJoin, WORD_SPACE_PT};

/// Accumulated state for "the paragraph currently being built" — `text` is
/// the flowing inline HTML gathered so far (escaped/styled runs, glue
/// collapsed to plain spaces so the BROWSER re-breaks lines, rather than
/// keeping the box stream's own fixed per-glyph advances); `open`
/// distinguishes "no paragraph started yet" (nothing to flush) from "a
/// paragraph with only whitespace so far" (still flushed as an empty-ish
/// `<p>`, matching upstream's own willingness to lay out a blank line).
struct Para {
    text: String,
    open: bool,
    /// S3 ("S3" — see `structure.rs`'s doc comment): set once, the first
    /// time `structure::find_heading_level` matches a box on this
    /// paragraph's line(s), by an outline-registered destination `Frame`.
    /// When `Some`, `flush_para` emits `<h{level+1}>` instead of `<p
    /// class="para">`. Never reset mid-paragraph (a paragraph's first
    /// matching `Frame` decides its tag; further boxes on the same line(s)
    /// cannot un-decide it), only on the next `flush_para`.
    heading_level: Option<i64>,
    /// An `inline-fil` stood before any content — the classic TeX
    /// right-flush/centre idiom (`\align-center` is `inline-fil ++ body ++
    /// inline-fil`, `\align-right` is `inline-fil ++ body`). Together with
    /// [`Para::trailing_fil`] this is the ONLY alignment signal the box
    /// stream carries once the eager line breaker's own justification is
    /// discarded, and it is a real one, not a guess.
    leading_fil: bool,
    /// The last thing seen was an `inline-fil` and nothing has been written
    /// since. Reset by any real content, so it genuinely means "trailing".
    /// On its own it means nothing — an ORDINARY paragraph ends with
    /// `inline-fil` (that is how `read-inline ctx {..} ++ inline-fil` fills
    /// the last line), so only the leading+trailing PAIR is centring.
    trailing_fil: bool,
    /// The depth of `Ctx::iframe_stack` when this block's walk began — the
    /// floor below which its paragraph flushes must not close anything. See
    /// `inline::close_open_wrappers`.
    wrapper_base: usize,
    /// The smallest left offset (pt) of any content on this paragraph's
    /// lines, i.e. its indentation.
    ///
    /// `block-frame-breakable` does not record its horizontal padding as a
    /// marker — `primitives.rs`'s `indent_left` folds `pad_l` into every
    /// contained line's per-box `x` instead. This walker discards `x`
    /// everywhere else (it has no page geometry to replay), so nesting a
    /// frame inside a frame produced no visible indentation at all: an
    /// `enumitem` list, which indents purely this way, came out with every
    /// level flush left.
    indent: Option<f64>,
    /// Every text run so far was fixed-pitch (and there was at least one), so
    /// this paragraph is a code block: its upstream line breaks are real, and
    /// it must not be justified or hyphenated. See `Ctx::mono_run`.
    mono: bool,
    /// Set once a proportional run appears, which disqualifies [`Para::mono`]
    /// for good — a `+code` block never mixes.
    mixed: bool,
}

impl Para {
    fn new(wrapper_base: usize) -> Self {
        Para {
            text: String::new(),
            open: false,
            heading_level: None,
            leading_fil: false,
            trailing_fil: false,
            wrapper_base,
            indent: None,
            mono: false,
            mixed: false,
        }
    }

    /// The `margin-left` declaration for this paragraph's recovered
    /// indentation, or the empty string.
    ///
    /// Suppressed inside a real `<ul>`/`<ol>`: there the markup already
    /// indents, and adding the box stream's own offset on top would double
    /// every level of an instrumented `itemize` list.
    fn indent_decl(&self, in_list: bool) -> String {
        match self.indent {
            Some(x) if x >= INDENT_MIN_PT && !in_list => format!("margin-left:{x}pt;"),
            _ => String::new(),
        }
    }

    /// The CSS `text-align` this paragraph's fil pattern asks for, or `None`
    /// for the stylesheet's default (justified).
    fn alignment(&self) -> Option<&'static str> {
        match (self.leading_fil, self.trailing_fil) {
            (true, true) => Some("center"),
            (true, false) => Some("right"),
            _ => None,
        }
    }
}

/// Walk one flat vertical-box list, appending HTML to `out`. Reentrant: an
/// `EmbeddedBlock` recurses into this same function for its nested
/// `Vec<VertBox>`, so a document with several levels of `embed-block-top`
/// nesting gets genuinely nested `<div class="embed">`s, matching the box
/// tree's own nesting depth exactly (not flattened).
pub(crate) fn walk_vboxes(out: &mut String, vboxes: &[VertBox], ctx: &Ctx) {
    let mut para = Para::new(ctx.iframe_stack.borrow().len());
    let mut pending_margin: f64 = 0.0;
    // S4: `ordered` per currently-open `<ul>`/`<ol>`, pushed by
    // `ListStart`/popped by `ListEnd` — `ListEnd` itself carries no payload
    // (design doc §4.1: not stored redundantly, nesting/kind both fall out
    // of the marker stream's own structure), so this is what lets a
    // `ListEnd` close the right tag. Nesting is automatic: a `ListStart`
    // reached while an `<li>` is open (i.e. between that `<li>`'s
    // `ItemStart`/`ItemEnd`) just emits its `<ul>`/`<ol>` inline in `out`,
    // right where document order puts it — no separate "current parent"
    // bookkeeping needed.
    let mut list_stack: Vec<bool> = Vec::new();

    for vb in vboxes {
        match vb {
            VertBox::Line { contents, .. } => {
                // The line's own left edge. A line that OPENS with an
                // `inline-fil` is aligned, not indented — everything after
                // the fil sits at whatever offset the alignment produced
                // (163pt for a centred table), which is not an indent and
                // must not become one. `data-align` already carries it.
                if !matches!(contents.first(), Some((_, PureHorzBox::OuterFil))) {
                    if let Some((x, _)) = contents.first() {
                        let x = x.0;
                        para.indent = Some(para.indent.map_or(x, |cur: f64| cur.min(x)));
                    }
                }
                // Whether an `EmbeddedBlock` on THIS line is the line's whole
                // point or a word in the middle of it — see
                // [`lone_embedded_block`].
                let embed_is_block = lone_embedded_block(contents);
                for (_, bx) in contents {
                    match bx {
                        // The one inline box that itself carries a nested
                        // block: close whatever paragraph text has been
                        // gathered so far, emit the nested block as its own
                        // `<div>`, then keep accumulating what (rarely, but
                        // legally) follows on the SAME `Line` into a fresh
                        // paragraph.
                        PureHorzBox::EmbeddedBlock { block, .. } if embed_is_block => {
                            flush_para(out, &mut para, &mut pending_margin, ctx, !list_stack.is_empty());
                            let margin = take_margin(&mut pending_margin);
                            let _ = write!(out, "<div class=\"embed\"{margin}>\n");
                            walk_vboxes(out, block, ctx);
                            out.push_str("</div>\n");
                        }
                        // S3 (design doc §3's `Tabular` row, `structure.rs`'s
                        // doc comment "Tables — genuinely recoverable"): like
                        // `EmbeddedBlock`, a `Tabular` is a real block-level
                        // grid, not flowing inline text — flush whatever
                        // paragraph text preceded it (typically just the
                        // `inline-fil` glue `single-centering-line` wraps it
                        // in) and emit a real `<table>` as its own element.
                        PureHorzBox::Tabular(tab) => {
                            flush_para(out, &mut para, &mut pending_margin, ctx, !list_stack.is_empty());
                            let margin = take_margin(&mut pending_margin);
                            structure::render_table(out, tab, &margin, ctx);
                        }
                        // `inline-fil` is not content: it is the alignment
                        // signal, read POSITIONALLY here rather than emitted
                        // (see `Para::leading_fil`). Handled at this level,
                        // not in `inline.rs`, because only the block walker
                        // knows where the paragraph starts and ends.
                        PureHorzBox::OuterFil => {
                            if para.text.trim().is_empty() {
                                para.leading_fil = true;
                            }
                            para.trailing_fil = true;
                            inline::emit_inline(&mut para.text, bx, ctx);
                        }
                        other => {
                            // S3: does THIS box carry (or nest) the `DecoId`
                            // of an outline-registered destination frame?
                            // First match on the paragraph's line(s) wins —
                            // see `Para::heading_level`'s doc comment.
                            if para.heading_level.is_none() {
                                para.heading_level = structure::find_heading_level(other, ctx);
                            }
                            if !para.open {
                                // An `inline-frame-breakable` region that
                                // was cut in half by the previous flush
                                // resumes here — see `Ctx::iframe_stack`.
                                inline::reopen_wrappers(&mut para.text, ctx, para.wrapper_base);
                            }
                            para.open = true;
                            let before = para.text.len();
                            inline::emit_inline(&mut para.text, other, ctx);
                            if para.text.len() != before {
                                para.trailing_fil = false;
                            }
                            if matches!(other, PureHorzBox::InnerString { .. }) {
                                if ctx.mono_run.get() {
                                    para.mono = !para.mixed;
                                } else {
                                    para.mono = false;
                                    para.mixed = true;
                                }
                            }
                        }
                    }
                }
                // A `Line`-to-`Line` boundary within the same paragraph run
                // is exactly the line-break glue the browser is supposed to
                // redo itself — record it as ordinary word-space-width glue
                // so the CJK rule can suppress it between two Japanese
                // characters (an upstream line break falls between two
                // characters that must NOT gain a space when rejoined) and
                // keep it between two Latin words.
                //
                // Unless the line breaker HYPHENATED here, in which case
                // both the hyphen and the space have to go — see
                // `rejoin_hyphenated_word`.
                if para.open {
                    if para.mono {
                        // Fixed-pitch text: the break is the AUTHOR's, not
                        // the line breaker's, so it survives as a `<br>`
                        // rather than collapsing to a space. Without this a
                        // `+code` block arrived as one long line.
                        inline::close_run(&mut para.text, ctx);
                        para.text.push_str("<br>\n");
                        ctx.break_hyphen.set(false);
                        ctx.reset_flow();
                    } else {
                        rejoin_lines(&mut para.text, ctx);
                    }
                }
            }
            // Adjacent vertical margins COLLAPSE — they take the larger,
            // they do not add up. That is SATySFi's own rule (`pagebreak`'s
            // `squash_margins`, upstream `pageBreak.ml:596-601`, which
            // max-collapses a block's top margin against the previous
            // block's bottom margin) and, conveniently, CSS's too, so the
            // emitted `margin-top` means the same thing in both models.
            //
            // Summing them, which is what this did, made every paragraph
            // break in the `latexcmds` manual a 36pt gap where the PDF sets
            // 5.28pt — roughly two blank lines inserted between every pair
            // of paragraphs in the document.
            VertBox::Skip(len) | VertBox::ParagTop(len) | VertBox::FramePad(len) => {
                flush_para(out, &mut para, &mut pending_margin, ctx, !list_stack.is_empty());
                pending_margin = pending_margin.max(len.0);
            }
            VertBox::ClearPage => {
                flush_para(out, &mut para, &mut pending_margin, ctx, !list_stack.is_empty());
                let margin = take_margin(&mut pending_margin);
                let _ = write!(out, "<hr class=\"clearpage\"{margin}>\n");
            }
            // No reflow meaning (design doc §3's mapping table) — dropped.
            VertBox::HookPageBreak(_) => {}
            VertBox::FrameStart(deco) => {
                flush_para(out, &mut para, &mut pending_margin, ctx, !list_stack.is_empty());
                let margin = margin_decl(&mut pending_margin);
                // Real nesting (design doc §3): a `FrameStart`/`FrameEnd`
                // pair opens/closes one `<div>`.
                //
                // The decoration is drawn too, when there is one — see
                // `Ctx::frame_decos`. `fire_hooks` already ran the callback
                // for the PDF; `structure::frame_decoration` turns the
                // resulting graphics into either a CSS background (a plain
                // filled panel) or a scalable SVG (anything else), so a
                // `stdjabook` title block keeps its frame instead of
                // arriving as bare centred text. A frame whose deco draws
                // nothing — the great majority, since packages use
                // `block-frame-breakable` for plain grouping — still gets
                // nothing, which is why this cannot be a blanket CSS border.
                //
                // S2 (design doc §4 "Links/metadata"): `annot.satyh`'s
                // `register-location-frame` fires `register-destination`
                // from THIS frame's own deco (`DecoId` shared by the
                // matching `FrameEnd`) — `ctx.dests` (`DocumentValue::
                // reflow_dests`) resolves it, giving this `<div>` a real
                // `id="…"` a `\href`-to-location's `<a href="#…">` can land
                // on.
                let id_attr = match ctx.dests.get(deco) {
                    Some(name) => format!(" id=\"{}\"", crate::escape_html(name)),
                    None => String::new(),
                };
                let deco_render = structure::frame_decoration(deco, ctx);
                let style = style_attr(&[&margin, &deco_render.style]);
                let _ = write!(
                    out,
                    "<div class=\"frame{}\"{id_attr}{style}>\n{}",
                    deco_render.extra_class, deco_render.svg,
                );
            }
            VertBox::FrameEnd(_deco) => {
                flush_para(out, &mut para, &mut pending_margin, ctx, !list_stack.is_empty());
                out.push_str("</div>\n");
            }
            // S4 ("Block level"): real nested `<ul>`/`<ol>`/`<li>`, the
            // whole point of this slice. Every arm flushes the open
            // paragraph first, same as `FrameStart`/`FrameEnd`/`ClearPage`
            // above — a marker is always a block-level boundary, never
            // mid-paragraph content.
            VertBox::ListMark(kind) => {
                flush_para(out, &mut para, &mut pending_margin, ctx, !list_stack.is_empty());
                let margin = take_margin(&mut pending_margin);
                match kind {
                    ListMarkKind::ListStart { ordered } => {
                        let tag = if *ordered { "ol" } else { "ul" };
                        let _ = write!(out, "<{tag} class=\"list\"{margin}>\n");
                        list_stack.push(*ordered);
                    }
                    ListMarkKind::ListEnd => {
                        // An unmatched `ListEnd` (should not happen — the
                        // marker is always stdlib-paired) closes a `<ul>`
                        // rather than panicking or corrupting later output.
                        let ordered = list_stack.pop().unwrap_or(false);
                        let tag = if ordered { "ol" } else { "ul" };
                        let _ = write!(out, "</{tag}>\n");
                    }
                    ListMarkKind::ItemStart => {
                        let _ = write!(out, "<li{margin}>\n");
                    }
                    ListMarkKind::ItemEnd => {
                        out.push_str("</li>\n");
                    }
                }
            }
        }
    }
    flush_para(out, &mut para, &mut pending_margin, ctx, !list_stack.is_empty());
    // A footnote referenced from a construct that never opens a paragraph (a
    // table cell, a bare frame) would otherwise have nowhere to land. Not a
    // second home for footnotes — just the guarantee that none is dropped.
    drain_footnotes(out, ctx);
}

/// Apply [`crate::recover::line_join`] to a proportional paragraph's
/// `Line`-to-`Line` boundary.
///
/// The CLASSIFICATION lives in `recover`, shared with the Markdown backend —
/// the rule has three cases and getting one wrong is a silently wrong word,
/// so there is one copy of it. What stays here is what the reflow backend
/// DOES with each case, which is not shared: this one accumulates a glue on
/// `Ctx`, and Markdown's own caller does something else with the same answer.
///
/// Called from `inline::emit_embedded_block` too, which walks a nested
/// block's lines for itself. (The fixed-pitch case is NOT here: only the
/// block walker tracks whether a paragraph is set in a monospace face, and
/// only there does a break belong to the author.)
pub(crate) fn rejoin_lines(text: &mut String, ctx: &Ctx) {
    match crate::recover::line_join(ctx.break_hyphen.replace(false), ends_with_hyphen(text)) {
        LineJoin::DropHyphen => drop_break_hyphen(text),
        LineJoin::KeepHyphen => {}
        LineJoin::Space => ctx.note_glue(WORD_SPACE_PT),
    }
}

/// Remove the hyphen the LINE BREAKER put at the end of this line, so a word
/// it split comes back together for the browser to re-break its own way.
///
/// Called only when `InlineMarkKind::BreakHyphen` said the hyphen is the
/// breaker's. That marker is what makes this exact: the splice
/// (`linebreak::line_content`) produces an ordinary `InnerString`, so without
/// it the only available test was the shape of the text — line ends
/// `letter + hyphen`, next line opens with a lowercase ASCII letter — which is
/// also the shape of an AUTHORED compound at a line end. It deleted real
/// hyphens: a paragraph wrapping at `code-printer` rendered as `codeprinter`.
///
/// The hyphen may sit just inside a run span that has since closed, so the
/// closing tag is lifted off and put back.
/// Is the `EmbeddedBlock` on this line the line's WHOLE POINT, or a word in
/// the middle of it?
///
/// The distinction decides which of the two renderings an `embed-block-top`/
/// `-bottom` box gets, and getting it wrong is not a cosmetic matter: the
/// block form flushes the surrounding paragraph and opens a `<div>`, so a box
/// that was one word of a sentence takes the rest of that sentence out of the
/// line with it.
///
/// - **The line's whole point** — a centred figure, a `textbox-with-width`
///   standing alone (`single-centering-line`, `+fig-block`): the box is a real
///   block-level thing that the box stream had no way to express except as a
///   one-box line, and it becomes `<div class="embed">`.
/// - **A word in the middle** — `latexcmds`' `\makebox`/`\framebox`, which is
///   how that package writes "typeset this at a FIXED WIDTH, right here":
///   `\fbox{\makebox(4cm){…}}` puts the embedded block between an
///   `inline-frame-breakable`'s two markers with an `A …  B` of ordinary prose
///   either side. Flushing there ejected the box's text onto a centred line of
///   its own and left the frame around nothing — measured on the probe in this
///   module's own test — so it goes through `inline::emit_embedded_block`
///   instead, as the intrinsically-sized inline-block it is.
///
/// "Whole point" is read off the line rather than guessed: exactly one box
/// that CARRIES anything, and it is the embedded block. Glue, `inline-fil`
/// (the centring idiom itself), kerns and the zero-width markers are not
/// content and do not count. An `InlineFrameMarker` anywhere on the line
/// disqualifies it outright even when the block is otherwise alone: a marker
/// means an inline wrapper is open across this box, and `<div>` inside an
/// open `<span>`/`<a>` is not nesting, it is a paragraph break with the
/// wrapper reopened after it.
fn lone_embedded_block(contents: &[(rustyfi_backend::Length, PureHorzBox)]) -> bool {
    let mut carriers = 0usize;
    let mut embeds = 0usize;
    for (_, bx) in contents {
        match bx {
            PureHorzBox::InlineFrameMarker { .. } => return false,
            PureHorzBox::EmbeddedBlock { .. } => {
                carriers += 1;
                embeds += 1;
            }
            PureHorzBox::OuterEmpty { .. }
            | PureHorzBox::OuterFil
            | PureHorzBox::FixedEmpty { .. }
            | PureHorzBox::Discretionary { .. }
            | PureHorzBox::HookPageBreak { .. }
            | PureHorzBox::FrameMarker { .. }
            | PureHorzBox::InlineMark(_) => {}
            _ => carriers += 1,
        }
    }
    carriers == 1 && embeds == 1
}

/// Whether the line just closed ends with a hyphen — looking through a run
/// span that has since closed, as [`drop_break_hyphen`] does.
fn ends_with_hyphen(text: &str) -> bool {
    let body = text.strip_suffix("</span>").unwrap_or(text);
    body.chars().next_back().is_some_and(crate::recover::is_hyphen)
}

fn drop_break_hyphen(text: &mut String) {
    let closed = text.ends_with("</span>");
    let body = if closed {
        &text[..text.len() - "</span>".len()]
    } else {
        &text[..]
    };
    if !body.chars().next_back().is_some_and(crate::recover::is_hyphen) {
        return;
    }
    let hyphen_len = body.chars().next_back().map_or(0, char::len_utf8);
    let cut = body.len() - hyphen_len;
    let tail = text[body.len()..].to_string();
    text.truncate(cut);
    text.push_str(&tail);
}

/// Close the current paragraph (if any content was gathered), writing `<p
/// class="para"{margin}>{trimmed text}</p>` to `out` — or, when S3's
/// `Para::heading_level` matched ("S3"), `<h{level+1} class="heading"
/// data-outline-level="{level}" {margin}>{trimmed text}</h{level+1}>`
/// instead: same accumulated inline content, same margin bookkeeping, just
/// a promoted tag. A no-op when nothing has been accumulated (e.g. two
/// consecutive `Skip`s, or a `Skip` before any real content) — the same
/// "emit nothing rather than an empty wrapper" guard `svg::emit_graphics`
/// and `inline::emit_graphics_box` make for an empty element list.
///
/// A paragraph whose content is entirely whitespace emits NOTHING and keeps
/// its pending margin for whatever comes next: the box stream is full of
/// lines that hold only an `inline-fil` (the glue `single-centering-line`
/// wraps a table or figure in), and each one used to become an empty `<p>`
/// carrying a stray blank line's worth of leading.
///
/// Footnote bodies queued by `inline.rs` (see its `Footnote` arm) are
/// drained straight after the closing tag, so each lands immediately below
/// the paragraph that referenced it.
fn flush_para(
    out: &mut String,
    para: &mut Para,
    pending_margin: &mut f64,
    ctx: &Ctx,
    in_list: bool,
) {
    if para.open {
        // A run span, and possibly a whole `inline-frame-breakable` wrapper
        // region, may still be open at the end of the paragraph's own
        // buffer. Close everything here, before the buffer is trimmed and
        // wrapped in `<p>` — an inline element may not straddle a block
        // boundary. The wrapper stack is left standing, so the next
        // paragraph's first content re-opens it (`Ctx::iframe_stack`).
        inline::close_open_wrappers(&mut para.text, ctx, para.wrapper_base);
        let trimmed = para.text.trim();
        if super::text::has_visible_content(trimmed) {
            let margin = margin_decl(pending_margin);
            match para.heading_level {
                Some(level) => {
                    let tag = structure::heading_tag(level);
                    let style = style_attr(&[&margin]);
                    let _ = write!(
                        out,
                        "<h{tag} class=\"heading\" data-outline-level=\"{level}\"{style}>{trimmed}</h{tag}>\n"
                    );
                }
                None => {
                    let align = match para.alignment() {
                        Some(a) => format!(" data-align=\"{a}\""),
                        None => String::new(),
                    };
                    let class = if para.mono { "para code" } else { "para" };
                    let style = style_attr(&[&margin, &para.indent_decl(in_list)]);
                    let _ = write!(out, "<p class=\"{class}\"{align}{style}>{trimmed}</p>\n");
                }
            }
            drain_footnotes(out, ctx);
        }
    }
    para.text.clear();
    para.open = false;
    para.heading_level = None;
    para.leading_fil = false;
    para.trailing_fil = false;
    para.indent = None;
    para.mono = false;
    para.mixed = false;
    // A paragraph boundary is a hard boundary for the inline-flow state: a
    // glue recorded at the end of one paragraph must not put a space at the
    // start of the next.
    ctx.reset_flow();
}

/// Emit every queued footnote body as an `<aside>`, in reference order, and
/// clear the queue. Each carries the `id` its `<sup>` reference links to and
/// a back-link to that reference, so the pair navigates in both directions —
/// the continuous-document replacement for "the reader looks down at the
/// foot of the page".
fn drain_footnotes(out: &mut String, ctx: &Ctx) {
    let pending: Vec<(usize, String)> = std::mem::take(&mut *ctx.footnotes.borrow_mut());
    for (n, body) in pending {
        // No number of our own: the body already opens with whatever number
        // or symbol the document assigned the note. The only thing added is
        // the return link to the reference's anchor.
        let _ = write!(
            out,
            "<aside class=\"footnote\" id=\"fn-{n}\" role=\"doc-footnote\">\n{}\
             <a class=\"fnback\" href=\"#fnref-{n}\" aria-label=\"back to reference\">\u{21A9}</a>\
             </aside>\n",
            body.trim(),
        );
    }
}

/// Consume the accumulated `Skip` length as a `style="margin-top:_pt;"`
/// attribute fragment (or the empty string for a zero/negative
/// accumulation — `Length` skips are never legitimately negative, but a
/// stray `0.0` should not emit a vacuous `style=""`).
fn take_margin(pending_margin: &mut f64) -> String {
    style_attr(&[&margin_decl(pending_margin)])
}

/// The accumulated `Skip` as a bare `margin-top:_pt;` declaration, for a
/// caller that has more than one declaration to write.
fn margin_decl(pending_margin: &mut f64) -> String {
    let m = *pending_margin;
    *pending_margin = 0.0;
    if m > 0.0 {
        format!("margin-top:{m}pt;")
    } else {
        String::new()
    }
}

/// Join CSS declarations into one ` style="…"` attribute fragment, or the
/// empty string when they are all empty — a single element may carry only one
/// `style` attribute, so the pieces have to be assembled before it is written.
fn style_attr(decls: &[&str]) -> String {
    let joined: String = decls.concat();
    if joined.is_empty() {
        String::new()
    } else {
        format!(" style=\"{joined}\"")
    }
}

/// Below this (pt) a recovered left offset is a kern or a rounding artefact,
/// not an indent worth reproducing.
const INDENT_MIN_PT: f64 = 1.0;
