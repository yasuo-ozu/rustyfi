//! The block-level `Vec<VertBox>` walker (`docs/plans/design-reflowable-html.md`
//! §3 "Block level"). Unlike the faithful backend's `Page`/`PlacedLine` walk
//! (already-placed, absolutely positioned), this walks the FLAT pre-page-break
//! list with a single linear pass, using a small amount of local state (a
//! "current paragraph" text accumulator and a "pending top margin" carried
//! from the last `Skip`) instead of a `Page`'s already-resolved geometry —
//! there is no x/y here at all, only document order.
//!
//! | `VertBox` | emitted as |
//! |--|--|
//! | a run of consecutive `Line`s | one `<p class="para">` (flowing text) |
//! | `Skip(len)` | closes the current paragraph; `len` becomes the NEXT block element's `margin-top` |
//! | `FrameStart`/`FrameEnd` | a real nested `<div class="frame">` (push/pop) |
//! | `ClearPage` | a soft `<hr class="clearpage">` separator (pagination is meaningless once reflowed) |
//! | `HookPageBreak` | dropped (no reflow meaning) |
//! | `ListMark(ListStart{ordered})`/`ListEnd`/`ItemStart`/`ItemEnd` | S4 (`docs/plans/design-reflow-s4-lists.md`): real nested `<ul>`/`<ol>`/`<li>` (push/pop, tracked by `list_stack` below) |
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
    /// S3 (`docs/plans/design-reflowable-html.md` §6 "S3" — see
    /// `structure.rs`'s doc comment): set once, the first time
    /// `structure::find_heading_level` matches a box on this paragraph's
    /// line(s), by an outline-registered destination `Frame`. When `Some`,
    /// `flush_para` emits `<h{level+1}>` instead of `<p class="para">`.
    /// Never reset mid-paragraph (a paragraph's first matching `Frame`
    /// decides its tag; further boxes on the same line(s) cannot un-decide
    /// it), only on the next `flush_para`.
    heading_level: Option<i64>,
}

impl Para {
    fn new() -> Self {
        Para {
            text: String::new(),
            open: false,
            heading_level: None,
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
    // S4 (`docs/plans/design-reflow-s4-lists.md` §4.2): `ordered` per
    // currently-open `<ul>`/`<ol>`, pushed by `ListStart`/popped by
    // `ListEnd` — `ListEnd` itself carries no payload (design doc §4.1: not
    // stored redundantly, nesting/kind both fall out of the marker stream's
    // own structure), so this is what lets a `ListEnd` close the right tag.
    // Nesting is automatic: a `ListStart` reached while an `<li>` is open
    // (i.e. between that `<li>`'s `ItemStart`/`ItemEnd`) just emits its
    // `<ul>`/`<ol>` inline in `out`, right where document order puts it —
    // no separate "current parent" bookkeeping needed.
    let mut list_stack: Vec<bool> = Vec::new();

    for vb in vboxes {
        match vb {
            VertBox::Line { contents, .. } => {
                for (_, bx) in contents {
                    match bx {
                        // The one inline box that itself carries a nested
                        // block: close whatever paragraph text has been
                        // gathered so far, emit the nested block as its own
                        // `<div>`, then keep accumulating what (rarely, but
                        // legally) follows on the SAME `Line` into a fresh
                        // paragraph.
                        PureHorzBox::EmbeddedBlock { block, .. } => {
                            flush_para(out, &mut para, &mut pending_margin);
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
                            flush_para(out, &mut para, &mut pending_margin);
                            let margin = take_margin(&mut pending_margin);
                            structure::render_table(out, tab, &margin, ctx);
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
                            inline::emit_inline(&mut para.text, other, ctx);
                        }
                    }
                }
                // A `Line`-to-`Line` boundary within the same paragraph run
                // is exactly the line-break glue the browser is supposed to
                // redo itself — collapse it to a single space (the eager
                // upstream break is discarded on purpose, design doc §3).
                if para.open {
                    para.text.push(' ');
                }
            }
            VertBox::Skip(len) | VertBox::ParagTop(len) | VertBox::FramePad(len) => {
                flush_para(out, &mut para, &mut pending_margin);
                pending_margin += len.0;
            }
            VertBox::ClearPage => {
                flush_para(out, &mut para, &mut pending_margin);
                let margin = take_margin(&mut pending_margin);
                let _ = write!(out, "<hr class=\"clearpage\"{margin}>\n");
            }
            // No reflow meaning (design doc §3's mapping table) — dropped.
            VertBox::HookPageBreak(_) => {}
            VertBox::FrameStart(deco) => {
                flush_para(out, &mut para, &mut pending_margin);
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
                flush_para(out, &mut para, &mut pending_margin);
                out.push_str("</div>\n");
            }
            // S4 (`docs/plans/design-reflow-s4-lists.md` §4.2 "Block
            // level"): real nested `<ul>`/`<ol>`/`<li>`, the whole point of
            // this slice. Every arm flushes the open paragraph first, same
            // as `FrameStart`/`FrameEnd`/`ClearPage` above — a marker is
            // always a block-level boundary, never mid-paragraph content.
            VertBox::ListMark(kind) => {
                flush_para(out, &mut para, &mut pending_margin);
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
    flush_para(out, &mut para, &mut pending_margin);
}

/// Close the current paragraph (if any content was gathered), writing
/// `<p class="para"{margin}>{trimmed text}</p>` to `out` — or, when S3's
/// `Para::heading_level` matched (`docs/plans/design-reflowable-html.md`
/// §6 "S3"), `<h{level+1} class="heading" data-outline-level="{level}"
/// {margin}>{trimmed text}</h{level+1}>` instead: same accumulated inline
/// content, same margin bookkeeping, just a promoted tag. A no-op when
/// nothing has been accumulated (e.g. two consecutive `Skip`s, or a `Skip`
/// before any real content) — mirrors `render_html_impl`'s own "nothing to
/// emit" guards elsewhere in this crate.
fn flush_para(out: &mut String, para: &mut Para, pending_margin: &mut f64) {
    if para.open {
        let margin = take_margin(pending_margin);
        let trimmed = para.text.trim();
        match para.heading_level {
            Some(level) => {
                let tag = structure::heading_tag(level);
                let _ = write!(
                    out,
                    "<h{tag} class=\"heading\" data-outline-level=\"{level}\"{margin}>{trimmed}</h{tag}>\n"
                );
            }
            None => {
                let _ = write!(out, "<p class=\"para\"{margin}>{trimmed}</p>\n");
            }
        }
    }
    para.text.clear();
    para.open = false;
    para.heading_level = None;
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
