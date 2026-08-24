//! `PureHorzBox` -> paragraph [`Piece`]s.
//!
//! The mapping is the Markdown backend's, with the four places LaTeX can say
//! more taken:
//!
//! | box | Markdown | here |
//! |--|--|--|
//! | `Math` | characters in reading order | real `$…$`, via [`crate::latex`] |
//! | `Graphics` | an `<svg>` a sanitizing renderer strips | a `tikzpicture` |
//! | a `\ref` (`AnnotAction::GotoName`) | plain text, no anchor scheme exists | `\hyperlink` |
//! | a `draw-text` label | flowed after its drawing | a `\node` at its own point |
//!
//! and one where it can say less: an `Image` is a sized placeholder rather
//! than the picture, because `\includegraphics` reads a FILE and a compile
//! produces one output path. See `tikz.rs`'s `placeholder`.

use rustyfi_backend::{
    AnnotAction, DecoId, GraphicsElem, HorzStringInfo, InlineMarkKind, Length, PureHorzBox,
};

use super::para::{LinkTarget, Para, Piece};
use super::Ctx;

/// Append `bx`'s LaTeX rendering to the paragraph being built.
pub(super) fn emit_inline(para: &mut Para, bx: &PureHorzBox, ctx: &Ctx) {
    match bx {
        // Handled FIRST, unconditionally of the bullet fence below: a
        // `BulletEnd` reached while suppressed must still clear the counter,
        // and an emphasis marker must still keep its stack balanced.
        PureHorzBox::InlineMark(kind) => match kind {
            InlineMarkKind::EmphStart { strong } => {
                // `\textbf` and `\emph` rather than `\textit`: `\emph` nests
                // correctly (an emphasis inside an emphasis goes back to
                // upright), which is what the document meant by emphasising
                // something already emphasised.
                para.pieces.push(Piece::Open(if *strong {
                    "\\textbf{"
                } else {
                    "\\emph{"
                }));
            }
            InlineMarkKind::EmphEnd => para.pieces.push(Piece::Close),
            // The drawn bullet glyph between this fence is dropped: the
            // `\item` the list writes replaces it.
            InlineMarkKind::BulletStart => ctx.bullet_suppress.set(ctx.bullet_suppress.get() + 1),
            InlineMarkKind::BulletEnd => ctx
                .bullet_suppress
                .set(ctx.bullet_suppress.get().saturating_sub(1)),
            // Everything after this on the line is the LINE BREAKER's hyphen,
            // not the author's. `block.rs` undoes it when it rejoins the
            // lines; all this arm does is say so.
            InlineMarkKind::BreakHyphen => ctx.break_hyphen.set(true),
        },

        // `inline-frame-breakable` splices its contents between a marker PAIR
        // rather than nesting them, so a link built this way — which is how
        // `annot.satyh` writes every `\href` and `\ref` — has to be opened
        // and closed positionally. Ahead of the bullet guard for the same
        // reason as `InlineMark`.
        PureHorzBox::InlineFrameMarker { id, end, .. } => {
            if *end {
                if ctx.iframe_stack.borrow_mut().pop().unwrap_or(false) {
                    para.pieces.push(Piece::Close);
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
        // document's alignment, which this backend does not reproduce (LaTeX
        // justifies for itself, at a measure the reader chose), so it is
        // nothing but a break opportunity here.
        PureHorzBox::OuterFil => ctx.note_glue(0.0),
        // `inline-skip`: an explicit, deliberately-sized gap. Always a `Gap`
        // inside a code block, where it is the indentation; in prose reduced
        // to the single space it is worth.
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
        // offered to split stays whole. Nothing is written: LaTeX hyphenates
        // for itself at a measure the reader chose, and a `\-` here would
        // fossilize the port's own dictionary while SUPPRESSING every other
        // break point in the word (a single `\-` makes TeX use only the
        // explicit ones).
        PureHorzBox::Discretionary { pre_break, .. } => {
            if !pre_break_carries_text(pre_break) {
                ctx.note_glue(glue_width(pre_break));
            }
        }

        // An unbreakable inline frame. Its decoration is dropped — a frame is
        // a box with a border and this backend writes flowing text — but its
        // `DecoId` may still make it a link.
        PureHorzBox::Frame { deco, contents, .. } => {
            let opened = open_link(para, deco, ctx);
            for (_, cbx) in contents {
                emit_inline(para, cbx, ctx);
            }
            if opened {
                para.pieces.push(Piece::Close);
            }
        }

        // Math, as real math. See `crate::latex`.
        PureHorzBox::Math { glyphs, rules, .. } => {
            let body = crate::latex::math_latex(glyphs, rules);
            // A math box also carries the paths a font cannot draw, and those
            // paths carry any `draw-text` the construction built — which is
            // how a BIG OPERATOR arrives. `crate::latex` reads the bars out of
            // them and ignores the rest, so the nested text is emitted here,
            // BEFORE the formula: a `draw-text` operator sits at the box's
            // own origin, and putting `\sum` after its limits reads as
            // `ₐᵇ∑`.
            emit_nested_text(para, rules, ctx);
            if !body.is_empty() {
                ctx.open_opaque(para);
                ctx.mark_math();
                para.push_markup(format!("${body}$"), &body);
            }
        }

        // A drawing, drawn. See `tikz.rs` — this is the one place LaTeX is a
        // better target than either of the other two backends.
        PureHorzBox::Graphics {
            elems,
            width,
            height,
            depth,
            ..
        } => {
            // A graphics box whose every element is a `draw-text` draws
            // nothing: it is a positioning wrapper, and everything a reader
            // wants from it is in the nested boxes. `easytable` wraps every
            // table in exactly this shape, so this arm is also where a
            // table's rules-only twin is paired with the real one.
            if elems.iter().all(is_pure_text) {
                let depth = ctx.push_overlaid_rules(elems);
                emit_nested_text(para, elems, ctx);
                ctx.pop_overlaid_rules(depth);
            } else {
                emit_drawing(para, elems, width.0, height.0 + depth.0, ctx);
            }
        }
        // A DEFERRED lang-side callback, always resolved to a plain
        // `Graphics` well before `reflow_source` is captured — so this arm is
        // realistically unreachable. There is nothing to measure and nothing
        // to draw, so it says nothing rather than claiming a figure.
        PureHorzBox::GraphicsOuter { .. } => {}

        // A raster image: a sized, labelled placeholder. See
        // `tikz::placeholder` for why the picture itself cannot travel.
        PureHorzBox::Image {
            image,
            width,
            height,
        } => {
            ctx.open_opaque(para);
            let n = ctx.image_number(image.0);
            let label = match ctx.images.get(image.0) {
                Some(res) if res.pdf.is_some() => format!("[embedded PDF page {n}]"),
                _ => format!("[image {n}]"),
            };
            ctx.mark_tikz();
            para.push_markup(
                super::tikz::placeholder(width.0, height.0, &label),
                label.clone(),
            );
        }

        // A table is block-level: `tabular` inside a paragraph is legal but a
        // full-measure one would overflow the line, and a `verbatim` cell is
        // not expressible at all. So it is QUEUED here and emitted by
        // `block.rs` as its own block once the surrounding paragraph closes.
        // This arm is the main path, not a fallback: `easytable` reaches a
        // `tabular` through a `draw-text` inside an `inline-graphics`, so by
        // the time one is seen it is already inside inline content.
        PureHorzBox::Tabular(tab) => {
            ctx.open_opaque(para);
            if let Some(tex) = super::table::render_table(tab, ctx) {
                ctx.pending_blocks.borrow_mut().push(tex);
            }
        }

        // A real footnote. LaTeX's own `\footnote` puts it at the foot of
        // whatever page it lands on, which is where the document put it
        // before page breaking removed the pages.
        PureHorzBox::Footnote { block } => {
            ctx.open_opaque(para);
            // Rendered NOW, where the marker rides, so it sees the
            // surrounding context — but into its own writer, with the
            // inline-flow state saved across it so the note's last character
            // does not decide the spacing of the word after the reference.
            let saved = (
                ctx.pending_glue.take(),
                ctx.last_char.take(),
                ctx.mono_run.get(),
            );
            // Park the block queue: without this a table the paragraph
            // queued before reaching this marker would be emitted INSIDE the
            // note's body.
            let parked = std::mem::take(&mut *ctx.pending_blocks.borrow_mut());
            let body = super::block::render_block(block, ctx);
            *ctx.pending_blocks.borrow_mut() = parked;
            ctx.pending_glue.set(saved.0);
            ctx.last_char.set(saved.1);
            ctx.mono_run.set(saved.2);
            let body = body.trim();
            if !body.is_empty() {
                para.push_markup(format!("\\footnote{{{body}}}"), "");
            }
            // The document has ALREADY typeset its own reference marker —
            // `stdjabook`'s `\footnote` sets a raised, three-quarter-size
            // numeral immediately after this box (`footnote-scheme.satyh:79`
            // emits `add-footnote … ++ ib-num`). Left in, the text would
            // carry two marks and the note would be numbered twice.
            // `set-manual-rising` is what makes it identifiable: raised text
            // is otherwise vanishingly rare in body copy.
            ctx.drop_fn_marker.set(true);
        }

        // Handled one level up, in `block.rs` — it has to CLOSE the open
        // paragraph, which this function cannot do. Kept as an explicit inert
        // arm so a new `PureHorzBox` variant still forces a compile error
        // here rather than silently falling through.
        PureHorzBox::EmbeddedBlock { .. } => {}

        // Zero-width markers and hooks: no meaning in a reflowed format.
        PureHorzBox::HookPageBreak { .. } | PureHorzBox::FrameMarker { .. } => {}
    }
}

/// A `FixedEmpty` (`inline-skip`) at least this wide (pt) is a deliberate gap
/// worth one space; anything narrower is a KERN and renders as nothing. The
/// same two populations the other backends separate here — a paragraph indent
/// or a table cell's padding above, the `\LaTeX` logo's own kerns and a
/// table-of-contents leader's dot spacing below.
const HSKIP_MIN_PT: f64 = 2.0;

/// Smaller than this in either dimension (pt) and a drawing's INK is a rule,
/// a leader dot or a piece of underlining, not a figure.
///
/// The size measured is the INK's, not the box's, and that distinction is
/// load-bearing: `stdjabook` draws the rule under a section heading as a
/// 440pt x 1pt line inside a 440pt x 4pt box. Judged by the box it is a
/// figure, and every heading in `easytable`'s manual would grow a
/// `tikzpicture` above and below it.
const GRAPHIC_MIN_PT: f64 = 4.0;

/// One `InnerString` run.
fn emit_run(para: &mut Para, info: &HorzStringInfo, text: &str, width: f64, ctx: &Ctx) {
    if text.is_empty() {
        return;
    }
    // The footnote reference marker the document typeset for itself — see the
    // `Footnote` arm. Dropped for as long as the runs stay RAISED, not just
    // for one: `stdjabook`'s marker is `\*#it-num;`, which is two runs (the
    // symbol and the numeral) inside one `set-manual-rising` context.
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
    if text.chars().any(crate::recover::is_cjk) {
        ctx.mark_cjk();
    }
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
/// **Both kinds become real links here**, which is where this backend parts
/// company with the Markdown one. `Uri` is a `\href`, obviously. `GotoName` —
/// a `\ref` to a section — is a `\hyperlink`, and it WORKS, because the
/// destination it names is one the document registered with
/// `register-destination` and the heading that registered it carries a
/// matching `\hypertarget` (`para.rs`'s `heading`). Markdown has to drop
/// these to plain text: it has no anchor scheme at all, and a renderer
/// invents heading anchors from the heading's own words.
fn open_link(para: &mut Para, deco: &DecoId, ctx: &Ctx) -> bool {
    let target = match ctx.links.get(deco) {
        Some(AnnotAction::Uri(uri)) => LinkTarget::Uri(uri.clone()),
        Some(AnnotAction::GotoName(name)) => LinkTarget::Goto(name.clone()),
        _ => return false,
    };
    ctx.mark_hyperref();
    para.pieces.push(Piece::LinkOpen(target));
    true
}

/// A drawing that actually draws something.
fn emit_drawing(para: &mut Para, elems: &[GraphicsElem], width: f64, height: f64, ctx: &Ctx) {
    let Some(((lo_x, lo_y), (hi_x, hi_y))) = elems
        .iter()
        .filter_map(rustyfi_backend::graphics_bbox)
        .reduce(|(alo, ahi), (blo, bhi)| {
            (
                (
                    Length(alo.0 .0.min(blo.0 .0)),
                    Length(alo.1 .0.min(blo.1 .0)),
                ),
                (
                    Length(ahi.0 .0.max(bhi.0 .0)),
                    Length(ahi.1 .0.max(bhi.1 .0)),
                ),
            )
        })
    else {
        return;
    };
    if hi_x.0 - lo_x.0 < GRAPHIC_MIN_PT || hi_y.0 - lo_y.0 < GRAPHIC_MIN_PT {
        // Not a figure: a hairline rule, a leader dot, a piece of
        // underlining. Dropped rather than marked — see [`GRAPHIC_MIN_PT`].
        // Its nested text, if any, still flows: an underlined WORD is a
        // `draw-text` over a rule, and the word is the point of it.
        emit_nested_text(para, elems, ctx);
        return;
    }
    ctx.open_opaque(para);
    // Rendered into its own paragraph buffer so a label carries the same
    // escaping, emphasis and math handling as body text — and with the
    // inline-flow state saved across it, since a label's last character must
    // not decide the spacing after the drawing.
    let mut label = |contents: &[(Length, PureHorzBox)]| -> String {
        let saved = (
            ctx.pending_glue.take(),
            ctx.last_char.take(),
            ctx.mono_run.get(),
        );
        let mut inner = Para {
            open: true,
            ..Para::default()
        };
        for (_, cbx) in contents {
            emit_inline(&mut inner, cbx, ctx);
        }
        ctx.pending_glue.set(saved.0);
        ctx.last_char.set(saved.1);
        ctx.mono_run.set(saved.2);
        // As PROSE whatever its face: a node's body is restricted horizontal
        // mode, where a `verbatim` cannot go.
        Para {
            mono: false,
            has_mono: false,
            ..inner
        }
        .render(None)
        .map(|r| r.text)
        .unwrap_or_default()
    };
    let scale = super::tikz::fit_scale(
        hi_x.0 - lo_x.0,
        hi_y.0 - lo_y.0,
        ctx.text_area.0,
        ctx.text_area.1,
    );
    match super::tikz::graphics_block(elems, scale, &mut label) {
        Some(tex) => {
            ctx.mark_tikz();
            // `plain` stays a named hole: it feeds the verbatim side of the
            // paragraph, which is what a content measurement reads, and where
            // a picture would be worse than useless.
            para.push_markup(tex, "[graphic]");
        }
        None => {
            ctx.mark_tikz();
            para.push_markup(
                super::tikz::placeholder(width, height, "[graphic]"),
                "[graphic]",
            );
        }
    }
}

/// Whether `elem` contributes no ink of its own — a `draw-text`, or a group
/// containing only those.
fn is_pure_text(elem: &GraphicsElem) -> bool {
    match elem {
        GraphicsElem::Text { .. } => true,
        GraphicsElem::Group(inner) | GraphicsElem::Clip(_, inner) => inner.iter().all(is_pure_text),
        _ => false,
    }
}

/// Emit the boxes nested inside `draw-text` elements, in document order,
/// with no drawing around them.
///
/// Used where there is no picture to place them in — a text-only wrapper, or
/// a drawing below the ink threshold. A drawing that IS emitted keeps its
/// labels as `\node`s at their own points instead ([`emit_drawing`]).
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
