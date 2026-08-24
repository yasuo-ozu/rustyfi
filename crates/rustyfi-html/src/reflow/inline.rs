//! `PureHorzBox` → inline HTML ("Inline level"), appending into an
//! already-open paragraph's (or inline frame's) text buffer: the browser
//! lays every span out in normal inline flow, and no run, paragraph, frame
//! or table here carries an x/y of its own.
//!
//! The ONE construct that does is `draw-text` at a point other than its own
//! box's origin — `\overset`/`\underset` and the big operators that carry
//! limits, whose whole content is "put this row above that one", which flow
//! cannot say. [`emit_placed_text`] places those, INSIDE the
//! relatively-positioned math/graphics wrapper they belong to and never
//! against the page; [`all_nested_text_at_anchor`] is the line between that
//! case and the wrapper-shaped one, which still flows.
//!
//! Slice 1 renders `InnerString` (styled + escaped text), the three glue
//! variants (collapsed to a plain space — the browser re-breaks, so the
//! exact stretch/shrink amounts have no reflow meaning), `Discretionary`
//! (a soft hyphen), and `Frame` (a real inline `<span>`, contents recursed).
//!
//! Slice 2 (design doc §4/§6 "S2") replaces the `Math`/`Graphics` PLACEHOLDER
//! `<span>`s with real, self-contained inline `<svg>` — see
//! [`emit_math_svg`]/[`emit_graphics_box`] — and gives `Frame` real `<a
//! href>`/`id=` treatment when its `DecoId` matches an observed link/
//! destination (`Ctx::links`/`Ctx::dests`, sourced from `DocumentValue::
//! reflow_links`/`reflow_dests`). A frame that DRAWS gets its drawing too,
//! as a background on the same wrapper — see [`wrapper_tags`] and
//! `structure::inline_frame_decoration`, which is where the placement of a
//! decoration over text the reader is about to re-break is argued.
//! `GraphicsOuter`/`Image`/`Footnote` remain
//! inert PLACEHOLDER `<span>`s (`GraphicsOuter` in particular is a
//! lang-side-only DEFERRED callback — `resolve_outer_graphics_in_contents`,
//! `rustyfi-lang/src/primitives.rs:3917`, always resolves it to a plain
//! `Graphics` box during `line-break`, well before `reflow_source` is
//! captured, so this arm is realistically unreachable; kept as an honest
//! placeholder rather than assumed-dead code).
//! `HookPageBreak`/`FrameMarker` render nothing (no reflow meaning, same as
//! the PDF writer's own wildcard arm).
//!
//! Slice 3 (design doc §6 "S3", `structure.rs`'s doc comment) replaces the
//! `Tabular` PLACEHOLDER `<span>` with a real `<table>` — see this module's
//! own `Tabular` arm below for why it delegates to `structure::render_table`
//! (`block.rs` handles the common top-level case directly, since a `<table>`
//! is block-level and needs to flush the surrounding paragraph first; this
//! arm is only the fallback for a `Tabular` nested inside inline content).
//!
//! Slice 4 ("Inline level") handles the new `InlineMark` box:
//! `EmphStart`/`EmphEnd` open/close a real `<em>`/`<strong>` (via
//! `Ctx::emph_stack`, since `EmphEnd` alone doesn't say which tag to close —
//! see that field's doc comment), and `BulletStart`/`BulletEnd` fence a
//! drawn bullet/number glyph run so it renders NOTHING
//! (`Ctx::bullet_suppress`) — the real marker comes from the `<ul>`/`<ol>`
//! `block.rs` now emits instead.

use std::fmt::Write as _;

use rustyfi_backend::{
    AnnotAction, Color, GraphicsElem, HorzStringInfo, InlineMarkKind, MathGlyph, PureHorzBox,
    VertBox,
};

use super::{Ctx, GlyphOutline};
use crate::image;

/// Append `bx`'s reflow rendering to `out`. Never touches `out`'s
/// surrounding whitespace/paragraph bookkeeping — that is the caller's
/// (`block.rs`'s) job.
pub(crate) fn emit_inline(out: &mut String, bx: &PureHorzBox, ctx: &Ctx) {
    match bx {
        // S4: handled FIRST, unconditionally of `ctx.bullet_suppress` below
        // — a `BulletEnd` reached WHILE suppressed must still clear the
        // counter, and an `EmphStart`/`EmphEnd` reached while suppressed
        // (should not happen — `itemize.satyh` never nests emphasis inside
        // its own bullet fence — but stays correct regardless) must still
        // keep the tag stack balanced.
        PureHorzBox::InlineMark(kind) => match kind {
            InlineMarkKind::EmphStart { strong } => {
                close_run(out, ctx);
                ctx.emph_stack.borrow_mut().push(*strong);
                out.push_str(if *strong { "<strong>" } else { "<em>" });
            }
            InlineMarkKind::EmphEnd => {
                close_run(out, ctx);
                // An unmatched `EmphEnd` (should not happen) closes `</em>`
                // rather than panicking.
                let strong = ctx.emph_stack.borrow_mut().pop().unwrap_or(false);
                out.push_str(if strong { "</strong>" } else { "</em>" });
            }
            InlineMarkKind::BulletStart => {
                *ctx.bullet_suppress.borrow_mut() += 1;
            }
            InlineMarkKind::BulletEnd => {
                let mut n = ctx.bullet_suppress.borrow_mut();
                *n = n.saturating_sub(1);
            }
            // Everything after this on the line is the LINE BREAKER's hyphen,
            // not the author's. `block.rs` undoes it when it rejoins the
            // lines; all this arm does is say so.
            InlineMarkKind::BreakHyphen => ctx.break_hyphen.set(true),
        },

        // `inline-frame-breakable` does NOT build a `Frame`: it splices its
        // contents between a marker PAIR so the frame can split across lines,
        // which means the wrapper has to be opened and closed positionally
        // rather than around a recursion. Same lookup as the `Frame` arm
        // below, keyed on the MARKER's own `DecoId`. Without this arm every
        // `\href`/`\ref` that goes through the breakable frame — which is
        // how `annot.satyh` writes them — renders its text unwrapped, with
        // no error anywhere.
        //
        // Handled here, ahead of the `bullet_suppress` guard below, for the
        // same reason `InlineMark` is: a marker skipped while suppressed
        // would leave the stack unbalanced and mis-close a later wrapper. A
        // link inside a bullet fence should not happen, but the tags stay
        // balanced regardless — while suppressed the stack is still
        // maintained and only the emission is dropped.
        PureHorzBox::InlineFrameMarker { id, end, .. } => {
            let suppressed = *ctx.bullet_suppress.borrow() > 0;
            close_run(out, ctx);
            if *end {
                let close = ctx
                    .iframe_stack
                    .borrow_mut()
                    .pop()
                    .map_or("</span>", |(_, close)| close);
                if !suppressed {
                    out.push_str(close);
                }
            } else {
                let (open, reopen, close) = wrapper_tags(id, ctx);
                if !suppressed {
                    out.push_str(&open);
                }
                ctx.iframe_stack.borrow_mut().push((reopen, close));
            }
        }

        // S4: while a `BulletStart`/`BulletEnd` fence is open, every OTHER
        // box (the bullet's `Graphics` circle / the enumerate index's
        // `InnerString` digit, and anything else that happened to ride
        // along) renders nothing — the real marker comes from the `<ul>`/
        // `<ol>` `block.rs` now emits. Matched here, ahead of every
        // concrete arm below, via a guarded wildcard (still fully
        // exhaustive together with the explicit arms that follow it — see
        // `rustc`'s own exhaustiveness check, which accepts this).
        _ if *ctx.bullet_suppress.borrow() > 0 => {}

        PureHorzBox::InnerString { info, text, .. } => emit_run(out, info, text, ctx),

        // Glue does NOT become a space here — it is RECORDED, and judged
        // once the following character is known. The box stream puts glue
        // between every pair of CJK characters and inside every hyphenatable
        // Latin word, so "glue means space" (what this arm used to do)
        // rendered Japanese as `研 究 計 画` and `\LaTeX` as `L AT EX`. See
        // `text.rs`'s doc comment for the whole argument and `wants_space`
        // for the rule.
        PureHorzBox::OuterEmpty { natural, .. } => ctx.note_glue(natural.0),
        // `inline-fil`: infinite stretch, no natural width. Never a space;
        // `block.rs` reads its POSITION instead, as the alignment signal it
        // is (leading + trailing fil = centred, leading only = flush right).
        PureHorzBox::OuterFil => ctx.note_glue(0.0),
        // `inline-skip`: an explicit, deliberately-sized, non-breakable gap.
        // Above a visible threshold it keeps its width as an inline-block
        // strut (intrinsic sizing, not positioning — the same licence math
        // and graphics have); below it, it is a kern and goes through the
        // ordinary glue rule, which drops it.
        PureHorzBox::FixedEmpty { width } => {
            if width.0 >= HSKIP_MIN_PT {
                ctx.resolve_glue(out, None);
                close_run(out, ctx);
                let _ = write!(
                    out,
                    "<span class=\"hskip\" style=\"width:{}pt;\"></span>",
                    width.0
                );
            } else {
                // A kern: recorded as zero-width glue, i.e. as nothing. A
                // sub-2pt `inline-skip` is a micro-adjustment, never a word
                // space — word spaces arrive as `OuterEmpty`.
                ctx.note_glue(0.0);
            }
        }

        // A break point that may or may not be taken. When it carries text
        // in `pre_break` it is a real hyphenation point (the pattern
        // dictionary's, `Latex&shy;Cmds`) and a soft hyphen is exactly
        // right: the browser re-breaks and shows the hyphen only if it
        // breaks there. When `pre_break` is bare glue it is a UAX#14 chunk
        // boundary — a soft hyphen there would invite the browser to
        // hyphenate `Con-tributors` at a point the dictionary never
        // sanctioned, and between two CJK characters it would be nonsense —
        // so it goes through the ordinary glue rule instead.
        PureHorzBox::Discretionary { pre_break, .. } => {
            if pre_break_carries_text(pre_break) {
                out.push_str("&shy;");
            } else {
                ctx.note_glue(glue_width(pre_break));
            }
        }

        // A real inline frame: no atomic-width fitting to preserve (that
        // was only ever needed for the eager line-breaker) — just recurse
        // its contents into one wrapper for CSS-hook purposes.
        //
        // S2 (design doc §4 "Links/metadata"): if THIS frame's `DecoId`
        // matches an observed `register-link-to-uri`/`-to-location` call
        // (`ctx.links`, `annot.satyh`'s `\href`), wrap the contents in a
        // real `<a href>` instead of a plain `<span>` — `Uri` maps to the
        // literal URL, `GotoName` to an in-document `#anchor` (the matching
        // destination is placed by `block.rs`'s `FrameStart`/`ctx.dests`
        // lookup, or by this SAME arm's `dest` fallback below when the
        // named-destination frame happens to be inline rather than block).
        // Falls back to `ctx.dests` (a `register-location-frame` used
        // inline rather than as a block frame) for a plain `id=` anchor
        // when there's no link action, then to the Slice-1 inert `<span>`.
        PureHorzBox::Frame { deco, contents, .. } => {
            let (open, _, close) = wrapper_tags(deco, ctx);
            close_run(out, ctx);
            out.push_str(&open);
            for (_, cbx) in contents {
                emit_inline(out, cbx, ctx);
            }
            close_run(out, ctx);
            out.push_str(close);
        }

        // Math is flattened to positioned glyphs at eval time (design doc
        // §4) — no fraction/sub/sup structure survives to render as MathML,
        // so (Slice 2, design doc §4's "the honest option") this renders
        // each glyph as positioned text and each rule (fraction bar/radical)
        // as an SVG path, bundled into ONE self-contained,
        // intrinsically-sized inline `<svg>` (see [`emit_math_svg`]) — there
        // is no page here to position a glyph WITHIN, so the drawing carries
        // its own coordinate space.
        PureHorzBox::Math {
            width,
            height,
            depth,
            glyphs,
            rules,
        } => emit_math_svg(out, width.0, height.0, depth.0, glyphs, rules, ctx),

        // Slice 2 (design doc §6/§4 "reuse svg::emit_graphics verbatim"):
        // real inline SVG, sized to the box's own metrics — see
        // [`emit_graphics_box`].
        PureHorzBox::Graphics {
            width,
            height,
            depth,
            elems,
            origin_independent: _,
        } => emit_graphics_box(out, width.0, height.0, depth.0, elems, ctx),

        // `GraphicsOuter` is a DEFERRED lang-side callback (`GraphicsFnId`,
        // resolved only by `resolve_outer_graphics_in_contents` at
        // `line-break` time, well before `reflow_source` is captured — see
        // this module's doc comment); the backend has no way to run it, same
        // limitation the PDF writer has (its own `emit_box` has no arm
        // for this variant at all, silently matching its wildcard). Kept as
        // an honest placeholder rather than silently dropped.
        PureHorzBox::GraphicsOuter { .. } => {
            open_opaque(out, ctx);
            out.push_str(
                "<span class=\"gfx-placeholder\" title=\"unresolved inline-graphics-outer (lang-side callback)\"></span>",
            );
        }
        // A real `<img>` with a self-contained data URI, sized in the
        // document's own points but capped at the column width so a figure
        // wider than a narrow viewport shrinks instead of overflowing
        // (`css.rs`'s `img.img` rule), with the bytes inlined by
        // `crate::image::data_uri`.
        PureHorzBox::Image {
            width,
            height,
            image,
        } => {
            open_opaque(out, ctx);
            match ctx.images.get(image.0) {
                // `load-pdf-image`: an imported PDF page carries no raster
                // samples at all (`ImageResource::pdf`), and rasterizing one
                // is out of scope for an HTML writer, so this keeps an
                // honestly-labelled box at the right size rather than
                // emitting a degenerate 0x0 `<img>`.
                Some(res) if res.pdf.is_some() => {
                    let _ = write!(
                        out,
                        "<span class=\"pdf-image\" style=\"width:{}pt; height:{}pt;\" \
                         title=\"embedded PDF page (not rasterized)\"></span>",
                        width.0, height.0,
                    );
                }
                // Placed more than once: its bytes go into the stylesheet
                // ONCE (`css.rs`'s `shared_image_rules`) and every placement
                // is a sized box referencing that rule. A data URI is
                // typically hundreds of kilobytes, so repeating it per
                // placement is the difference between a 13 MB file and a
                // 1.9 MB one for a manual that shows the same two figures
                // seventeen times. The element is `aria-hidden` rather than
                // an `<img>`, which costs nothing here: SATySFi carries no
                // alt text, so the `<img>` below is `alt=""` — decorative —
                // and the two are equivalent to a screen reader.
                Some(_) if ctx.image_sharing(image.0).1 => {
                    let canon = ctx.image_sharing(image.0).0;
                    let mut shared = ctx.shared_images.borrow_mut();
                    if !shared.contains(&canon) {
                        shared.push(canon);
                    }
                    drop(shared);
                    let _ = write!(
                        out,
                        "<span class=\"img shared-img-{canon}\" style=\"width:{}pt; height:{}pt;\" \
                         aria-hidden=\"true\"></span>",
                        width.0, height.0,
                    );
                }
                Some(res) => {
                    let _ = write!(
                        out,
                        "<img class=\"img\" src=\"{}\" style=\"width:{}pt; height:{}pt;\" alt=\"\">",
                        image::data_uri(res),
                        width.0,
                        height.0,
                    );
                }
                // Out-of-range `ImageId` — should not happen; the PDF writer
                // skips rather than panics, and so does this.
                None => {}
            }
        }
        // S3 ("S3", `structure.rs`'s "Tables — genuinely recoverable"): a
        // real `<table>`. `block.rs`'s own `VertBox::Line` walk already
        // special- cases the common top-level case (flushing the open
        // paragraph first, since a `<table>` is block-level); THIS arm is
        // the fallback for a `Tabular` nested inside inline content this
        // module recurses into on its own (a `Frame`'s `contents`, or a
        // table cell that itself contains a nested `Tabular`) — no
        // surrounding paragraph to flush here, so no `extra_attrs` margin.
        PureHorzBox::Tabular(tab) => {
            open_opaque(out, ctx);
            super::structure::render_table(out, tab, "", ctx)
        }
        // A footnote has nowhere to be collected TO in a continuous
        // document — there is no page foot any more — so it becomes a
        // numbered reference here and its body is queued for `block.rs`'s
        // `flush_para` to place as an `<aside>` immediately after the
        // paragraph that referenced it. See the `reflow` module's doc
        // comment for why "just after the paragraph" was chosen over
        // "collected at the end".
        PureHorzBox::Footnote { block } => {
            open_opaque(out, ctx);
            let n = ctx.footnote_seq.get() + 1;
            ctx.footnote_seq.set(n);
            // Render the body NOW, at the point the marker rides, so it sees
            // the surrounding context — but into its own buffer, and with
            // the inline-flow state saved/restored around it, since the
            // footnote's own last character must not decide the spacing of
            // the word after the reference in the main text.
            let saved = (ctx.pending_glue.take(), ctx.last_char.take());
            // Park the queue too: the nested walk drains whatever it finds
            // there when it finishes, and an earlier sibling footnote from
            // this same paragraph must not end up nested inside this one.
            let parked = std::mem::take(&mut *ctx.footnotes.borrow_mut());
            let mut body = String::new();
            super::block::walk_vboxes(&mut body, block, ctx);
            ctx.pending_glue.set(saved.0);
            ctx.last_char.set(saved.1);
            let mut queue = ctx.footnotes.borrow_mut();
            *queue = parked;
            queue.push((n, body));
            drop(queue);
            // An EMPTY anchor, not a visible marker.
            //
            // The document has already typeset its own reference marker —
            // `stdjabook`'s `\footnote` sets a superscript `*1` in the text
            // immediately after this box — so emitting a numbered `<sup>`
            // here put `1*1` on the page, and the note's body then repeated
            // the document's number a third time. What is missing is not a
            // marker but a link TARGET, which is all this is; the
            // `<aside>`'s back-link points at it. Forward navigation is not
            // worth duplicating the marker for, since the note lands a
            // couple of lines below rather than pages away.
            let _ = write!(out, "<span class=\"fnref\" id=\"fnref-{n}\"></span>");
        }

        // An `EmbeddedBlock` reached from INSIDE inline content — see
        // [`emit_embedded_block`] for what it is doing there and why this arm
        // was empty (and wrong) until it was measured.
        PureHorzBox::EmbeddedBlock { width, block, .. } => {
            emit_embedded_block(out, width.0, block, ctx)
        }

        // No reflow meaning (zero-width markers/hooks; matches the PDF
        // writer's own wildcard treatment of these two).
        PureHorzBox::HookPageBreak { .. } | PureHorzBox::FrameMarker { .. } => {}
    }
}

/// An `embed-block-top`/`-bottom` box reached from INSIDE inline content, as
/// an intrinsically-sized inline-block holding its own text.
///
/// **This arm used to be empty, and the comment saying so claimed it was
/// "unreachable in practice". That was false**, and it silently deleted
/// content. `block.rs`'s own per-`Line` loop does handle the common case —
/// an `EmbeddedBlock` sitting directly on a line, where it can flush the open
/// paragraph and emit a real `<div class="embed">` — but that is not the only
/// way one arrives. A package that composes a block into a DRAWING reaches
/// this function instead, through `svg::emit_graphics`' `draw-text` callback:
/// `figbox`'s `frame`/`bgcolor`/`shift`/`rotate`/`scale`/`graffiti` each wrap
/// their argument in `inline-graphics (fun (x, y) -> [draw-text (x, y) ib])`,
/// so `textbox-with-width 100pt {…} |> frame 1pt Color.black` — a framed
/// paragraph, the single most ordinary thing that package does — put an
/// `EmbeddedBlock` here and the text vanished, leaving an empty rectangle the
/// right size. A `Frame`'s `contents` and a nested table cell reach it the
/// same way.
///
/// **Why it renders as an inline-block of INLINE content rather than reusing
/// `block::walk_vboxes`.** Everything this function writes ends up inside the
/// enclosing `<p class="para">`, and an HTML parser closes an open `<p>` at
/// the first block-level start tag it meets — so emitting the `<p>`s that
/// `walk_vboxes` produces would not nest them, it would TERMINATE the
/// paragraph they are inside and spill the rest of it out. So each of the
/// block's lines is emitted as inline content, and only a real paragraph
/// boundary (a `Skip`/`ParagTop`/`FramePad`, or a frame edge) becomes a
/// `<br>`; a line-to-line boundary inside one paragraph becomes ordinary
/// glue, exactly as `block.rs` treats it, because the browser is going to
/// re-break the text itself.
///
/// **The width is the document's and is kept**, since that is the whole
/// content of the construction — `textbox-with-width 100pt` means "break this
/// paragraph at 100pt". `max-width:100%` keeps it from overflowing a narrow
/// reader. The baseline needs no declaration: an inline-block's own baseline
/// is that of its LAST line box, which is exactly `embed-block-bottom`'s
/// `anchor_last`. `embed-block-top` anchors on its FIRST line instead and CSS
/// has no spelling for that; it is left on the same rule, which differs only
/// for a multi-line top-anchored block and is a great deal closer than the
/// nothing this emitted before.
fn emit_embedded_block(out: &mut String, width: f64, block: &[VertBox], ctx: &Ctx) {
    open_opaque(out, ctx);
    let _ = write!(
        out,
        "<span class=\"embed-inline\" style=\"width:{width}pt;\">"
    );
    let mut wrote_line = false;
    let mut want_break = false;
    for vb in block {
        match vb {
            VertBox::Line { contents, .. } => {
                if want_break && wrote_line {
                    close_run(out, ctx);
                    out.push_str("<br>");
                    ctx.reset_flow();
                }
                want_break = false;
                for (_, bx) in contents {
                    emit_inline(out, bx, ctx);
                }
                wrote_line = true;
                // Between two lines of ONE paragraph: the browser re-breaks,
                // so the port's break becomes glue — or nothing at all where
                // the breaker hyphenated a word. `block.rs`'s own line loop
                // makes the identical call.
                super::block::rejoin_lines(out, ctx);
            }
            // A real paragraph boundary inside the embedded block.
            VertBox::Skip(_)
            | VertBox::ParagTop(_)
            | VertBox::FramePad(_)
            | VertBox::ClearPage
            | VertBox::FrameStart(_)
            | VertBox::FrameEnd(_) => want_break = true,
            // Markers with no inline rendering — a list inside an embedded
            // block inside a drawing keeps its item text and loses only its
            // bullet, which is the same trade `bullet_suppress` already makes.
            _ => {}
        }
    }
    close_run(out, ctx);
    ctx.reset_flow();
    out.push_str("</span>");
}

/// A `FixedEmpty` (`inline-skip`) at least this wide (pt) survives as a real
/// sized strut; anything narrower is a KERN and renders as nothing at all.
///
/// Two points separates the two populations cleanly in the corpus. Above it
/// sit the deliberate gaps — a paragraph indent (one em, 10.56pt), a table
/// cell's padding (6pt), the space after a section number (10pt) — which a
/// reader would miss. Below it sit micro-adjustments: the `\LaTeX` logo's
/// four kerns, and the 1pt gaps between the dots of a table-of-contents
/// leader, of which `enumitem`'s manual has some hundreds. Rendering those
/// as struts broke a leader into one `<span>` per dot; rendering them as
/// spaces would be wider than the kern they stand for. Nothing is right for
/// both.
const HSKIP_MIN_PT: f64 = 2.0;

/// One `InnerString` run.
///
/// The text is written as TEXT: a run set in the document's body style (see
/// `text::BodyStyle`, which `css.rs` puts on `body`) gets no element around
/// it at all, so a paragraph of ordinary prose serialises as ordinary prose
/// rather than as one `<span style="font-size:10.56pt">` per syllable. Only
/// the properties that genuinely differ from the body style are emitted, and
/// the size as an `em` RATIO rather than an absolute point size, so the
/// whole document rescales from the single value on `body`.
///
/// Still no `left`/`top`/`position` — this is flowing content, not a placed
/// box; `vertical-align` (not `position`) handles a non-zero `rising`, since
/// it needs no positioned ancestor and composes with the inline flow.
fn emit_run(out: &mut String, info: &HorzStringInfo, text: &str, ctx: &Ctx) {
    if text.is_empty() {
        return;
    }
    ctx.resolve_glue(out, text.chars().next());
    ctx.last_char.set(text.chars().next_back());
    ctx.mono_run.set(ctx.is_monospace(Some(info.font)));

    let mut style = String::new();
    if !ctx.body.matches(info.font, info.size.0) {
        // A run in the body's OWN face names no family of its own — the
        // `body` rule already names it.
        if Some(info.font) != ctx.body.font {
            if let Some(stack) = ctx.font_family_for(info.font) {
                let _ = write!(style, "font-family:{stack};");
            }
        }
        if (info.size.0 - ctx.body.size).abs() >= 0.005 {
            let _ = write!(style, "font-size:{:.4}em;", info.size.0 / ctx.body.size);
        }
    }
    // Non-black only, mirroring the PDF writer's own fill-color guard, so a
    // plain black run stays unwrapped.
    if info.color != Color::Gray(0.0) {
        let _ = write!(style, "color:{};", crate::svg::css_color(info.color));
    }
    if info.rising.0 != 0.0 {
        let _ = write!(style, "vertical-align:{}pt;", info.rising.0);
    }

    let escaped = crate::escape_html(text);
    if style.is_empty() {
        close_run(out, ctx);
        out.push_str(&escaped);
        return;
    }
    // Same style as the span already open: keep writing into it. This is
    // what turns the box stream's one-`InnerString`-per-CJK-character (and
    // one-per-hyphenation-chunk) into a single span of readable text.
    let mut open = ctx.open_run.borrow_mut();
    match open.as_deref() {
        Some(current) if current == style => {}
        _ => {
            if open.is_some() {
                out.push_str("</span>");
            }
            let _ = write!(out, "<span class=\"run\" style=\"{style}\">");
            *open = Some(style);
        }
    }
    out.push_str(&escaped);
}

/// Close the `<span class="run">` left open by [`emit_run`], if any. Called
/// by everything that writes something which is not part of the run's text —
/// a wrapper tag, a strut, an `<svg>`, an `<img>`, the end of a paragraph or
/// a table cell. A space and a soft hyphen do NOT call it: they carry no
/// style and belong inside the word.
pub(crate) fn close_run(out: &mut String, ctx: &Ctx) {
    if ctx.open_run.borrow_mut().take().is_some() {
        out.push_str("</span>");
    }
}

/// The opening and closing tags a decorated inline region gets, from its
/// `DecoId` alone. Shared by the `Frame` arm (a wrapper around a recursion)
/// and the `InlineFrameMarker` arm (the same wrapper, opened and closed
/// positionally because `inline-frame-breakable` splices rather than nests)
/// so the two can never disagree about what a given `DecoId` means:
///
/// - an observed `register-link-to-uri`/`-to-location`
///   (`Ctx::links`, `annot.satyh`'s `\href`) → a real `<a href>`, `Uri` to
///   the literal URL and `GotoName` to an in-document `#anchor`;
/// - an observed `register-destination` (`Ctx::dests`,
///   `register-location-frame`) → a plain `<span>` carrying the `id=` that
///   anchor lands on;
/// - neither → an inert `<span class="iframe">`, kept as a CSS hook.
///
/// Independently of all three, a frame that DREW something
/// (`Ctx::frame_decos`, an inline entry — `railway`'s `\uwave`) also carries
/// the `ideco ideco-N` classes that paint it; see [`inline_deco_classes`].
/// Independently, because whether a region is a link and whether it is
/// decorated are unrelated: `\href` is a link that draws nothing, `\uwave`
/// draws and is not a link, and a decorated link is legal and gets both.
///
/// Returns `(open, reopen, close)`. `reopen` differs from `open` only for a
/// destination wrapper, whose `id=` must appear once and only once even if
/// the region has to be split across a paragraph boundary
/// (`Ctx::iframe_stack`). The decoration classes are on BOTH: a region split
/// across a paragraph boundary must keep drawing on the far side.
fn wrapper_tags(deco: &rustyfi_backend::DecoId, ctx: &Ctx) -> (String, String, &'static str) {
    let deco_cls = inline_deco_classes(deco, ctx);
    if let Some(action) = ctx.links.get(deco) {
        let href = match action {
            AnnotAction::Uri(uri) => crate::escape_html(uri),
            AnnotAction::GotoName(name) => format!("#{}", crate::escape_html(name)),
        };
        let tag = format!("<a class=\"link{deco_cls}\" href=\"{href}\">");
        (tag.clone(), tag, "</a>")
    } else if let Some(name) = ctx.dests.get(deco) {
        (
            format!(
                "<span class=\"iframe{deco_cls}\" id=\"{}\">",
                crate::escape_html(name)
            ),
            format!("<span class=\"iframe{deco_cls}\">"),
            "</span>",
        )
    } else {
        (
            format!("<span class=\"iframe{deco_cls}\">"),
            format!("<span class=\"iframe{deco_cls}\">"),
            "</span>",
        )
    }
}

/// The ` ideco ideco-N` a decorated inline region's wrapper carries, or the
/// empty string when this `DecoId` drew nothing.
///
/// Registering on Ctx rather than writing the drawing inline is the
/// `shared_images` bargain — see [`Ctx::inline_decos`], including why the
/// index is looked up by the declarations rather than by the `DecoId`.
fn inline_deco_classes(deco: &rustyfi_backend::DecoId, ctx: &Ctx) -> String {
    let Some(frame) = ctx.frame_decos.get(deco) else {
        return String::new();
    };
    let Some(rule) = super::structure::inline_frame_decoration(frame) else {
        return String::new();
    };
    let mut decos = ctx.inline_decos.borrow_mut();
    let i = match decos.iter().position(|seen| *seen == rule) {
        Some(i) => i,
        None => {
            decos.push(rule);
            decos.len() - 1
        }
    };
    format!(" ideco ideco-{i}")
}

/// Close every inline wrapper this block opened, innermost first, and leave
/// the stack in place so [`reopen_wrappers`] can restore them. Called by
/// `block.rs` when a paragraph is flushed mid-wrapper.
///
/// `base` is the stack depth its `walk_vboxes` was entered at, so only the
/// wrappers THIS block opened are closed. A nested walk — an
/// `EmbeddedBlock`, a footnote body, a table cell — runs with the enclosing
/// paragraph already flushed but its wrapper stack deliberately still
/// standing; without the base it would emit that paragraph's closing tags a
/// second time, inside the nested block.
pub(crate) fn close_open_wrappers(out: &mut String, ctx: &Ctx, base: usize) {
    close_run(out, ctx);
    for (_, close) in ctx.iframe_stack.borrow().iter().skip(base).rev() {
        out.push_str(close);
    }
}

/// Re-open, outermost first, every wrapper [`close_open_wrappers`] closed.
pub(crate) fn reopen_wrappers(out: &mut String, ctx: &Ctx, base: usize) {
    for (reopen, _) in ctx.iframe_stack.borrow().iter().skip(base) {
        out.push_str(reopen);
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

/// Write an opaque, non-textual inline element (`<svg>`, `<img>`,
/// `<table>`, a footnote reference): settle any pending glue against "no
/// following character", then forget the last character, since the next
/// glue has nothing textual on its left to be judged against.
fn open_opaque(out: &mut String, ctx: &Ctx) {
    ctx.resolve_glue(out, None);
    close_run(out, ctx);
    ctx.last_char.set(None);
}

/// Slice 2 (design doc §4 "Graphics — inline SVG, reuse `svg::emit_graphics`
/// verbatim"): wrap a graphics-bearing box's `elems` in an intrinsically
/// sized `<span>` (`position:relative; display:inline-block`, sized to the
/// box's own `width×(height+depth)` and baseline-aligned via
/// `vertical-align:-depth`) and reuse [`crate::svg::emit_graphics`] UNCHANGED
/// inside it, anchored at the wrapper's own top-left `(0, height)` — exactly
/// the design doc's "supplies its own anchor (0,0 for an inline-block
/// wrapper)". [`crate::svg::emit_graphics`]'s own `<svg>` carries
/// `position:absolute; left:0; top:0`, which is why the WRAPPER (not the
/// page) must be `position:relative`: that scopes the absolute positioning
/// to this one inline box, so it composes with normal flow instead of
/// escaping to the nearest positioned ancestor (which could be the `.doc`
/// root, or nothing at all) — this is the one place `position:absolute`
/// legitimately appears in this module's output, and it never affects
/// block-level layout (the design doc's own "inline SVG for math/graphics is
/// fine — that's intrinsic sizing, not page positioning").
///
/// `nested` (for `GraphicsElem::Text`/`draw-text`, the one arm that steps
/// outside the local coordinate frame — see `svg.rs`'s own doc comment)
/// re-enters THIS module's [`emit_inline`] rather than any page-absolute box
/// emitter, since reflow has no PAGE coordinates. It does have
/// WRAPPER-LOCAL ones, though — that is exactly what the callback's `x`/`y`
/// are — and whether they are used is [`all_nested_text_at_anchor`]'s call:
/// a run at the box's own origin stays in flow (same place, still
/// reflowable), a run anywhere else is placed by [`emit_placed_text`].
fn emit_graphics_box(
    out: &mut String,
    width: f64,
    height: f64,
    depth: f64,
    elems: &[GraphicsElem],
    ctx: &Ctx,
) {
    if elems.is_empty() {
        return;
    }
    let placed = !all_nested_text_at_anchor(elems);
    // A graphics box whose every element is a `draw-text` DRAWS nothing: the
    // `<svg>` comes out with an empty `<g>` and all the content goes to
    // `nested`. Emitting the wrapper anyway reserved the box's full size a
    // second time, on top of the content's own — `easytable` wraps each table
    // in exactly this shape, and every table in a document arrived under a
    // table-sized rectangle of blank space.
    //
    // Only when the content is at the box's own origin, though. When it is
    // placed there is no second reservation to avoid (placed content is out
    // of flow and contributes no size at all) and the wrapper is REQUIRED:
    // it is the `position:relative` box the placement is relative to.
    if !placed && elems.iter().all(is_pure_text) {
        // The overlaid halves of one table are visible together only here —
        // see `Ctx::tabular_rules`. Collect any rules-only tabular's rules
        // before emitting, and drop them again after, so the pairing can
        // never reach an unrelated table later in the document.
        let pushed = collect_overlaid_rules(elems, ctx);
        let mut nested = String::new();
        emit_text_only(&mut nested, elems, ctx);
        ctx.tabular_rules.borrow_mut().truncate(pushed);
        out.push_str(&nested);
        return;
    }
    open_opaque(out, ctx);
    let total_h = height + depth;
    // Built before the wrapper is opened, because whether there is nested
    // flow content decides how the wrapper states its size — see
    // [`wrapper_size`].
    let mut drawing = String::new();
    let mut nested = String::new();
    crate::svg::emit_graphics(
        &mut drawing,
        elems,
        width,
        height,
        depth,
        0.0,
        height,
        &mut |_svg, cbx, x, y| {
            if placed {
                emit_placed_text(&mut nested, cbx, x, y, ctx)
            } else {
                emit_nested_text(&mut nested, cbx, ctx)
            }
        },
    );
    let _ = writeln!(
        out,
        "<span class=\"gfx\" style=\"position:relative; display:inline-block; \
         {} vertical-align:{}pt;\">",
        wrapper_size(width, total_h, !placed && !nested.is_empty()),
        -depth,
    );
    out.push_str(&drawing);
    out.push_str(&nested);
    out.push_str("</span>\n");
}

/// The `width`/`height` declarations for a math or graphics wrapper.
///
/// The wrapper is an `inline-block` sized to the box's own metrics, and
/// while its only children are the absolutely-positioned `<svg>`s that is
/// exactly right: it reserves the space the layout engine measured, and the
/// measurements say the SVG ink stays inside it to within a device pixel at
/// every zoom and device-scale factor.
///
/// A `draw-text` run breaks that. Its boxes cannot go inside the `<svg>`
/// (see [`emit_nested_text`]), so they end up as FLOW content in the
/// wrapper — and flow content does not make a fixed-size inline-block grow,
/// it overflows, painting over the lines above and below. Measured on
/// `latexcmds`: the `∑` of a `\sum`, which arrives here as a nested run
/// rather than as a `MathGlyph`, hung 6.1pt out of a 10.4pt box. Stating
/// the reserved size as a MINIMUM keeps it as the floor it was always meant
/// to be while letting the box contain whatever the nested content needs —
/// worst overflow across `latexcmds`' 55 wrappers goes from 6.1pt to
/// 0.4pt, which is antialiasing. The alternative, moving the nested content
/// out of the wrapper entirely, was tried and is worse: a `draw-text`
/// operator sits at the box's own origin, which is where in-flow content
/// starts anyway, so today's placement is right for the common leading-
/// operator case and moving it puts `\sum_a^b`'s scripts BEFORE its sigma.
fn wrapper_size(width: f64, total_h: f64, has_flow_content: bool) -> String {
    if has_flow_content {
        format!("min-width:{width}pt; min-height:{total_h}pt;")
    } else {
        format!("width:{width}pt; height:{total_h}pt;")
    }
}

/// Whether `elem` contributes no ink of its own — a `draw-text`, or a group
/// containing only those. `Group`/`Clip` recurse so a `unite-graphics` of
/// text runs is recognised too.
fn is_pure_text(elem: &GraphicsElem) -> bool {
    match elem {
        GraphicsElem::Text { .. } => true,
        GraphicsElem::Group(inner) | GraphicsElem::Clip(_, inner) => inner.iter().all(is_pure_text),
        _ => false,
    }
}

/// Record every rules-bearing `Tabular` in this overlay on
/// [`Ctx::tabular_rules`], returning the stack depth to truncate back to.
fn collect_overlaid_rules(elems: &[GraphicsElem], ctx: &Ctx) -> usize {
    let base = ctx.tabular_rules.borrow().len();
    walk_tabulars(elems, &mut |tab| {
        if !tab.rules.is_empty() {
            ctx.tabular_rules.borrow_mut().push((
                tab.width.0,
                tab.height.0,
                tab.rules.clone(),
            ));
        }
    });
    base
}

/// Visit every `Tabular` reachable through a text-only graphics group's
/// nested boxes.
fn walk_tabulars(elems: &[GraphicsElem], f: &mut impl FnMut(&rustyfi_backend::TabularBox)) {
    for elem in elems {
        match elem {
            GraphicsElem::Text { contents, .. } => {
                for (_, bx) in contents {
                    if let PureHorzBox::Tabular(tab) = bx {
                        f(tab);
                    }
                }
            }
            GraphicsElem::Group(inner) | GraphicsElem::Clip(_, inner) => walk_tabulars(inner, f),
            _ => {}
        }
    }
}

/// The counterpart of [`is_pure_text`]: emit those runs' contents inline, in
/// document order, with no wrapper of their own.
fn emit_text_only(out: &mut String, elems: &[GraphicsElem], ctx: &Ctx) {
    for elem in elems {
        match elem {
            GraphicsElem::Text { contents, .. } => {
                for (_, cbx) in contents {
                    emit_nested_text(out, cbx, ctx);
                }
            }
            GraphicsElem::Group(inner) | GraphicsElem::Clip(_, inner) => {
                emit_text_only(out, inner, ctx)
            }
            _ => {}
        }
    }
}

/// `draw-text`'s nested boxes, rendered for the reflow backend.
///
/// They are collected into a SIDE buffer and appended after the `</svg>`,
/// never written where `svg::emit_graphics`'s callback offers them — which
/// is inside the `<svg>`'s `<g>`. What this module emits there would be
/// `<span>`s and `<a>`s of flowing text, and an HTML
/// element inside `<svg>` outside a `<foreignObject>` is not valid markup at
/// all — the browser's parser closes the `<svg>` at the first one and the
/// rest of the drawing escapes into the document.
///
/// This is the arm for a run whose point IS the wrapper's own anchor
/// ([`all_nested_text_at_anchor`]), where "after the drawing, in flow" and
/// "at the point" are the same place — so the content keeps reflowing, which
/// is the whole premise of this backend. A run at any other point goes
/// through [`emit_placed_text`] instead.
///
/// It stays INSIDE the wrapper `<span>`, and that is what makes the
/// wrapper's own size a MINIMUM rather than a fixed reservation — see
/// [`wrapper_size`], which is where the consequence is worked out.
fn emit_nested_text(nested: &mut String, bx: &PureHorzBox, ctx: &Ctx) {
    emit_inline(nested, bx, ctx);
    close_run(nested, ctx);
}

/// The same nested box, PLACED at the wrapper-local `(x, y)` that
/// `svg::emit_graphics` computed for it — `x` its left edge, `y` its
/// BASELINE, both measured from the wrapper's top-left corner.
///
/// **Why this exists.** `draw-text` is how a package draws one piece of
/// content at a point relative to another: `\overset`/`\underset` and every
/// big-operator-with-limits in `latexcmds` are an `inline-graphics` holding
/// two or three of them, one per stacked row. Rendered in flow they came out
/// side by side in source order — `\underset{m}{Y}` as `Y m`,
/// `\normal-overset{TOP}{BASE}` as `BASETOP` — because flow has no way to
/// express "above". The coordinates were being computed and discarded.
///
/// **Why it is a second `position:absolute`, and why that is contained.**
/// The wrapper is `position:relative`, so this positions within ONE inline
/// box and never against the page; it is the same licence the wrapper's own
/// `<svg>` children already have (see [`emit_graphics_box`]), extended from
/// the drawing to the drawing's text. `css.rs`'s `.dtx` rule carries the
/// `position`, so the invariant test can still count absolute rules; only
/// the two per-box numbers are written inline.
///
/// **The strut, which is the part that is not obvious.** `top` positions a
/// box's TOP, and what we know is where its BASELINE goes, so `top` must be
/// `y` minus the box's own ascent — and the browser's idea of the ascent of
/// whatever `emit_inline` writes (font ascender + half-leading) is neither
/// the port's `height` nor knowable here. So the container does not rely on
/// it: `line-height: 0` reduces every inline box inside to a zero-height
/// contribution centred on its content area, and a zero-width inline-block
/// strut of exactly `ascent` then defines the line box's top edge on its own
/// (an empty inline-block sits with its BOTTOM on the baseline). The
/// container's top edge therefore lands exactly `ascent` above the baseline
/// whatever font the reader has, which is what makes `top = y - ascent`
/// exact. The ascent is the box's own — `pure_natural_metrics`, the same
/// per-variant table the line breaker measures with, not the enclosing run's
/// (a `\overset`'s two rows have different heights and share no ascent).
fn emit_placed_text(nested: &mut String, bx: &PureHorzBox, x: f64, y: f64, ctx: &Ctx) {
    let (_, height, _) = rustyfi_backend::pure_natural_metrics(std::iter::once(bx));
    let ascent = height.0;
    let top = y - ascent;
    let _ = write!(
        nested,
        "<span class=\"dtx\" style=\"left:{x}pt; top:{top}pt;\">\
         <span class=\"dtx-strut\" style=\"height:{ascent}pt;\"></span>",
    );
    // No whitespace between the strut and the content: they are inline-level
    // siblings, and a text node between them would render as a real space.
    emit_inline(nested, bx, ctx);
    close_run(nested, ctx);
    nested.push_str("</span>\n");
}

/// Whether every `draw-text` run in `elems` is anchored at the graphics
/// box's OWN origin — `pt == (0, 0)`, the point `svg::emit_graphics` maps to
/// the wrapper's top-left/baseline anchor.
///
/// This is the whole test for "is this box POSITIONING its contents, or
/// merely WRAPPING them". A wrapper is how a package makes an inline box out
/// of content it has already laid out — `easytable` overlays a table and its
/// rules with two `draw-text (x, y)` at the callback's own point, `figbox`
/// and `enumitem` wrap a single one — and for those, in-flow is both
/// correct and reflowable, so nothing changes. A run at any other point is
/// the document placing content deliberately, and is honoured
/// ([`emit_placed_text`]).
///
/// ALL-OR-NOTHING per graphics box, deliberately: the runs of one
/// construction share a coordinate frame, and mixing an in-flow row (whose
/// baseline the browser picks) with a placed one (whose baseline is the
/// document's) puts the two rows in different frames. `\normal-overset` is
/// exactly this shape — its base row's x-offset is zero whenever the base is
/// the wider of the two, so a per-run choice would flow the base and place
/// the accent above where the base is not.
///
/// Only the run's ANCHOR `pt` is tested, never the per-box `dx` beside it.
/// `pt` is the point the DOCUMENT chose; `dx` is where the run's own line
/// breaker put each box WITHIN the run, left to right from that point — which
/// is exactly what inline flow reproduces for free. Counting a non-zero `dx`
/// as "off-anchor" pulled `enumitem`'s bullets out of flow, since a bullet
/// label is one `draw-text pt` whose run happens to be a `hskip` followed by
/// the mark; the placement was equivalent, but it traded a reflowing bullet
/// for a pinned one and gained nothing.
///
/// `Group`/`Clip` recurse because `svg::emit_graphics`' own walker passes its
/// `tx`/`ty` through them unchanged, so a run inside one is in the same
/// frame as a run outside it.
fn all_nested_text_at_anchor(elems: &[GraphicsElem]) -> bool {
    /// `draw-text` points are built by arithmetic on the callback's own
    /// `(x, y)` — `x +' (w -' w-main) *' 0.5` is exactly zero when the two
    /// widths are equal, but only to within the rounding of the subtraction
    /// that produced them. A tolerance far below a rendered pixel keeps
    /// "the author wrote the anchor" from turning on the last bit.
    const EPS: f64 = 1e-9;
    elems.iter().all(|elem| match elem {
        GraphicsElem::Text { pt, .. } => pt.0 .0.abs() < EPS && pt.1 .0.abs() < EPS,
        GraphicsElem::Group(inner) | GraphicsElem::Clip(_, inner) => {
            all_nested_text_at_anchor(inner)
        }
        _ => true,
    })
}

/// Slice 2 (design doc §4 "Math"): MathML is not recoverable (structure is
/// flattened to positioned glyphs by `read_math`/`layout_math_value` well
/// before any box exists), so this renders the honest approximation instead
/// — each glyph as positioned text, each `rules`
/// element (fraction bar/radical) as an SVG path — bundled into ONE
/// self-contained, intrinsically-sized inline `<svg>` (the design doc's
/// "inline `<svg>` sized to the box").
///
/// Two sub-layers, both anchored at the SAME wrapper `(0,0)` top-left:
/// - **Glyphs**: native SVG `<text>` elements, positioned directly in the
///   `<svg>`'s own native (y-DOWN) coordinate space — `MathGlyph.dx`/`dy`
///   are box-local y-**up** offsets from the box's own baseline (the same
///   convention `GraphicsElem::Path` points use, confirmed by the PDF
///   writer's own `anchor_y + glyph.dy` arithmetic in its y-up space,
///   `rustyfi-pdf`'s `place_math`), so a local
///   `(dx, dy)` lands at SVG-native `(dx, height - dy)` — computed BY HAND
///   here (not via a `<g transform>` flip) specifically so `<text>` glyphs
///   are never inside a `scale(1,-1)` group, which would render them
///   MIRRORED upside-down (SVG text has no orientation-independence the way
///   a filled path does).
/// - **Rules**: [`crate::svg::emit_graphics`] reused VERBATIM (same call
///   shape as [`emit_graphics_box`]) for `rules` — these ARE orientation-
///   independent paths, so they go through the normal `<g transform>` flip
///   this helper already implements.
///
/// **`font-size` is written in USER UNITS, not `pt`, and that is the whole
/// point of [`math_font_size_uu`]** — see that function for why writing the
/// `pt` value with a `pt` suffix inside this viewport magnifies every glyph
/// by exactly 4/3 while leaving `dx`/`dy` and the `rules` paths alone.
fn emit_math_svg(
    out: &mut String,
    width: f64,
    height: f64,
    depth: f64,
    glyphs: &[MathGlyph],
    rules: &[GraphicsElem],
    ctx: &Ctx,
) {
    if glyphs.is_empty() && rules.is_empty() {
        return;
    }
    // Whether this run's `draw-text` rules are POSITIONING their contents or
    // merely wrapping them — see [`all_nested_text_at_anchor`]. A big
    // operator's limits reach this function rather than
    // [`emit_graphics_box`]: `text-in-math` folds the operator's own
    // `inline-graphics` into the enclosing math run's `rules`, which also
    // shifts its `pt` by wherever the operator sits in the run, so even a
    // single `draw-text (x, y)` is off-anchor once it is not the run's first
    // atom.
    let placed = !all_nested_text_at_anchor(rules);
    // A math box that draws NOTHING of its own — no glyphs, and every rule
    // a `draw-text` — must not emit the wrapper, for exactly the reason
    // [`emit_graphics_box`] does not: the wrapper would reserve the box's
    // full size a SECOND time, on top of the nested content's own. Both
    // `\paren`-style decorations `latexcmds` builds out of `draw-text` are
    // this shape, and each arrived under a blank rectangle as tall as the
    // equation. Placed content is out of flow, so there is no second
    // reservation then and the wrapper is what the placement is relative to.
    if !placed && glyphs.is_empty() && rules.iter().all(is_pure_text) {
        let mut nested = String::new();
        emit_text_only(&mut nested, rules, ctx);
        out.push_str(&nested);
        return;
    }
    open_opaque(out, ctx);
    let total_h = height + depth;
    // Built before the wrapper is opened, because whether there is nested
    // flow content decides how the wrapper states its size — see
    // [`wrapper_size`].
    let mut drawing = String::new();
    let _ = writeln!(
        drawing,
        "<svg class=\"math-glyphs\" style=\"position:absolute; left:0; top:0; overflow:visible;\" \
         width=\"{width}pt\" height=\"{total_h}pt\" viewBox=\"0 0 {width} {total_h}\">",
    );
    let mut phantom = Phantom::default();
    for (i, g) in glyphs.iter().enumerate() {
        let x = g.dx.0;
        let y = height - g.dy.0 - g.info.rising.0;
        // Every glyph is drawn from the face's own outline where one can be
        // had, so the equation does not depend on the reader having the face
        // — see [`emit_math_glyph_path`] and `Ctx::math_glyph_outline`. The
        // characters themselves survive as invisible, selectable text
        // ([`Phantom`]); without it a `<path>` would be uncopyable,
        // unsearchable and unreadable to a screen reader.
        if let Some(outline) = ctx.math_glyph_outline(g) {
            emit_math_glyph_path(&mut drawing, &outline, g, x, y);
            if let Some(text) = phantom_text(glyphs, i) {
                phantom.push(text, g, x, y);
            }
            continue;
        }
        let mut style = format!("font-size:{};", math_font_size_uu(g.info.size.0));
        if let Some(stack) = ctx.font_family_for(g.info.font) {
            style.push_str(&format!("font-family:{stack};"));
        }
        if g.info.color != Color::Gray(0.0) {
            style.push_str(&format!("fill:{};", crate::svg::css_color(g.info.color)));
        }
        let _ = writeln!(
            drawing,
            "<text x=\"{x}\" y=\"{y}\" style=\"{style}\">{}</text>",
            crate::escape_html(&g.text),
        );
    }
    phantom.finish(&mut drawing);
    drawing.push_str("</svg>\n");
    let mut nested = String::new();
    if !rules.is_empty() {
        crate::svg::emit_graphics(
            &mut drawing,
            rules,
            width,
            height,
            depth,
            0.0,
            height,
            &mut |_svg, cbx, x, y| {
                if placed {
                    emit_placed_text(&mut nested, cbx, x, y, ctx)
                } else {
                    emit_nested_text(&mut nested, cbx, ctx)
                }
            },
        );
    }
    let _ = writeln!(
        out,
        "<span class=\"math\" style=\"position:relative; display:inline-block; \
         {} vertical-align:{}pt;\">",
        wrapper_size(width, total_h, !placed && !nested.is_empty()),
        -depth,
    );
    out.push_str(&drawing);
    out.push_str(&nested);
    out.push_str("</span>\n");
}

/// One `MathGlyph`'s ink, as SVG `<path>`s of the face's own outlines —
/// placed at the same `(x, y)` the `<text>` branch would have used, which is
/// the glyph's ORIGIN (pen position), not its top-left.
///
/// **Why EVERY math glyph goes this way**, not only the variant ones.
/// A `<text>` names a face and hopes; a reader without it gets a substitute
/// whose advances are not the ones the equation was laid out against. Math is
/// the one place in this backend where that is fatal rather than untidy,
/// because every glyph is positioned ABSOLUTELY (`MathGlyph::dx`/`dy`) and
/// there is no flow to absorb the difference. Measured on the reported
/// symptom, `\forall \epsilon \: \exists \delta` at 12pt: the port reserves
/// 7.992pt for `∀` and lays `ε` down at that offset, while a substituted face
/// draws the quantifier 12.000pt wide, so the two overlap. The full argument
/// and the fallback conditions are on `Ctx::math_glyph_outline`.
///
/// **What was wrong before that.** `MathGlyph::gid` is `Some` exactly when the
/// glyph the document laid out is not the one its `text` cmaps to: an
/// OpenType MATH `MathVariants` record — a display-size big operator
/// (`push_big_char_glyph`), a stretchy delimiter or one part of a
/// `GlyphAssembly` (`push_delimiter_glyph`) — or an `ssty` script form
/// (`push_char_glyph`). The PDF writer emits the id straight into the content
/// stream (`cid.rs`'s `encode_glyph_run`); an SVG `<text>` can only address
/// the CHARACTER, so this backend drew the base glyph and there was no
/// spelling of `∑` that would have produced the display one.
///
/// **It was two symptoms of one bug, and this fixes both.** The size was the
/// visible half; the misplacement was the consequence. Measured on the
/// playground's "Displayed equations" example at 12pt, in Latin Modern Math:
/// `∑` is `summation` (advance 1.056 em) and the display variant is
/// `summation.v1` (advance 1.444 em, ink 0.056..1.387 em).
/// `layout_math_list`'s `UpperLimit`/`LowerLimit` arms centre each limit on
/// the base's own width (`center_offsets`) — 17.328pt, the VARIANT's advance,
/// because the variant is what the document laid out — so `n` and `k = 1`
/// were both centred on x = 8.664, while the base-size `∑` this backend
/// actually painted has its ink centred on x = 6.330. Every limit sat 2.334pt
/// right of the operator it belonged to. `∫` shows the same arithmetic
/// without the centring: its scripts are set to the RIGHT at the base's
/// width, so the subscript began at x = 11.988 (again the variant's advance)
/// with a 4.008pt gap after the 7.980pt base glyph. Drawing the variant
/// closes both, because every one of those offsets was already right about a
/// glyph that was not being drawn.
///
/// **Why the outline and not a scaled `<text>`**, the cheaper repair. Scaling
/// the base glyph by the advance ratio fixes the horizontal centring by
/// construction but not the ink: for `∑` the ratio is 1.367 against a true
/// height+depth ratio of 1.400 (2.5% short — fine), but for `∫` it is 1.502
/// against 2.000, leaving the integral 25% too short. The display forms are
/// separately drawn glyphs, not scalings of the base, and the two operators
/// the report names disagree by enough that no single scale factor serves
/// both. The outline is what the PDF draws, so this makes the two backends
/// agree rather than approximately agree — and it is also the only branch
/// here that does not depend on the reader having Latin Modern Math
/// installed, which for these glyphs is the difference between the right
/// shape and an arbitrary substitute.
///
/// **Geometry.** `d` is in design units, y-up (`svg::glyph_outline_d`); the
/// `<text>` around it is in the math `<svg>`'s native y-DOWN space, at
/// 1 user unit = 1 pt. So the per-element transform is the whole conversion:
/// translate to the pen position, then `scale(s, -s)` with
/// `s = size / units_per_em` — the y-flip that [`emit_math_svg`] deliberately
/// does NOT apply to `<text>` (it would mirror the letters) is exactly right
/// for a filled path, which is orientation-independent.
///
/// A record holding several characters emits one `<path>` per inked one, each
/// translated by the pen offset `Ctx::math_glyph_outline` accumulated for it
/// — already in points, so it simply adds to `x`.
///
/// **No `fill-rule`**, unlike every other path this backend writes. Glyph
/// outlines are defined under NONZERO winding — SVG's default — and CFF faces
/// in particular use overlapping contours that even-odd would punch holes in.
/// `svg.rs`'s `Fill`/`Clip` arms say `evenodd` because they are reproducing
/// PDF's `f*`; this is reproducing a font.
fn emit_math_glyph_path(out: &mut String, outline: &GlyphOutline, g: &MathGlyph, x: f64, y: f64) {
    let s = g.info.size.0 / outline.upem;
    let mut attrs = String::new();
    if g.info.color != Color::Gray(0.0) {
        attrs.push_str(&format!(" fill=\"{}\"", crate::svg::css_color(g.info.color)));
    }
    for (d, pen) in &outline.parts {
        let _ = writeln!(
            out,
            "<path d=\"{d}\" transform=\"translate({} {y}) scale({s} {})\"{attrs}/>",
            x + pen,
            -s,
        );
    }
}

/// The characters `glyphs[i]` should contribute to the document's TEXT, or
/// `None` when it should contribute none.
///
/// Almost always the record's own `text`. The exception is a stretchy
/// delimiter grown from a `GlyphAssembly`: `push_delimiter_glyph` emits one
/// `MathGlyph` per PART — a top, some extenders, a bottom — and gives every
/// one of them the same `text` and the same `dx`, since they are stacked in a
/// single column. Copying that verbatim would put `(((((` in the clipboard
/// where the page shows one tall bracket. So a record whose `text` and `dx`
/// both repeat its predecessor's is a continuation part and stays silent;
/// the first part already carries the character.
///
/// Nothing else in the corpus produces two glyph records at an identical `dx`
/// with identical text — that would be one character painted on top of
/// another, which is a layout bug rather than a construction.
fn phantom_text(glyphs: &[MathGlyph], i: usize) -> Option<&str> {
    let g = &glyphs[i];
    if g.text.is_empty() {
        return None;
    }
    if let Some(prev) = i.checked_sub(1).map(|p| &glyphs[p]) {
        if prev.text == g.text && prev.dx == g.dx {
            return None;
        }
    }
    Some(&g.text)
}

/// The invisible, SELECTABLE text that rides with a run of outlined glyphs,
/// carrying the characters the `<path>`s beside them draw.
///
/// **This is not a nicety.** A `<path>` is a shape: it cannot be selected,
/// copied, found with the browser's in-page search, or announced by a screen
/// reader. Outlining every math glyph without this would silently destroy all
/// four for every equation in the document — trading one real fidelity bug
/// for four accessibility ones. The technique is the one PDF viewers use for
/// a scanned page with an OCR layer: paint the picture, and put the text
/// behind it where the machinery that reads text can still find it.
///
/// **`fill: none` (`css.rs`'s `.math-glyphs .mphantom`), and specifically NOT
/// `visibility: hidden` or `display: none`.** The latter two remove the
/// element from the accessibility tree and from the selection along with the
/// paint, which is exactly the thing being avoided; `fill: none` removes only
/// the paint. Verified in headless chromium rather than assumed — see
/// `crates/rustyfi/tests/html_math_selection.rs`, which drives a real browser
/// over a real render.
///
/// **It steals no hit-testing from the paths.** SVG's default
/// `pointer-events: visiblePainted` tests the FILL only where a fill is
/// actually painted, and none is — so `elementFromPoint` over an equation
/// returns the wrapper, not this. Selection is unaffected by that, because it
/// walks text nodes rather than hit-testing paint. It changes no layout
/// either: SVG text contributes nothing to the flow.
///
/// **ONE `<text>` per run, one `<tspan>` per glyph**, rather than a `<text>`
/// each. Chrome serialises a selection that spans several `<text>` elements
/// with a newline between every one, so a reader copying `∀ε : ∃δ` got each
/// character on its own line; `<tspan>`s inside a single `<text>` are inline
/// and copy as `∀𝜀:∃𝛿`. It is also where the wrapper's `class` and the run's
/// shared `font-size` are paid for once instead of per glyph.
///
/// No whitespace is written between the `<tspan>`s or inside the `<text>`:
/// under SVG's default `xml:space` a newline there collapses to a real space
/// and would show up in the copied text.
///
/// **Document order is reading order**, because [`emit_math_svg`]'s loop
/// walks `glyphs` in the order the math layout produced them — a base before
/// its scripts, a numerator before its denominator — and this preserves that
/// order.
///
/// The only property carried is `font-size`, and only where a glyph departs
/// from the run's first: it sizes the selection highlight the browser paints
/// over an invisible glyph. The FAMILY is deliberately not repeated — this
/// text is never drawn, so naming a face would buy nothing and cost ~110
/// bytes on every glyph in the document.
#[derive(Default)]
struct Phantom {
    /// The `<tspan>`s so far, concatenated with no separator but the
    /// occasional deliberate space (see [`Phantom::push`]).
    spans: String,
    /// The first glyph's size, hoisted onto the enclosing `<text>`.
    size: Option<f64>,
    /// The previous glyph's right edge (`dx + width`) and baseline `y`, which
    /// is what decides whether a space belongs between it and the next.
    prev: Option<(f64, f64)>,
}

/// Two phantom glyphs are on the SAME ROW when their baselines agree to
/// within this many points — a threshold rather than equality because the
/// baselines are arithmetic on `Length`s, not copies of one value.
const PHANTOM_ROW_EPS: f64 = 0.5;

/// A horizontal gap of at least this fraction of the font size becomes a
/// space in the copied text. A word space is 0.25–0.33 em in the faces this
/// port bundles and the widest math space (`\;`, 5/18 em) is 0.28, so this
/// takes both and leaves italic correction and the sub-0.1 em inter-atom
/// kerns alone.
const PHANTOM_SPACE_EM: f64 = 0.2;

impl Phantom {
    /// Add one glyph record's characters, at the pen position the `<path>`
    /// beside it uses.
    ///
    /// **Why a gap can become a space.** Nothing else can put one there:
    /// `primitives::math_boxes_of_inline_boxes` turns the glue inside a
    /// `text-in-math` body into bare ADVANCE and keeps no character for it,
    /// so `${x \text!{ if and only if } y}` reaches this backend as four
    /// glyph records reading `if`, `and`, `only`, `if` and nothing between
    /// them. Concatenating those verbatim copies as `ifandonlyif`. The gap is
    /// the only surviving evidence that a space was set, and reading it back
    /// is what a PDF text extractor does with the same absolutely-placed
    /// glyphs — `place_math` writes one `Tj` per glyph at its own point, and
    /// poppler reconstructs the spaces the same way.
    ///
    /// **Same row only, and only forwards.** A script or a big operator's
    /// limit sits on its own baseline and at an `x` that may run BACKWARDS
    /// relative to the glyph before it (`∑` at 0, its subscript at 0.46, its
    /// superscript back at 5.70), so a gap across rows means nothing about
    /// reading order and must not manufacture a space.
    fn push(&mut self, text: &str, g: &MathGlyph, x: f64, y: f64) {
        let size = g.info.size.0;
        if let Some((prev_right, prev_y)) = self.prev {
            if (prev_y - y).abs() < PHANTOM_ROW_EPS && x - prev_right >= size * PHANTOM_SPACE_EM {
                self.spans.push(' ');
            }
        }
        self.prev = Some((x + g.width.0, y));
        let attr = match self.size {
            None => {
                self.size = Some(size);
                String::new()
            }
            Some(run) if (run - size).abs() < 1e-9 => String::new(),
            Some(_) => format!(" style=\"font-size:{};\"", math_font_size_uu(size)),
        };
        let _ = write!(
            self.spans,
            "<tspan x=\"{x}\" y=\"{y}\"{attr}>{}</tspan>",
            crate::escape_html(text),
        );
    }

    fn finish(self, out: &mut String) {
        let Some(size) = self.size else { return };
        let _ = writeln!(
            out,
            "<text class=\"mphantom\" style=\"font-size:{};\">{}</text>",
            math_font_size_uu(size),
            self.spans,
        );
    }
}

/// A math glyph's `pt` font size, spelled for the inside of
/// [`emit_math_svg`]'s viewport — i.e. in SVG USER UNITS, as a `px` length.
///
/// **The bug this exists to prevent.** The math `<svg>` is
/// `width="{w}pt" viewBox="0 0 {w} {h}"`, so one user unit renders as exactly
/// one `pt` — the deliberate "1 user unit = 1 pt" contract `svg.rs`'s module
/// comment states, and what makes `MathGlyph::dx`/`dy` and every `rules` path
/// coordinate emittable as a bare `Length` with no conversion. An ABSOLUTE
/// CSS length inside that viewport does NOT get the same treatment: `pt`
/// resolves against the CSS reference pixel *before* the viewBox transform
/// (SVG fixes 1px = 1 user unit for absolute-unit conversion), so
/// `font-size:12pt` becomes 16 user units and then renders at 16pt. Every
/// glyph came out 4/3 too big while its POSITION stayed right, so glyphs
/// overlapped each other, overflowed the fraction bars and radical overbars
/// (which, being `rules` paths, were correctly scaled), and — because the
/// wrapper `<span>` reserves only the box's own `height`/`depth` while the
/// `<svg>` is `overflow:visible` — spilled ink into the lines above and
/// below. The PDF was never affected: it positions each glyph absolutely and
/// sets the size in the content stream's own points.
///
/// **Why `px` and not a bare number**, which is what "user units" literally
/// means. A unitless length is legal in SVG only as a PRESENTATION
/// ATTRIBUTE (`font-size="12"`); this size goes into `style="…"`, which is
/// CSS, and CSS requires a unit on a non-zero `<length>`. Measured in
/// chromium inside a `viewBox="0 0 100 100"`/`width="100pt"` viewport, four
/// spellings of "12" on the same `<text>`:
///
/// | written                     | computed | user units |
/// |-----------------------------|----------|-----------:|
/// | `style="font-size:12pt"`    | `16px`   |         16 |
/// | `style="font-size:12px"`    | `12px`   |         12 |
/// | `style="font-size:12"`      | `12px`   |         12 |
/// | `font-size="12"` (attribute)| `12px`   |         12 |
///
/// So the bare `style` spelling happens to work in Blink — Blink runs the
/// SVG presentation-attribute grammar over the declaration — but it is
/// invalid CSS and Gecko drops it, which would leave the glyph at the
/// INHERITED body size with no error anywhere. `px` is the portable
/// spelling of one user unit (SVG fixes 1px = 1 user unit), so the number
/// is unchanged and only the unit is corrected: 12pt of document size ->
/// `font-size:12px` -> 12 user units -> 12pt rendered.
///
/// Every other length inside this viewport is already unitless, because
/// every other one is an attribute rather than CSS: `x`/`y` here, and
/// `svg.rs`'s `d`, `stroke-width`, `stroke-dasharray` and
/// `stroke-dashoffset`. `font-size` is the only one that had to be a
/// declaration, which is why it was the only one that got this wrong.
fn math_font_size_uu(size_pt: f64) -> String {
    format!("{size_pt}px")
}
