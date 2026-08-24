//! `PureHorzBox` -> paragraph [`Piece`]s.
//!
//! The mapping is the HTML backend's, minus everything Markdown has no way to
//! say. What survives, and how it is recovered, is in each arm below; the two
//! that are worth reading before the rest are `Footnote` (which has to
//! suppress the marker the document typeset for itself) and `Graphics` (which
//! is where a drawing goes when the format has no drawings).

use rustyfi_backend::{
    AnnotAction, DecoId, GraphicsElem, HorzStringInfo, InlineMarkKind, Length, PureHorzBox,
};

use super::math;
use super::para::{Para, Piece};
use super::Ctx;

/// Append `bx`'s Markdown rendering to the paragraph being built.
pub(super) fn emit_inline(para: &mut Para, bx: &PureHorzBox, ctx: &Ctx) {
    match bx {
        // Handled FIRST, unconditionally of the bullet fence below: a
        // `BulletEnd` reached while suppressed must still clear the counter,
        // and an emphasis marker must still keep its stack balanced.
        PureHorzBox::InlineMark(kind) => match kind {
            InlineMarkKind::EmphStart { strong } => {
                let delim = if *strong { "**" } else { "*" };
                ctx.emph_stack.borrow_mut().push(delim);
                para.pieces.push(Piece::EmphOpen(delim));
            }
            InlineMarkKind::EmphEnd => {
                let delim = ctx.emph_stack.borrow_mut().pop().unwrap_or("*");
                para.pieces.push(Piece::EmphClose(delim));
            }
            // The drawn bullet glyph between this fence is dropped: the
            // `- `/`1. ` the list writes replaces it.
            InlineMarkKind::BulletStart => ctx.bullet_suppress.set(ctx.bullet_suppress.get() + 1),
            InlineMarkKind::BulletEnd => {
                ctx.bullet_suppress.set(ctx.bullet_suppress.get().saturating_sub(1))
            }
            // Everything after this on the line is the LINE BREAKER's hyphen,
            // not the author's. `block.rs` undoes it when it rejoins the
            // lines; all this arm does is say so.
            InlineMarkKind::BreakHyphen => ctx.break_hyphen.set(true),
        },

        // `inline-frame-breakable` splices its contents between a marker PAIR
        // rather than nesting them, so a link built this way — which is how
        // `annot.satyh` writes every `\href` and `\ref` — has to be opened
        // and closed positionally. Ahead of the bullet guard for the same
        // reason as `InlineMark`: a marker skipped while suppressed would
        // leave the stack unbalanced.
        PureHorzBox::InlineFrameMarker { id, end, .. } => {
            if *end {
                if ctx.iframe_stack.borrow_mut().pop().unwrap_or(false) {
                    para.pieces.push(Piece::LinkClose);
                }
            } else {
                let opened = open_link(para, id, ctx);
                ctx.iframe_stack.borrow_mut().push(opened);
            }
        }

        // While a bullet fence is open every other box renders nothing.
        _ if ctx.bullet_suppress.get() > 0 => {}

        PureHorzBox::InnerString {
            info, text, width, ..
        } => emit_run(para, info, text, width.0, ctx),

        // Glue is RECORDED, not written: whether it is a space depends on the
        // character that follows (`crate::recover::wants_space`), which is
        // why Japanese does not come out as `研 究 計 画`. Inside fixed-pitch
        // text it is not a word space at all but a measured column gap, and
        // is kept as one — see `Piece::Gap`.
        PureHorzBox::OuterEmpty { natural, .. } => {
            if ctx.mono_run.get() {
                para.pieces.push(Piece::Gap(natural.0));
            } else {
                ctx.note_glue(natural.0);
            }
        }
        // `inline-fil`: infinite stretch, no natural width. It carries the
        // document's alignment, which Markdown cannot express, so it is
        // nothing but a break opportunity here.
        PureHorzBox::OuterFil => ctx.note_glue(0.0),
        // `inline-skip`: an explicit, deliberately-sized gap. Always a
        // `Gap` — inside a code block that is the indentation, and in prose
        // `Para::render` reduces it to the single space it is worth.
        PureHorzBox::FixedEmpty { width } => {
            if ctx.mono_run.get() {
                para.pieces.push(Piece::Gap(width.0));
            } else if width.0 >= HSKIP_MIN_PT {
                ctx.resolve_glue(para, None);
                para.push_text(" ", false);
                ctx.last_char.set(Some(' '));
            } else {
                ctx.note_glue(0.0);
            }
        }

        // A break point the paragraph breaker did NOT take, so the word it
        // offered to split stays whole. Nothing is written: Markdown's only
        // spelling for a conditional hyphen is a literal U+00AD, an invisible
        // character that would break search and copy-paste for a break the
        // reader's own renderer is going to redo anyway. (A break the
        // paragraph breaker DID take does not arrive here — `line_content`
        // splices it as real text plus an `InlineMarkKind::BreakHyphen`.)
        PureHorzBox::Discretionary { pre_break, .. } => {
            if !pre_break_carries_text(pre_break) {
                ctx.note_glue(glue_width(pre_break));
            }
        }

        // An unbreakable inline frame. Its decoration is dropped — Markdown
        // has no borders — but its `DecoId` may still make it a link.
        PureHorzBox::Frame { deco, contents, .. } => {
            let opened = open_link(para, deco, ctx);
            for (_, cbx) in contents {
                emit_inline(para, cbx, ctx);
            }
            if opened {
                para.pieces.push(Piece::LinkClose);
            }
        }

        // Math: its characters, in reading order. See `math.rs` for what that
        // recovers, what it costs, and why an inline `<svg>` was rejected.
        PureHorzBox::Math { glyphs, rules, .. } => {
            // The `rules` are the paths a font cannot draw, but they also
            // carry any `draw-text` the construction built — and a BIG
            // OPERATOR arrives that way rather than as a `MathGlyph`. Emitted
            // FIRST, because a `draw-text` operator sits at the box's own
            // origin: `\sum_a^b` is a sigma with limits, and putting the
            // sigma after them would read as `ₐᵇ∑`.
            emit_nested_text(para, rules, ctx);
            let text = math::math_text(glyphs, rules);
            if !text.is_empty() {
                ctx.resolve_glue(para, text.chars().next());
                ctx.last_char.set(text.chars().next_back());
                para.push_text(&text, false);
                ctx.mono_run.set(false);
            }
        }

        // Graphics have no Markdown counterpart at all — see this module's
        // `graphic_placeholder` for the choice and its cost.
        PureHorzBox::Graphics { elems, .. } => {
            // A graphics box whose every element is a `draw-text` draws
            // nothing: it is a positioning wrapper, and everything a reader
            // wants from it is in the nested boxes. `easytable` wraps every
            // table in exactly this shape.
            if elems.iter().all(is_pure_text) {
                emit_nested_text(para, elems, ctx);
            } else {
                graphic_placeholder(para, elems, ctx);
            }
        }
        // A DEFERRED lang-side callback, always resolved to a plain
        // `Graphics` well before `reflow_source` is captured — so this arm is
        // realistically unreachable. There is nothing to measure and nothing
        // to draw, so it says nothing rather than claiming a figure.
        PureHorzBox::GraphicsOuter { .. } => {}

        // A real image, as a REFERENCE-style link whose definition is
        // collected at the foot of the document — see `Ctx::image_ref`.
        PureHorzBox::Image { image, .. } => {
            ctx.open_opaque(para);
            match ctx.images.get(image.0) {
                // `load-pdf-image` carries no raster samples at all, and
                // rasterizing a PDF page is out of scope here as it is for
                // the HTML backend.
                Some(res) if res.pdf.is_some() => {
                    para.push_markup("\\[embedded PDF page\\]", "[embedded PDF page]")
                }
                Some(_) => {
                    let (label, n) = ctx.image_ref(image.0);
                    para.push_markup(format!("![image {n}][{label}]"), format!("[image {n}]"));
                }
                // Out-of-range `ImageId` — should not happen; the PDF writer
                // skips rather than panics, and so does this.
                None => {}
            }
        }

        // A table is block-level in Markdown — a pipe table cannot sit inside
        // a sentence — so it is QUEUED here and emitted by `block.rs` as its
        // own block once the surrounding paragraph closes. This arm is the
        // main path, not a fallback: `easytable` reaches a `tabular` through
        // a `draw-text` inside an `inline-graphics`, so by the time one is
        // seen it is already inside inline content.
        PureHorzBox::Tabular(tab) => {
            ctx.open_opaque(para);
            if let Some(md) = super::table::render_table(tab, ctx) {
                ctx.pending_blocks.borrow_mut().push(md);
            }
        }

        // A GFM footnote: `[^n]` here, the body collected at the foot of the
        // document. That is a better fit than the HTML backend's in-flow
        // `<aside>` — Markdown has a real footnote construct, and readers'
        // renderers already know where to put one.
        PureHorzBox::Footnote { block } => {
            ctx.open_opaque(para);
            let n = ctx.footnote_seq.get() + 1;
            ctx.footnote_seq.set(n);
            // Rendered NOW, where the marker rides, so it sees the
            // surrounding context — but into its own writer, with the
            // inline-flow state saved across it so the note's last character
            // does not decide the spacing of the word after the reference.
            let saved = (ctx.pending_glue.take(), ctx.last_char.take(), ctx.mono_run.get());
            // Park both queues. The nested walk drains whatever it finds in
            // them when it finishes, so without this an earlier sibling
            // footnote from this same paragraph — or a table the paragraph
            // queued before reaching this marker — would be emitted INSIDE
            // this note's body.
            let parked = std::mem::take(&mut *ctx.footnotes.borrow_mut());
            let parked_blocks = std::mem::take(&mut *ctx.pending_blocks.borrow_mut());
            let body = super::block::render_block(block, ctx);
            *ctx.pending_blocks.borrow_mut() = parked_blocks;
            ctx.pending_glue.set(saved.0);
            ctx.last_char.set(saved.1);
            ctx.mono_run.set(saved.2);
            let mut queue = ctx.footnotes.borrow_mut();
            *queue = parked;
            queue.push((n, body));
            drop(queue);
            para.push_markup(format!("[^{n}]"), "");
            // The document has ALREADY typeset its own reference marker —
            // `stdjabook`'s `\footnote` sets a raised, three-quarter-size
            // numeral immediately after this box (`footnote-scheme.satyh:79`
            // emits `add-footnote … ++ ib-num`). Left in, the text would read
            // `[^1]*1` and the note would be numbered twice. `set-manual-rising`
            // is what makes it identifiable: raised text is otherwise
            // vanishingly rare in body copy, so this suppresses the next run
            // only while it is raised, and only immediately here.
            ctx.drop_fn_marker.set(true);
        }

        // Handled one level up, in `block.rs` — it has to CLOSE the open
        // paragraph, which this function cannot do. Kept as an explicit inert
        // arm so a new `PureHorzBox` variant still forces a compile error
        // here rather than silently falling through.
        PureHorzBox::EmbeddedBlock { .. } => {}

        // Zero-width markers and hooks: no meaning in any reflowed format.
        PureHorzBox::HookPageBreak { .. } | PureHorzBox::FrameMarker { .. } => {}
    }
}

/// A `FixedEmpty` (`inline-skip`) at least this wide (pt) is a deliberate gap
/// worth one space; anything narrower is a KERN and renders as nothing. The
/// same two populations the HTML backend separates here — a paragraph indent
/// or a table cell's padding above, the `\LaTeX` logo's kerns and a
/// table-of-contents leader's dot spacing below.
const HSKIP_MIN_PT: f64 = 2.0;

/// One `InnerString` run.
fn emit_run(para: &mut Para, info: &HorzStringInfo, text: &str, width: f64, ctx: &Ctx) {
    if text.is_empty() {
        return;
    }
    // The footnote reference marker the document typeset for itself — see the
    // `Footnote` arm. Dropped for as long as the runs stay RAISED, not just
    // for one: `stdjabook`'s marker is `\*#it-num;`, which is two runs (the
    // symbol and the numeral) inside one `set-manual-rising` context, so a
    // one-shot suppression ate the `*` and printed the `1`. The first run at
    // the ordinary baseline is where the note ends and the sentence resumes.
    if ctx.drop_fn_marker.get() {
        if info.rising.0 > 0.0 {
            return;
        }
        ctx.drop_fn_marker.set(false);
    }
    let mono = crate::recover::is_monospace(ctx.fonts, Some(info.font));
    ctx.resolve_glue(para, text.chars().next());
    ctx.last_char.set(text.chars().next_back());
    ctx.mono_run.set(mono);
    // The fixed-pitch advance, measured rather than assumed: in a fixed-pitch
    // face every character is one advance wide, so a run's own width divided
    // by its character count IS the column width a code block's indentation
    // is counted in. Only a run of plain ASCII is measured — a fixed-pitch
    // Latin face still sets a stray CJK character full-width, which would
    // halve the estimate.
    if mono && ctx.mono_advance.get().is_none() {
        let n = text.chars().count();
        if n > 0 && width > 0.0 && text.chars().all(|c| c.is_ascii_graphic()) {
            ctx.mono_advance.set(Some(width / n as f64));
        }
    }
    para.push_text(text, mono);
}

/// Open a link for `deco` if one was registered against it, returning whether
/// anything was pushed (so the caller knows whether to close it).
///
/// `Uri` becomes a real `[text](url)`. `GotoName` — a `\ref` to a section —
/// becomes PLAIN TEXT: Markdown has no document-wide anchor scheme, and a
/// renderer generates heading anchors from the heading's own words, so
/// `[Section 3](#sec:intro)` would be a link that goes nowhere. The
/// cross-reference text the document typeset ("Section 3", "Figure 2") is
/// already what a reader needs.
fn open_link(para: &mut Para, deco: &DecoId, ctx: &Ctx) -> bool {
    match ctx.links.get(deco) {
        Some(AnnotAction::Uri(uri)) => {
            para.pieces.push(Piece::LinkOpen(uri.clone()));
            true
        }
        _ => false,
    }
}

/// What a drawing becomes: the drawing itself, as an `<svg>`.
///
/// Markdown has no vector-drawing syntax, but every Markdown target of
/// consequence accepts raw HTML, and the paths are already to hand — the same
/// `GraphicsElem`s the PDF writer strokes and fills. So the figure survives
/// instead of leaving a hole, and it survives as VECTOR: no rasterizer is
/// needed, which matters because this repo does not have one.
///
/// The cost is real and worth stating. A renderer that sanitizes HTML —
/// GitHub's comment fields, most static-site pipelines — drops the `<svg>`
/// and leaves nothing in its place, which is strictly worse than the
/// `[graphic]` hole this replaced. That is the trade: the common case
/// (a file read locally, in an editor preview, or by any renderer that
/// passes HTML through) gains the actual picture.
///
/// Below the size threshold nothing is written at all. That is not a
/// rounding-off: the corpus is full of hairline rules, leader dots and
/// underline strokes drawn as one-off graphics, and marking each of them
/// would bury the drawings that matter under punctuation.
///
/// The size measured is the INK's, not the box's, and that distinction is
/// load-bearing. `stdjabook` draws the rule under a section heading as a
/// 440pt x 1pt line inside a 440pt x 4pt box; judged by the box it is a
/// figure, and `easytable`'s manual grew a `[graphic]` above and below every
/// heading in it. Judged by the ink it is what it is.
fn graphic_placeholder(para: &mut Para, elems: &[GraphicsElem], ctx: &Ctx) {
    let Some(((lo_x, lo_y), (hi_x, hi_y))) = elems
        .iter()
        .filter_map(rustyfi_backend::graphics_bbox)
        .reduce(|(alo, ahi), (blo, bhi)| {
            (
                (Length(alo.0 .0.min(blo.0 .0)), Length(alo.1 .0.min(blo.1 .0))),
                (Length(ahi.0 .0.max(bhi.0 .0)), Length(ahi.1 .0.max(bhi.1 .0))),
            )
        })
    else {
        return;
    };
    if hi_x.0 - lo_x.0 < GRAPHIC_MIN_PT || hi_y.0 - lo_y.0 < GRAPHIC_MIN_PT {
        return;
    }
    ctx.open_opaque(para);
    match crate::svg::graphics_block(elems, ctx.fonts) {
        // `plain` stays a named hole: it feeds the plain-text side of the
        // paragraph, which is what content measurement and search read, and
        // where a wall of path data would be worse than useless.
        Some(svg) => para.push_markup(svg, "[graphic]"),
        None => para.push_markup("\\[graphic\\]", "[graphic]"),
    }
}

/// Smaller than this in either dimension (pt) and a drawing's INK is a rule,
/// a leader dot or a piece of underlining, not a figure.
const GRAPHIC_MIN_PT: f64 = 4.0;

/// Whether `elem` contributes no ink of its own — a `draw-text`, or a group
/// containing only those.
fn is_pure_text(elem: &GraphicsElem) -> bool {
    match elem {
        GraphicsElem::Text { .. } => true,
        GraphicsElem::Group(inner) | GraphicsElem::Clip(_, inner) => inner.iter().all(is_pure_text),
        _ => false,
    }
}

/// Emit the boxes nested inside `draw-text` elements, in document order.
///
/// A `draw-text` run's boxes are placed at a point inside the drawing, and
/// there is no drawing here to place them in, so they simply flow — the same
/// documented approximation the HTML backend makes.
fn emit_nested_text(para: &mut Para, elems: &[GraphicsElem], ctx: &Ctx) {
    for elem in elems {
        match elem {
            GraphicsElem::Text { contents, .. } => {
                for (_, cbx) in contents {
                    emit_inline(para, cbx, ctx);
                }
            }
            GraphicsElem::Group(inner) | GraphicsElem::Clip(_, inner) => {
                emit_nested_text(para, inner, ctx)
            }
            _ => {}
        }
    }
}

/// Does a `Discretionary`'s `pre_break` carry a visible character (the
/// hyphenation dictionary's hyphen), as opposed to bare glue?
fn pre_break_carries_text(pre_break: &[PureHorzBox]) -> bool {
    pre_break.iter().any(|b| match b {
        PureHorzBox::InnerString { text, .. } => !text.is_empty(),
        _ => false,
    })
}

/// The total natural width of a `Discretionary`'s `pre_break` glue, fed to
/// the ordinary glue rule when there is no hyphen to show.
fn glue_width(pre_break: &[PureHorzBox]) -> f64 {
    pre_break
        .iter()
        .map(|b| match b {
            PureHorzBox::OuterEmpty { natural, .. } => natural.0,
            PureHorzBox::FixedEmpty { width } => width.0,
            _ => 0.0,
        })
        .sum()
}
