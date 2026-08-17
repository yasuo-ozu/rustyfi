//! `PureHorzBox` → inline HTML (`docs/plans/design-reflowable-html.md` §3
//! "Inline level"), appending into an already-open paragraph's (or inline
//! frame's) text buffer — never absolutely positioned, never carrying an
//! x/y of its own; the browser lays every span out in normal inline flow.
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
//! reflow_links`/`reflow_dests`). `GraphicsOuter`/`Image`/`Footnote` remain
//! inert PLACEHOLDER `<span>`s (`GraphicsOuter` in particular is a
//! lang-side-only DEFERRED callback — `resolve_outer_graphics_in_contents`,
//! `rustyfi-lang/src/primitives.rs:3917`, always resolves it to a plain
//! `Graphics` box during `line-break`, well before `reflow_source` is
//! captured, so this arm is realistically unreachable; kept as an honest
//! placeholder rather than assumed-dead code).
//! `HookPageBreak`/`FrameMarker` render nothing (no reflow meaning, same as
//! both the faithful HTML writer and the PDF writer's own wildcard arms).
//!
//! Slice 3 (design doc §6 "S3", `structure.rs`'s doc comment) replaces the
//! `Tabular` PLACEHOLDER `<span>` with a real `<table>` — see this module's
//! own `Tabular` arm below for why it delegates to `structure::render_table`
//! (`block.rs` handles the common top-level case directly, since a `<table>`
//! is block-level and needs to flush the surrounding paragraph first; this
//! arm is only the fallback for a `Tabular` nested inside inline content).
//!
//! Slice 4 (`docs/plans/design-reflow-s4-lists.md` §4.2 "Inline level")
//! handles the new `InlineMark` box: `EmphStart`/`EmphEnd` open/close a real
//! `<em>`/`<strong>` (via `Ctx::emph_stack`, since `EmphEnd` alone doesn't
//! say which tag to close — see that field's doc comment), and
//! `BulletStart`/`BulletEnd` fence a drawn bullet/number glyph run so it
//! renders NOTHING (`Ctx::bullet_suppress`) — the real marker comes from the
//! `<ul>`/`<ol>` `block.rs` now emits instead.

use std::fmt::Write as _;

use rustyfi_backend::{
    AnnotAction, Color, GraphicsElem, HorzStringInfo, InlineMarkKind, MathGlyph, PureHorzBox,
};

use super::Ctx;

/// Append `bx`'s reflow rendering to `out`. Never touches `out`'s
/// surrounding whitespace/paragraph bookkeeping — that is the caller's
/// (`block.rs`'s) job.
pub(crate) fn emit_inline(out: &mut String, bx: &PureHorzBox, ctx: &Ctx) {
    match bx {
        // S4 (`docs/plans/design-reflow-s4-lists.md` §4.2): handled FIRST,
        // unconditionally of `ctx.bullet_suppress` below — a `BulletEnd`
        // reached WHILE suppressed must still clear the counter, and an
        // `EmphStart`/`EmphEnd` reached while suppressed (should not happen
        // — `itemize.satyh` never nests emphasis inside its own bullet
        // fence — but stays correct regardless) must still keep the tag
        // stack balanced.
        PureHorzBox::InlineMark(kind) => match kind {
            InlineMarkKind::EmphStart { strong } => {
                ctx.emph_stack.borrow_mut().push(*strong);
                out.push_str(if *strong { "<strong>" } else { "<em>" });
            }
            InlineMarkKind::EmphEnd => {
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
        },

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

        // Glue: the browser re-breaks lines itself, so the exact
        // natural/stretch/shrink amounts have no meaning here — collapse to
        // one space (HTML's own whitespace-collapsing then does the rest,
        // since this module's CSS never sets `white-space: pre`, unlike the
        // faithful mode).
        PureHorzBox::OuterEmpty { .. } | PureHorzBox::OuterFil | PureHorzBox::FixedEmpty { .. } => {
            out.push(' ');
        }

        // A break point that may or may not be taken; the browser
        // re-hyphenates on its own; render a soft hyphen so a manual
        // hyphenation opportunity survives without forcing a visible one.
        PureHorzBox::Discretionary { .. } => {
            out.push_str("&shy;");
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
            if let Some(action) = ctx.links.get(deco) {
                let href = match action {
                    AnnotAction::Uri(uri) => crate::escape_html(uri),
                    AnnotAction::GotoName(name) => format!("#{}", crate::escape_html(name)),
                };
                out.push_str(&format!("<a class=\"link\" href=\"{href}\">"));
                for (_, cbx) in contents {
                    emit_inline(out, cbx, ctx);
                }
                out.push_str("</a>");
            } else if let Some(name) = ctx.dests.get(deco) {
                out.push_str(&format!(
                    "<span class=\"iframe\" id=\"{}\">",
                    crate::escape_html(name)
                ));
                for (_, cbx) in contents {
                    emit_inline(out, cbx, ctx);
                }
                out.push_str("</span>");
            } else {
                out.push_str("<span class=\"iframe\">");
                for (_, cbx) in contents {
                    emit_inline(out, cbx, ctx);
                }
                out.push_str("</span>");
            }
        }

        // Math is flattened to positioned glyphs at eval time (design doc
        // §4) — no fraction/sub/sup structure survives to render as MathML,
        // so (Slice 2, design doc §4's "the honest option") this renders the
        // SAME way the faithful backend's approximation does: each glyph as
        // positioned text, each rule (fraction bar/radical) as an SVG path
        // — but bundled into ONE self-contained, intrinsically-sized inline
        // `<svg>` (see [`emit_math_svg`]) instead of the faithful mode's
        // page-absolute `<span>`s, since reflow has no page to be absolute
        // WITHIN.
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
        } => emit_graphics_box(out, width.0, height.0, depth.0, elems, ctx),

        // `GraphicsOuter` is a DEFERRED lang-side callback (`GraphicsFnId`,
        // resolved only by `resolve_outer_graphics_in_contents` at
        // `line-break` time, well before `reflow_source` is captured — see
        // this module's doc comment); the backend has no way to run it, same
        // limitation the faithful writer has (its own `emit_box` has no arm
        // for this variant at all, silently matching its wildcard). Kept as
        // an honest placeholder rather than silently dropped.
        PureHorzBox::GraphicsOuter { .. } => {
            out.push_str(
                "<span class=\"gfx-placeholder\" title=\"unresolved inline-graphics-outer (lang-side callback)\"></span>",
            );
        }
        // Out of scope for this backend so far — no dedicated recovery lever
        // exists for either (`Image` has no data-URI plumbing here yet;
        // `Footnote` has no linked-`<aside>` collection pass).
        PureHorzBox::Image { .. } => {
            out.push_str(
                "<span class=\"image-placeholder\" title=\"image rendering deferred to a later reflow slice\"></span>",
            );
        }
        // S3 (`docs/plans/design-reflowable-html.md` §6 "S3",
        // `structure.rs`'s "Tables — genuinely recoverable"): a real
        // `<table>`. `block.rs`'s own `VertBox::Line` walk already special-
        // cases the common top-level case (flushing the open paragraph
        // first, since a `<table>` is block-level); THIS arm is the fallback
        // for a `Tabular` nested inside inline content this module recurses
        // into on its own (a `Frame`'s `contents`, or a table cell that
        // itself contains a nested `Tabular`) — no surrounding paragraph to
        // flush here, so no `extra_attrs` margin.
        PureHorzBox::Tabular(tab) => super::structure::render_table(out, tab, "", ctx),
        PureHorzBox::Footnote { .. } => {
            out.push_str(
                "<span class=\"footnote-placeholder\" title=\"footnote rendering deferred to a later reflow slice\"></span>",
            );
        }

        // `EmbeddedBlock` is handled one level up, in `block.rs`'s own
        // per-`Line`-contents loop (it needs to CLOSE the open paragraph,
        // which this function's `&mut String` signature has no way to do)
        // — unreachable in practice, kept as an explicit inert arm rather
        // than a silent catch-all so a future new `PureHorzBox` variant
        // still forces a compile error here instead of silently falling
        // through.
        PureHorzBox::EmbeddedBlock { .. } => {}

        // No reflow meaning (zero-width markers/hooks; matches the
        // faithful writer's own wildcard treatment of these two).
        PureHorzBox::HookPageBreak { .. } | PureHorzBox::FrameMarker { .. } => {}
    }
}

/// One `InnerString` run: escaped text wrapped in a `<span>` carrying its
/// font/size/color/rising as CSS — no `left`/`top`/`position` (this is
/// flowing content, not a placed box). `vertical-align` (not `position`)
/// handles a non-zero `rising`, since it needs no positioned ancestor and
/// composes correctly with the surrounding inline flow.
fn emit_run(out: &mut String, info: &HorzStringInfo, text: &str, ctx: &Ctx) {
    let mut style = String::new();
    if let Some(family) = ctx.font_family_for(info.font) {
        style.push_str(&format!("font-family:\"{family}\";"));
    }
    style.push_str(&format!("font-size:{}pt;", info.size.0));
    // Non-black only, mirroring the faithful writer's own guard, so a plain
    // black run's `<span>` stays uncluttered.
    if info.color != Color::Gray(0.0) {
        style.push_str(&format!("color:{};", crate::svg::css_color(info.color)));
    }
    if info.rising.0 != 0.0 {
        style.push_str(&format!("vertical-align:{}pt;", info.rising.0));
    }
    out.push_str(&format!(
        "<span class=\"run\" style=\"{style}\">{}</span>",
        crate::escape_html(text),
    ));
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
/// re-enters THIS module's [`emit_inline`] rather than the faithful writer's
/// page-absolute `emit_box`, since reflow has no page coordinates to place
/// nested content at; the `_x`/`_y` callback args are therefore unused here
/// (a `draw-text` run's nested boxes render inline, at their natural flow
/// position within the wrapper, not at their SVG-local point — a documented
/// approximation for this rare construction, same spirit as the faithful
/// mode's own `Math.glyph.gid` approximation).
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
    let total_h = height + depth;
    let _ = write!(
        out,
        "<span class=\"gfx\" style=\"position:relative; display:inline-block; \
         width:{width}pt; height:{total_h}pt; vertical-align:{}pt;\">\n",
        -depth,
    );
    crate::svg::emit_graphics(out, elems, width, height, depth, 0.0, height, &mut |out, cbx, _x, _y| {
        emit_inline(out, cbx, ctx)
    });
    out.push_str("</span>\n");
}

/// Slice 2 (design doc §4 "Math"): MathML is not recoverable (structure is
/// flattened to positioned glyphs by `read_math`/`layout_math_value` well
/// before any box exists), so this renders the SAME honest approximation the
/// faithful backend does — each glyph as positioned text, each `rules`
/// element (fraction bar/radical) as an SVG path — bundled into ONE
/// self-contained, intrinsically-sized inline `<svg>` (the design doc's
/// "inline `<svg>` sized to the box").
///
/// Two sub-layers, both anchored at the SAME wrapper `(0,0)` top-left:
/// - **Glyphs**: native SVG `<text>` elements, positioned directly in the
///   `<svg>`'s own native (y-DOWN) coordinate space — `MathGlyph.dx`/`dy`
///   are box-local y-**up** offsets from the box's own baseline (the same
///   convention `GraphicsElem::Path` points use, confirmed by the faithful
///   writer's `ty - g.dy.0` arithmetic, `lib.rs`'s `Math` arm), so a local
///   `(dx, dy)` lands at SVG-native `(dx, height - dy)` — computed BY HAND
///   here (not via a `<g transform>` flip) specifically so `<text>` glyphs
///   are never inside a `scale(1,-1)` group, which would render them
///   MIRRORED upside-down (SVG text has no orientation-independence the way
///   a filled path does).
/// - **Rules**: [`crate::svg::emit_graphics`] reused VERBATIM (same call
///   shape as [`emit_graphics_box`]) for `rules` — these ARE orientation-
///   independent paths, so they go through the normal `<g transform>` flip
///   this helper already implements.
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
    let total_h = height + depth;
    let _ = write!(
        out,
        "<span class=\"math\" style=\"position:relative; display:inline-block; \
         width:{width}pt; height:{total_h}pt; vertical-align:{}pt;\">\n\
         <svg class=\"math-glyphs\" style=\"position:absolute; left:0; top:0; overflow:visible;\" \
         width=\"{width}pt\" height=\"{total_h}pt\" viewBox=\"0 0 {width} {total_h}\">\n",
        -depth,
    );
    for g in glyphs {
        let x = g.dx.0;
        let y = height - g.dy.0 - g.info.rising.0;
        let mut style = format!("font-size:{}pt;", g.info.size.0);
        if let Some(family) = ctx.font_family_for(g.info.font) {
            style.push_str(&format!("font-family:\"{family}\";"));
        }
        if g.info.color != Color::Gray(0.0) {
            style.push_str(&format!("fill:{};", crate::svg::css_color(g.info.color)));
        }
        let _ = write!(
            out,
            "<text x=\"{x}\" y=\"{y}\" style=\"{style}\">{}</text>\n",
            crate::escape_html(&g.text),
        );
    }
    out.push_str("</svg>\n");
    if !rules.is_empty() {
        crate::svg::emit_graphics(out, rules, width, height, depth, 0.0, height, &mut |out, cbx, _x, _y| {
            emit_inline(out, cbx, ctx)
        });
    }
    out.push_str("</span>\n");
}
