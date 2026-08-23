//! HTML output backend, Slice 2 (: "graphics (inline SVG)"). The
//! `place_graphics`/`emit_path` analogue (`rustyfi-pdf/src/lib.rs:708,803`) for the
//! HTML writer: turns a `PureHorzBox::Graphics`/`TabularBox::rules` element list
//! into one inline `<svg>…</svg>` (one per graphics-bearing box, per the design
//! doc's per-primitive table), a private submodule of the `rustyfi-html` crate (not
//! `pub`, not crate-wide — see that module's doc comment on why this whole feature
//! lives inside `rustyfi-pdf` rather than a new crate).
//!
//! **Coordinate system, reconciled once here.** `GraphicsElem`'s `Path`
//! coordinates are box-local and y-**up** from the box's own baseline-left
//! origin (`graphics.rs`'s `Point` doc comment); the PDF writer's
//! `place_graphics` reconciles this with a single per-box `cm` TRANSLATE
//! (no flip — PDF device space is *already* y-up, so box-local coordinates
//! need no flip to become PDF-native, only a shift to the box's placed
//! anchor). HTML/SVG's page space is y-**down** (CSS `top`, `render_html_fixed`'s
//! own doc comment), so here the per-box wrapper needs an actual flip, not
//! just a translate: [`emit_graphics`] gives each `<svg>` its own tiny
//! `width×(height+depth)` viewport with `viewBox="0 0 width (height+depth)"`
//! (so 1 SVG user unit = 1 pt, matching every `Length` value here directly,
//! with no unit conversion), CSS-positions that viewport's TOP-LEFT corner
//! at the box's placed top-left `(tx, ty - height)` (the same "baseline
//! minus ascent" arithmetic `emit_run`/`emit_box`'s `InnerString` arm uses),
//! and wraps the contents in one inner `<g transform="translate(0,height)
//! scale(1,-1)">` — the SVG analogue of `place_graphics`'s `cm`, decomposed
//! into "CSS position of the svg root" (the translate-to-anchor half) plus
//! "one local flip" (the y-up-to-y-down half PDF never needs). A local point
//! `(px, py)` then lands at SVG-local `(px, height - py)`, i.e. page
//! `(tx + px, ty - py)` — exactly mirroring how `emit_box`'s `InnerString`
//! arm turns a y-up `rising` into a page-y-down subtraction.
//!
//! **`GraphicsElem::Text` stays inside the `<svg>`, as SVG.** A `draw-text`
//! run's placed sub-boxes are ordinary `PureHorzBox`es (text runs, possibly
//! nested graphics), and both writers used to hand them back to their own
//! per-box emitter through a callback, which emitted HTML `<span>`s — while
//! the walk was between `<g …>` and `</g>`. That is not merely a
//! coordinate-frame divergence, it is invalid: `span` is on the HTML
//! parser's foreign-content BREAKOUT list, so a browser reading it closes
//! the `<svg>` early and reparses the rest of the drawing as HTML. The
//! easytable manual alone emitted 1559 such elements.
//!
//! So [`draw_box`] renders those sub-boxes as real SVG instead: a text run
//! becomes `<text>`, a nested graphic a translated `<g>`. Both live in the
//! box-local y-up frame the enclosing `<g transform>` already establishes,
//! so no page coordinates are involved at all and the caller supplies only a
//! font resolver ([`FontResolver`]) rather than a whole nested emitter. Text
//! needs one extra `scale(1,-1)` of its own to come out upright inside that
//! flip — glyphs, unlike filled paths, are not orientation-independent.
//!
//! What SVG genuinely cannot host is a nested `Image`, `Tabular` or
//! `EmbeddedBlock` — an image needs the document's image table, which this
//! module has no access to, and the other two are block layout, which SVG
//! only takes through `<foreignObject>`. `figbox`'s manual puts all three
//! under a `draw-text`, so dropping them is not an option: they go into
//! [`emit_graphics`]'s `deferred` out-parameter with their box-local
//! coordinates, for the caller to emit AFTER the `</svg>` — where the
//! faithful writer positions them absolutely, exactly as its old callback
//! did, and the reflowing one lets them flow.

use rustyfi_backend::{
    Closing, Color, FontKey, GraphicsElem, HorzStringInfo, Path, PathSeg, PureHorzBox,
};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Resolves a run's [`FontKey`] to the CSS `font-family` naming its
/// `@font-face` embedding, recording the file as used — i.e. each writer's
/// own `Ctx::font_family_for`. `None` in base-14 mode. This is all
/// [`emit_graphics`] needs from its caller now that `GraphicsElem::Text` is
/// rendered here rather than handed back (see this module's doc comment).
pub(super) type FontResolver<'a> = &'a mut dyn FnMut(FontKey) -> Option<String>;

/// A `draw-text` sub-box SVG cannot host, with its BOX-LOCAL y-up position
/// (`x` from the box's left edge, `y` from its baseline) — see this module's
/// doc comment. The caller emits these after [`emit_graphics`] returns; the
/// page-space anchor is `(tx + x, ty - y)`, the same formula every other arm
/// of a writer's own `emit_box` uses.
pub(super) type Deferred = Vec<(f64, f64, PureHorzBox)>;

/// Emit one graphics-bearing box's `elems` as a single inline `<svg>`,
/// CSS-positioned at the box's placed top-left corner (`tx` the left edge,
/// `ty` the baseline — the same convention every other `emit_box` arm uses).
/// `width`/`height`/`depth` are the box's own outer metrics (`Graphics`'s or
/// `TabularBox`'s own fields), used only to size the `<svg>` viewport/
/// `viewBox` (see the module doc comment). Emits nothing for an empty
/// `elems` (mirrors `page_content`'s `place_graphics` guard, `lib.rs:575`,
/// which skips the wrapper entirely rather than emitting a vacuous `q…Q`).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_graphics(
    out: &mut String,
    elems: &[GraphicsElem],
    width: f64,
    height: f64,
    depth: f64,
    tx: f64,
    ty: f64,
    fonts: FontResolver<'_>,
    deferred: &mut Deferred,
) {
    if elems.is_empty() {
        return;
    }
    let total_h = height + depth;
    let top = ty - height;
    let _ = write!(
        out,
        "<svg class=\"gfx\" style=\"position:absolute; left:{tx}pt; top:{top}pt; overflow:visible;\" \
         width=\"{width}pt\" height=\"{total_h}pt\" viewBox=\"0 0 {width} {total_h}\">\n\
         <g transform=\"translate(0,{height}) scale(1,-1)\">\n",
    );
    emit_elems(out, elems, fonts, deferred);
    out.push_str("</g>\n</svg>\n");
}

/// The recursive element walker (`Group`/`Clip` reenter this, not
/// [`emit_graphics`], so a nested container never gets its own `<svg>`
/// wrapper — exactly `place_graphics`'s own `Group`/`Clip` arms, which
/// recurse into itself, not `page_content`'s `q`/`cm` prologue).
fn emit_elems(
    out: &mut String,
    elems: &[GraphicsElem],
    fonts: FontResolver<'_>,
    deferred: &mut Deferred,
) {
    for elem in elems {
        match elem {
            // Even-odd fill, matching `place_graphics`'s `content.fill_even_odd()`
            // (upstream `op_f'`, not nonzero winding — same rationale as the
            // PDF writer's own doc comment, `lib.rs:720-723`).
            GraphicsElem::Fill(color, path) => {
                let _ = write!(
                    out,
                    "<path d=\"{}\" fill=\"{}\" fill-rule=\"evenodd\" stroke=\"none\"/>\n",
                    path_d(path),
                    css_color(*color),
                );
            }
            GraphicsElem::Stroke(width, color, path) => {
                let _ = write!(
                    out,
                    "<path d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\"/>\n",
                    path_d(path),
                    css_color(*color),
                    width.0,
                );
            }
            // `dashed-stroke`: identical to `Stroke` plus SVG's dash-array/
            // -offset attributes (the `stroke-dasharray`/`stroke-dashoffset`
            // analogue of the PDF writer's `set_dash_pattern`, `lib.rs:740`).
            GraphicsElem::DashedStroke(width, dash, color, path) => {
                let _ = write!(
                    out,
                    "<path d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\" \
                     stroke-dasharray=\"{} {}\" stroke-dashoffset=\"{}\"/>\n",
                    path_d(path),
                    css_color(*color),
                    width.0,
                    dash.0 .0,
                    dash.1 .0,
                    dash.2 .0,
                );
            }
            // `draw-text` (roadmap C1 upstream): its sub-boxes are drawn as
            // SVG, in this same box-local y-up frame, at the element's own
            // point plus each box's `dx`. See this module's doc comment on
            // why they are no longer handed back to the caller as HTML.
            //
            // `transform` (`linear-transform-graphics`) is the run's own 2×2
            // about its local origin, applied BEFORE the `pt` translation —
            // exactly the PDF writer's `cm [a c b d ptx pty]`
            // (`rustyfi-pdf/src/lib.rs`'s own `Text` arm), which is
            // component-for-component an SVG `matrix(a,c,b,d,ptx,pty)`. The
            // callback this arm replaced ignored it outright, so a rotated
            // `draw-text` came out upright.
            GraphicsElem::Text {
                pt,
                contents,
                transform,
                ..
            } => match transform {
                None => {
                    for (dx, bx) in contents {
                        draw_box(out, bx, (pt.0 + *dx).0, pt.1 .0, fonts, deferred);
                    }
                }
                Some((a, b, c, d)) => {
                    let _ = write!(
                        out,
                        "<g transform=\"matrix({a},{c},{b},{d},{},{})\">\n",
                        pt.0 .0, pt.1 .0,
                    );
                    for (dx, bx) in contents {
                        draw_box(out, bx, dx.0, 0.0, fonts, deferred);
                    }
                    out.push_str("</g>\n");
                }
            },
            // L5b (prim-retype-sweep.md §3.3): 0.1's `graphics` collection
            // container nodes — never reached by any 0.0.6 program (see
            // `GraphicsElem`'s own doc comment), handled anyway for parity
            // with the PDF writer's exhaustive match (no wildcard arm there
            // either). A bare `<g>` — no transform of its own, since the
            // enclosing `<g transform>` from `emit_graphics` (or an outer
            // `Group`/`Clip`) already applies.
            GraphicsElem::Group(inner) => {
                out.push_str("<g>\n");
                emit_elems(out, inner, fonts, deferred);
                out.push_str("</g>\n");
            }
            // `graphicD.ml:323-336`'s clip: an SVG `<clipPath>` definition
            // referenced by a `<g clip-path="url(#…)">` wrapper — the
            // `path`/`W*`/`n` PDF sequence's SVG equivalent (even-odd, same
            // fill-rule as `Fill`). IDs are process-global and monotonic
            // (`next_clip_id`) since a document can nest/repeat clips freely
            // and SVG `id`s must be unique within one document.
            GraphicsElem::Clip(path, inner) => {
                let id = next_clip_id();
                let _ = write!(
                    out,
                    "<clipPath id=\"html-clip-{id}\"><path d=\"{}\" fill-rule=\"evenodd\"/></clipPath>\n\
                     <g clip-path=\"url(#html-clip-{id})\">\n",
                    path_d(path),
                );
                emit_elems(out, inner, fonts, deferred);
                out.push_str("</g>\n");
            }
            // Not ink: a deferred `register-destination` marker, already
            // consumed by `fire_hooks` into `DocExtras::destinations`. The PDF
            // writer skips it for the same reason (`rustyfi-pdf/src/lib.rs`'s
            // `place_graphics` guard) — there it becomes a `/Dests` catalog
            // entry rather than a content-stream op; here the anchor is
            // emitted by the surrounding writer, not by the SVG.
            GraphicsElem::Destination { .. } => {}
        }
    }
}

/// One `draw-text` sub-box, drawn as SVG at box-local y-up `(x, y)` —
/// `y` being its baseline. Recursive: a `Frame` is transparent, a nested
/// `Graphics` becomes a translated `<g>` whose contents go back through
/// [`emit_elems`] in the same frame. See this module's doc comment for what
/// is deliberately not drawn, and why this is SVG rather than HTML.
fn draw_box(
    out: &mut String,
    bx: &PureHorzBox,
    x: f64,
    y: f64,
    fonts: FontResolver<'_>,
    deferred: &mut Deferred,
) {
    match bx {
        PureHorzBox::InnerString { info, text, .. } => {
            draw_text_run(out, info, text, x, y + info.rising.0, fonts)
        }
        // Transparent: every child sits on the frame's own baseline, only
        // `dx` varying — the same treatment the two page writers' `Frame`
        // arms give it.
        PureHorzBox::Frame { contents, .. } => {
            for (dx, cbx) in contents {
                draw_box(out, cbx, x + dx.0, y, fonts, deferred);
            }
        }
        // A graphic inside a `draw-text` inside a graphic. Its `elems` are
        // local to ITS baseline-left origin, so one translate places them —
        // no second flip, since the outer `<g>`'s is still in force.
        PureHorzBox::Graphics { elems, .. } => {
            let _ = write!(out, "<g transform=\"translate({x},{y})\">\n");
            emit_elems(out, elems, fonts, deferred);
            out.push_str("</g>\n");
        }
        // Math flattens to positioned glyphs plus rules, both already in the
        // y-up box-local convention — the same two layers `reflow/inline.rs`
        // draws for a top-level `Math` box.
        PureHorzBox::Math { glyphs, rules, .. } => {
            for g in glyphs {
                draw_text_run(
                    out,
                    &g.info,
                    &g.text,
                    x + g.dx.0,
                    y + g.dy.0 + g.info.rising.0,
                    fonts,
                );
            }
            if !rules.is_empty() {
                let _ = write!(out, "<g transform=\"translate({x},{y})\">\n");
                emit_elems(out, rules, fonts, deferred);
                out.push_str("</g>\n");
            }
        }
        // Block layout and images: SVG has no element for these, so they go
        // back to the caller with their box-local position (see this
        // module's doc comment and [`Deferred`]).
        PureHorzBox::Image { .. } | PureHorzBox::Tabular(_) | PureHorzBox::EmbeddedBlock { .. } => {
            deferred.push((x, y, bx.clone()))
        }
        // Glue, markers and hooks carry no ink at all.
        _ => {}
    }
}

/// One run of drawn text as an SVG `<text>` whose origin is its baseline
/// start. The `scale(1,-1)` undoes [`emit_graphics`]'s per-box flip for this
/// element only: a filled path is orientation-independent and wants the
/// flip, a glyph is not and would come out mirrored.
fn draw_text_run(
    out: &mut String,
    info: &HorzStringInfo,
    text: &str,
    x: f64,
    y: f64,
    fonts: FontResolver<'_>,
) {
    if text.is_empty() {
        return;
    }
    let mut style = format!("font-size:{}pt;", info.size.0);
    if let Some(family) = fonts(info.font) {
        // Unquoted — `fonts::font_family_name` is a bare CSS identifier
        // exactly so it survives an attribute.
        let _ = write!(style, "font-family:{family};");
    }
    if info.color != Color::Gray(0.0) {
        let _ = write!(style, "fill:{};", css_color(info.color));
    }
    let _ = write!(
        out,
        "<text transform=\"translate({x},{y}) scale(1,-1)\" style=\"{style}\">{}</text>\n",
        crate::escape_html(text),
    );
}

/// Process-global monotonic counter for `<clipPath id>` uniqueness (SVG IDs
/// must be unique within one document; a page can contain arbitrarily many
/// independent `Clip` elements, and `render_html_fixed` has no other natural
/// "clip index" to thread through the box walker).
static NEXT_CLIP_ID: AtomicUsize = AtomicUsize::new(0);

fn next_clip_id() -> usize {
    NEXT_CLIP_ID.fetch_add(1, Ordering::Relaxed)
}

/// One `Path`'s subpaths as an SVG `d` attribute value: `M` the start,
/// `PathSeg::Line`->`L`, `PathSeg::Bezier`->`C`, then the closing
/// (`Closing::Open`-> nothing, `Closing::Line`->`Z`, `Closing::Bezier`-> one
/// final `C` back to the subpath's own `start`, then `Z`) — the direct SVG
/// analogue of `emit_path`'s `m`/`l`/`c`/`h` PDF operators (`lib.rs:803`).
/// Coordinates are written RAW (box-local, y-up) — the single per-box `<g
/// transform>` in [`emit_graphics`] does the y-flip once, exactly like the
/// PDF writer's `cm`; a per-coordinate flip here would double up (see the
/// PDF writer's own "don't add one" warning, `lib.rs:699-700`, which applies
/// verbatim here).
fn path_d(path: &Path) -> String {
    let mut d = String::new();
    for sub in &path.subpaths {
        let _ = write!(d, "M{} {} ", sub.start.0 .0, sub.start.1 .0);
        for seg in &sub.segs {
            match seg {
                PathSeg::Line(pt) => {
                    let _ = write!(d, "L{} {} ", pt.0 .0, pt.1 .0);
                }
                PathSeg::Bezier(c1, c2, dest) => {
                    let _ = write!(
                        d,
                        "C{} {} {} {} {} {} ",
                        c1.0 .0, c1.1 .0, c2.0 .0, c2.1 .0, dest.0 .0, dest.1 .0,
                    );
                }
            }
        }
        match sub.closing {
            Closing::Open => {}
            Closing::Line => d.push_str("Z "),
            Closing::Bezier(c1, c2) => {
                let _ = write!(
                    d,
                    "C{} {} {} {} {} {} ",
                    c1.0 .0, c1.1 .0, c2.0 .0, c2.1 .0, sub.start.0 .0, sub.start.1 .0,
                );
                d.push_str("Z ");
            }
        }
    }
    d.trim_end().to_string()
}

/// `Color` -> a CSS `rgb()` string. `Gray`/`Rgb` are exact (`f64` 0..=1 ->
/// `u8` 0..=255, rounded); `Cmyk` has no CSS/SVG device-CMYK analogue (this
/// module's doc comment on `Color` upstream, `graphics.rs:26-34`, and the
/// design doc's §Risks "CMYK color" note), so this applies the standard
/// naive subtractive conversion `channel = (1 - c_channel) * (1 - k)` —
/// lossy, documented, and the same class of approximation the design doc
/// flags as acceptable for a preview/visual-diff use case, not
/// color-managed print output.
pub(super) fn css_color(color: Color) -> String {
    let (r, g, b) = match color {
        Color::Gray(v) => (v, v, v),
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Cmyk(c, m, y, k) => (
            (1.0 - c) * (1.0 - k),
            (1.0 - m) * (1.0 - k),
            (1.0 - y) * (1.0 - k),
        ),
    };
    let to_u8 = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("rgb({},{},{})", to_u8(r), to_u8(g), to_u8(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyfi_backend::Length;

    #[test]
    fn cmyk_black_and_white_round_trip() {
        // K=1 (all channels) -> black regardless of C/M/Y; K=0,C=M=Y=0 ->
        // white — the two unambiguous sanity checks for the naive formula.
        assert_eq!(css_color(Color::Cmyk(0.0, 0.0, 0.0, 1.0)), "rgb(0,0,0)");
        assert_eq!(
            css_color(Color::Cmyk(0.0, 0.0, 0.0, 0.0)),
            "rgb(255,255,255)"
        );
        // Pure cyan (C=1, M=Y=K=0) drops the red channel only.
        assert_eq!(css_color(Color::Cmyk(1.0, 0.0, 0.0, 0.0)), "rgb(0,255,255)");
    }

    #[test]
    fn path_d_maps_line_and_bezier_segments() {
        let path = Path {
            subpaths: vec![rustyfi_backend::Subpath {
                start: (Length::pt(0.0), Length::pt(0.0)),
                segs: vec![
                    PathSeg::Line((Length::pt(10.0), Length::pt(0.0))),
                    PathSeg::Bezier(
                        (Length::pt(10.0), Length::pt(5.0)),
                        (Length::pt(5.0), Length::pt(10.0)),
                        (Length::pt(0.0), Length::pt(10.0)),
                    ),
                ],
                closing: Closing::Line,
            }],
        };
        let d = path_d(&path);
        assert_eq!(d, "M0 0 L10 0 C10 5 5 10 0 10 Z");
    }
}
