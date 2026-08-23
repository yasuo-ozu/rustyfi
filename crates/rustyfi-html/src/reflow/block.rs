//! The block-level `Vec<VertBox>` walker ("Block level"). Unlike the faithful
//! backend's `Page`/`PlacedLine` walk (already-placed, absolutely positioned),
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

/// Accumulated state for "the paragraph currently being built" — `text` is
/// the flowing inline HTML gathered so far (escaped/styled runs, glue
/// collapsed to plain spaces so the BROWSER re-breaks lines, unlike the
/// faithful mode's fixed per-glyph positioning); `open` distinguishes "no
/// paragraph started yet" (nothing to flush) from "a paragraph with only
/// whitespace so far" (still flushed as an empty-ish `<p>`, matching
/// upstream's own willingness to lay out a blank line).
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
}

impl Para {
    fn new() -> Self {
        Para {
            text: String::new(),
            open: false,
            heading_level: None,
            leading_fil: false,
            trailing_fil: false,
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
    let mut para = Para::new();
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
                // The line breaker has already run, and this backend is
                // undoing it. A break it TOOK at a hyphenation point left the
                // discretionary's `pre_break` — the hyphen — spliced in as
                // the line's own last box (`linebreak.rs`'s `line_content`).
                // Rejoined into flowing text that hyphen is not in the
                // document: `figbox`, broken as `fig-` / `box`, came out as
                // `fig- box`. Drop it and put back the soft hyphen it stands
                // for, so the BROWSER decides whether to show one.
                let spliced = spliced_hyphen_index(contents);
                let body = &contents[..spliced.unwrap_or(contents.len())];
                for (_, bx) in body {
                    match bx {
                        // The one inline box that itself carries a nested
                        // block: close whatever paragraph text has been
                        // gathered so far, emit the nested block as its own
                        // `<div>`, then keep accumulating what (rarely, but
                        // legally) follows on the SAME `Line` into a fresh
                        // paragraph.
                        PureHorzBox::EmbeddedBlock { block, .. } => {
                            flush_para(out, &mut para, &mut pending_margin, ctx);
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
                            flush_para(out, &mut para, &mut pending_margin, ctx);
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
                            para.open = true;
                            let before = para.text.len();
                            inline::emit_inline(&mut para.text, other, ctx);
                            if para.text.len() != before {
                                para.trailing_fil = false;
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
                // Except after a hyphen, in EITHER of its two forms: the
                // breaker's own, replaced by a soft hyphen just above, and
                // one that was in the text all along (`align-` / `right` is
                // a UAX#14 break AFTER the hyphen, which stays part of the
                // run). Both rejoin as one word. This is deliberately not a
                // rule in `text::wants_space`: a real glue next to a dash
                // (`SATySFi - a typesetter`) is a real space, and only THIS
                // boundary is a line break being undone.
                if spliced.is_some() {
                    para.text.push_str("&shy;");
                    ctx.last_char.set(None);
                } else if para.open && !ctx.last_char.get().is_some_and(is_hyphen) {
                    ctx.note_glue(WORD_SPACE_PT);
                }
            }
            VertBox::Skip(len) | VertBox::ParagTop(len) | VertBox::FramePad(len) => {
                flush_para(out, &mut para, &mut pending_margin, ctx);
                pending_margin += len.0;
            }
            VertBox::ClearPage => {
                flush_para(out, &mut para, &mut pending_margin, ctx);
                let margin = take_margin(&mut pending_margin);
                let _ = write!(out, "<hr class=\"clearpage\"{margin}>\n");
            }
            // No reflow meaning (design doc §3's mapping table) — dropped.
            VertBox::HookPageBreak(_) => {}
            VertBox::FrameStart(deco) => {
                flush_para(out, &mut para, &mut pending_margin, ctx);
                let margin = take_margin(&mut pending_margin);
                // Real nesting (design doc §3): a `FrameStart`/`FrameEnd`
                // pair opens/closes one `<div>` — the decoration itself
                // (`DecoId`) is a lang-side callback this backend can't run
                // (same documented gap the faithful mode has for block-frame
                // decos), so only a generic `.frame` class + this pending
                // margin ride along; a future slice can resolve common decos
                // to CSS (design doc §6, Slice 3).
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
                let _ = write!(out, "<div class=\"frame\"{id_attr}{margin}>\n");
            }
            VertBox::FrameEnd(_deco) => {
                flush_para(out, &mut para, &mut pending_margin, ctx);
                out.push_str("</div>\n");
            }
            // S4 ("Block level"): real nested `<ul>`/`<ol>`/`<li>`, the
            // whole point of this slice. Every arm flushes the open
            // paragraph first, same as `FrameStart`/`FrameEnd`/`ClearPage`
            // above — a marker is always a block-level boundary, never
            // mid-paragraph content.
            VertBox::ListMark(kind) => {
                flush_para(out, &mut para, &mut pending_margin, ctx);
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
    flush_para(out, &mut para, &mut pending_margin, ctx);
    // A footnote (or a nested table) reached from a construct that never
    // opens a paragraph — a table cell, a bare frame — would otherwise have
    // nowhere to land. Not a second home for either, just the guarantee that
    // none is dropped.
    drain_deferred_blocks(out, ctx);
    drain_footnotes(out, ctx);
}

/// A plain inter-word space, in points, standing in for the line break
/// between two consecutive `Line`s of one paragraph. The exact value is
/// immaterial — it only has to be above `text::wants_space`'s zero-width
/// threshold, since the decision it feeds is "is this a CJK pair" rather
/// than "how wide".
const WORD_SPACE_PT: f64 = 3.0;

/// The hyphens a broken word can end a line on: ASCII, Unicode HYPHEN, and
/// the non-breaking one (which a break should never fall after, but costs
/// nothing to cover).
fn is_hyphen(c: char) -> bool {
    matches!(c, '-' | '\u{2010}' | '\u{2011}')
}

/// The index of the hyphen a TAKEN discretionary contributed to the end of
/// this line, if it did. `linebreak.rs`'s `line_content` appends the chosen
/// break's `pre_break` slot last and nothing after it, and the hyphenation
/// dictionary's `pre_break` is a single one-character run — whereas a hyphen
/// that was in the SOURCE is part of a longer chunk (`align-`), since UAX#14
/// breaks after it rather than splitting it off. So a lone trailing
/// hyphen-only run is the breaker's, and only the breaker's.
fn spliced_hyphen_index(contents: &[(rustyfi_backend::Length, PureHorzBox)]) -> Option<usize> {
    let last = contents.len().checked_sub(1)?;
    match &contents[last].1 {
        PureHorzBox::InnerString { text, .. } => {
            let mut chars = text.chars();
            let one_hyphen = chars.next().is_some_and(is_hyphen) && chars.next().is_none();
            one_hyphen.then_some(last)
        }
        _ => None,
    }
}

/// Close the current paragraph (if any content was gathered), writing `<p
/// class="para"{margin}>{trimmed text}</p>` to `out` — or, when S3's
/// `Para::heading_level` matched ("S3"), `<h{level+1} class="heading"
/// data-outline-level="{level}" {margin}>{trimmed text}</h{level+1}>`
/// instead: same accumulated inline content, same margin bookkeeping, just
/// a promoted tag. A no-op when nothing has been accumulated (e.g. two
/// consecutive `Skip`s, or a `Skip` before any real content) — mirrors
/// `render_html_impl`'s own "nothing to emit" guards elsewhere in this
/// crate.
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
fn flush_para(out: &mut String, para: &mut Para, pending_margin: &mut f64, ctx: &Ctx) {
    if para.open {
        let trimmed = para.text.trim();
        if !trimmed.is_empty() {
            let margin = take_margin(pending_margin);
            match para.heading_level {
                Some(level) => {
                    let tag = structure::heading_tag(level);
                    let _ = write!(
                        out,
                        "<h{tag} class=\"heading\" data-outline-level=\"{level}\"{margin}>{trimmed}</h{tag}>\n"
                    );
                }
                None => {
                    let align = match para.alignment() {
                        Some(a) => format!(" data-align=\"{a}\""),
                        None => String::new(),
                    };
                    let _ = write!(out, "<p class=\"para\"{align}{margin}>{trimmed}</p>\n");
                }
            }
            drain_deferred_blocks(out, ctx);
            drain_footnotes(out, ctx);
        }
    }
    // A paragraph that emitted nothing (whitespace only) can still have
    // queued a block — a `Line` holding only an `inline-fil` and a nested
    // `Tabular` is exactly that — so this drain is outside the guard above.
    drain_deferred_blocks(out, ctx);
    para.text.clear();
    para.open = false;
    para.heading_level = None;
    para.leading_fil = false;
    para.trailing_fil = false;
    // A paragraph boundary is a hard boundary for the inline-flow state: a
    // glue recorded at the end of one paragraph must not put a space at the
    // start of the next.
    ctx.reset_flow();
}

/// Emit every block-level element that turned up mid-paragraph (a nested
/// `<table>`, an `EmbeddedBlock` drawn into a graphic) and clear the queue —
/// see [`super::Ctx::deferred_blocks`]. Ordered before the footnotes: the
/// table is part of what the paragraph SAYS, the footnote is a note about it.
fn drain_deferred_blocks(out: &mut String, ctx: &Ctx) {
    for html in std::mem::take(&mut *ctx.deferred_blocks.borrow_mut()) {
        out.push_str(&html);
    }
}

/// Emit every queued footnote body as an `<aside>`, in reference order, and
/// clear the queue. Each carries the `id` its `<sup>` reference links to and
/// a back-link to that reference, so the pair navigates in both directions —
/// the continuous-document replacement for "the reader looks down at the
/// foot of the page".
fn drain_footnotes(out: &mut String, ctx: &Ctx) {
    let pending: Vec<(usize, String)> = std::mem::take(&mut *ctx.footnotes.borrow_mut());
    for (n, body) in pending {
        let _ = write!(
            out,
            "<aside class=\"footnote\" id=\"fn-{n}\" role=\"doc-footnote\">\
             <a class=\"fnback\" href=\"#fnref-{n}\">{n}</a>\n{}</aside>\n",
            body.trim(),
        );
    }
}

/// Consume the accumulated `Skip` length as a `style="margin-top:_pt;"`
/// attribute fragment (or the empty string for a zero/negative
/// accumulation — `Length` skips are never legitimately negative, but a
/// stray `0.0` should not emit a vacuous `style=""`).
fn take_margin(pending_margin: &mut f64) -> String {
    let m = *pending_margin;
    *pending_margin = 0.0;
    if m > 0.0 {
        format!(" style=\"margin-top:{m}pt;\"")
    } else {
        String::new()
    }
}
