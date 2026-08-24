//! `PureHorzBox` -> paragraph [`Piece`]s.
//!
//! The mapping is the Markdown backend's, with the four places LaTeX can say
//! more taken:
//!
//! | box | Markdown | here |
//! |--|--|--|
//! | `Math` | characters in reading order | real `$…$`, via [`rustyfi_html::latex`] |
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
use rustyfi_html::recover;

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
        // character that follows (`rustyfi_html::recover::wants_space`), which is
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
            } else if width.0 >= recover::HSKIP_MIN_PT {
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
            if !recover::pre_break_carries_text(pre_break) {
                ctx.note_glue(recover::glue_width(pre_break));
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

        // Math, as real math. See `rustyfi_html::latex`.
        PureHorzBox::Math { glyphs, rules, .. } => {
            let body = rustyfi_html::latex::math_latex(glyphs, rules);
            // A math box also carries the paths a font cannot draw, and those
            // paths carry any `draw-text` the construction built — which is
            // how a BIG OPERATOR arrives. `rustyfi_html::latex` reads the bars out of
            // them and ignores the rest, so the nested text is emitted here,
            // BEFORE the formula: a `draw-text` operator sits at the box's
            // own origin, and putting `\sum` after its limits reads as
            // `ₐᵇ∑`.
            emit_nested_text(para, rules, ctx);
            if !body.is_empty() {
                ctx.open_opaque(para);
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
            if elems.iter().all(recover::is_pure_text) {
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
            // surrounding context — but into its own writer, and inside
            // `Ctx::isolated` so the note's last character does not decide
            // the spacing of the word after the reference.
            let body = ctx.isolated(|| {
                // Park the block queue too: without this a table the
                // paragraph queued before reaching this marker would be
                // emitted INSIDE the note's body.
                let parked = std::mem::take(&mut *ctx.pending_blocks.borrow_mut());
                let body = super::block::render_block(block, ctx);
                *ctx.pending_blocks.borrow_mut() = parked;
                body
            });
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
    let mono = ctx.mono_files.is_monospace(ctx.fonts, Some(info.font));
    ctx.resolve_glue(para, text.chars().next());
    ctx.last_char.set(text.chars().next_back());
    ctx.mono_run.set(mono);
    // Guarded on the flag, not just on the scan: this is a per-RUN question
    // whose answer never goes back to `false`, and the box stream emits one
    // run per CJK character.
    if !ctx.uses_cjk.get() && text.chars().any(recover::is_cjk) {
        ctx.mark_cjk();
    }
    if mono && ctx.mono_advance.get().is_none() {
        ctx.mono_advance.set(recover::mono_advance(text, width));
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
    let Some(((lo_x, lo_y), (hi_x, hi_y))) = recover::ink_bbox(elems) else {
        return;
    };
    let (ink_w, ink_h) = (hi_x.0 - lo_x.0, hi_y.0 - lo_y.0);
    if ink_w < recover::GRAPHIC_MIN_PT || ink_h < recover::GRAPHIC_MIN_PT {
        // Not a figure: a hairline rule, a leader dot, a piece of
        // underlining. Dropped rather than marked — see `GRAPHIC_MIN_PT`.
        // Its nested text, if any, still flows: an underlined WORD is a
        // `draw-text` over a rule, and the word is the point of it.
        emit_nested_text(para, elems, ctx);
        return;
    }
    ctx.open_opaque(para);
    // Rendered into its own paragraph buffer so a label carries the same
    // escaping, emphasis and math handling as body text — and inside
    // `Ctx::isolated`, since a label's last character must not decide the
    // spacing after the drawing.
    let label = |contents: &[(Length, PureHorzBox)]| -> String {
        ctx.isolated(|| {
            let mut inner = Para {
                open: true,
                ..Para::default()
            };
            for (_, cbx) in contents {
                emit_inline(&mut inner, cbx, ctx);
            }
            inner.render_inline()
        })
    };
    let scale = super::tikz::fit_scale(ink_w, ink_h, ctx.text_area.0, ctx.text_area.1);
    ctx.mark_tikz();
    // `plain` stays a named hole: it feeds the verbatim side of the
    // paragraph, which is what a content measurement reads, and where a
    // picture would be worse than useless.
    let tex = super::tikz::graphics_block(elems, scale, &label)
        .unwrap_or_else(|| super::tikz::placeholder(width, height, "[graphic]"));
    para.push_markup(tex, "[graphic]");
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


