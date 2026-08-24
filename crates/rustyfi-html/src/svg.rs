//! The `place_graphics`/`emit_path` analogue (`rustyfi-pdf/src/lib.rs`) for
//! the HTML backend: turns a `PureHorzBox::Graphics`/`TabularBox::rules`
//! element list into one inline `<svg>…</svg>`, one per graphics-bearing
//! box. A private submodule of the `rustyfi-html` crate — `pub(super)`, not
//! `pub`: nothing outside the crate builds SVG.
//!
//! It also holds [`glyph_outline_d`], the one path in this backend that comes
//! from a FONT rather than from a `GraphicsElem` — the MATH-table variant
//! glyphs (`MathGlyph::gid`) that no `<text>` can address. Same `d` grammar,
//! different source; see that function.
//!
//! **Coordinate system, reconciled once here.** `GraphicsElem`'s `Path`
//! coordinates are box-local and y-**up** from the box's own baseline-left
//! origin (`graphics.rs`'s `Point` doc comment); the PDF writer's
//! `place_graphics` reconciles this with a single per-box `cm` TRANSLATE
//! (no flip — PDF device space is *already* y-up, so box-local coordinates
//! need no flip to become PDF-native, only a shift to the box's placed
//! anchor). HTML/SVG's page space is y-**down** (CSS `top`), so here the
//! per-box wrapper needs an actual flip, not just a translate:
//! [`emit_graphics`] gives each `<svg>` its own tiny `width×(height+depth)`
//! viewport with `viewBox="0 0 width (height+depth)"` (so 1 SVG user unit =
//! 1 pt, matching every `Length` value here directly, with no unit
//! conversion), CSS-positions that viewport's TOP-LEFT corner at the box's
//! top-left `(tx, ty - height)` (the usual "baseline minus ascent"
//! arithmetic), and wraps the contents in one inner `<g
//! transform="translate(0,height) scale(1,-1)">` — the SVG analogue of
//! `place_graphics`'s `cm`, decomposed into "CSS position of the svg root"
//! (the translate-to-anchor half) plus "one local flip" (the y-up-to-y-down
//! half PDF never needs). A local point `(px, py)` then lands at SVG-local
//! `(px, height - py)`, i.e. outer-frame `(tx + px, ty - py)`.
//!
//! **`GraphicsElem::Text` breaks that decomposition on purpose.** A
//! `draw-text` run's placed sub-boxes are ordinary `PureHorzBox`es (text
//! runs, possibly nested graphics/images), which the caller emits as
//! `<span>`/`<svg>` children positioned relative to the nearest positioned
//! ancestor, NOT this box's local `<g transform>` (CSS absolute positioning
//! does not compose with an SVG sibling's coordinate transform the way a PDF
//! content-stream operator composes with the active CTM). So `Text` is
//! handled OUTSIDE the `<svg>`/`<g>` nest entirely: its nested boxes are
//! handed to the `nested` callback with coordinates computed by hand
//! (`tx + pt.x + dx`, `ty - pt.y`), the one documented divergence from the
//! PDF writer's `place_graphics` (whose `emit_nested` callback runs INSIDE
//! the `q`/`cm` block precisely because PDF text ops DO compose with the
//! CTM).
//!
//! **"OUTSIDE" is enforced HERE, not left to the caller.** The callback is
//! handed a SIDE buffer that [`emit_graphics`] appends after its own
//! `</svg>`, never the `out` the drawing is going into. An HTML element
//! inside `<svg>` and outside a `<foreignObject>` is not merely unusual, it
//! ends the parser's foreign-content insertion mode: `span` is on the HTML
//! parser's own breakout list, so the browser CLOSES the `<svg>` at the
//! first nested run and every element after it lands outside the drawing
//! and never renders. That was measurable — 16 of `latexcmds`' 208
//! `--format html-fixed` `<path>`s were parsed clean out of their `<svg>`
//! — and invisible in the source, which is well-formed XML. Placement is
//! unaffected: the nested boxes carry absolute coordinates of their own, so
//! where in the DOM they sit does not move them.
//!
//! That last sentence is a statement about this module's CONTRACT, and it is
//! the callback's job to hold up its end. The `(x, y)` handed to `nested` is
//! the run's real anchor — its left edge and its BASELINE — and a consumer
//! that appends the content and ignores them gets the wrapper origin
//! instead, which is only the same place when the run was anchored there.
//! `reflow::inline`'s `emit_placed_text` is where they are honoured.

use rustyfi_backend::{Closing, Color, GraphicsElem, Path, PathSeg, PureHorzBox};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The nested-emitter callback type — the `&mut String` analogue of
/// `rustyfi-pdf`'s own `NestedEmitter` (`lib.rs`). [`emit_graphics`] invokes
/// this only for `GraphicsElem::Text`'s sub-boxes (see this module's doc
/// comment on why they can't stay inside the SVG's local coordinate frame);
/// every other variant is handled directly by this module.
pub(super) type NestedEmitter<'a> = &'a mut dyn FnMut(&mut String, &PureHorzBox, f64, f64);

/// [`graphics_background`]'s answer: the `<svg>` document, then the ink
/// bounding box it was cropped to as `(x0, y0)`/`(x1, y1)`, in the caller's
/// own box-local, y-**up** coordinates.
pub(super) type MeasuredSvg = (String, (f64, f64), (f64, f64));

/// Emit one graphics-bearing box's `elems` as a single inline `<svg>`,
/// CSS-positioned at `(tx, ty - height)`: `tx` is the box's left edge and
/// `ty` its BASELINE, both in whatever frame the caller has made the
/// `position:relative` ancestor. Every caller today passes `(0.0, height)` —
/// a baseline `height` down from the top of the inline-block wrapper it has
/// just opened — so the `<svg>` lands at that wrapper's own top-left corner.
/// `width`/`height`/`depth` are the box's own outer metrics (`Graphics`'s or
/// `TabularBox`'s own fields), used only to size the `<svg>` viewport/
/// `viewBox` (see the module doc comment). Emits nothing for an empty
/// `elems` — the same guard `page_content` puts around its own
/// `place_graphics` overlay call, which skips the wrapper entirely rather
/// than emitting a vacuous `q…Q`.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_graphics(
    out: &mut String,
    elems: &[GraphicsElem],
    width: f64,
    height: f64,
    depth: f64,
    tx: f64,
    ty: f64,
    nested: NestedEmitter<'_>,
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
    // `after` is the ONLY buffer the nested emitter ever sees — see this
    // module's doc comment on why an HTML child of `<svg>` silently ejects
    // the rest of the drawing from it.
    let mut after = String::new();
    emit_elems(out, &mut after, elems, tx, ty, nested);
    out.push_str("</g>\n</svg>\n");
    out.push_str(&after);
}

/// One drawing as a SELF-CONTAINED `<svg>`, on a single line, in normal flow.
///
/// [`emit_graphics`] is not usable for this: it writes `position:absolute`
/// with a `left`/`top` computed against an ancestor the reflow backend makes
/// `position:relative`, and a Markdown file has no such ancestor — the
/// drawing would be positioned against the page and land on top of the text.
/// So this one carries no positioning at all and is sized to the drawing
/// itself.
///
/// The viewport is the INK's bounding box, not the box's. A graphics box is
/// routinely far larger than what it draws (a full-measure wrapper around a
/// short rule), and a viewBox taken from the box would surround every figure
/// with its own width in empty space.
///
/// Single-line by construction. A Markdown paragraph is one line, and a raw
/// `<svg>` broken across lines would have blank lines and `nl2br` inserted
/// into the middle of it by the reader's own parser. On its own line it
/// satisfies CommonMark's HTML-block rule 7 and is passed through whole; used
/// mid-paragraph it is inline HTML, which is equally valid.
///
/// `None` when the drawing has no bounding box (nothing to draw).
///
/// `GraphicsElem::Text` sub-boxes are DROPPED here — they are HTML, and an
/// HTML child ejects the remainder of the drawing from the `<svg>` (see this
/// module's own doc comment). The caller is responsible for their text; see
/// `markdown::inline`'s `Graphics` arm.
pub(super) fn graphics_block(elems: &[GraphicsElem]) -> Option<String> {
    // `pad_y = 0.0`, `stretch = false`: byte-for-byte what this always was.
    // The caller that wants the other settings is [`graphics_background`].
    graphics_svg(elems, 0.0, false).map(|(svg, _, _)| svg)
}

/// The same self-contained one-line `<svg>` as [`graphics_block`], but built
/// to be RESIZED into a box of someone else's choosing, and returning the
/// geometry that makes that placeable: `(svg, (x0, y0), (x1, y1))` — the ink
/// bounding box in the caller's own box-local, y-**up** coordinates, with
/// `pad_y` already added to its bottom and top.
///
/// The box comes back because a caller that places the drawing somewhere other
/// than "here, in flow" needs to know how big the ink is and where it sat
/// relative to the box it came out of, and the crop that makes this `<svg>`
/// tight is exactly what throws that away. `reflow::structure`'s
/// `inline_frame_decoration` reads it to tell an underline from a highlight
/// panel, and to tile at the drawing's own width.
///
/// `pad_y` inflates the viewport at the top and the bottom. A `<svg>` clips to
/// its viewport, and `graphics_bbox` measures a path's CONTROL points, not its
/// STROKE — so a 0.5pt stroke along the box's own edge loses its outer 0.25pt.
/// On a full-page figure that is invisible; on `railway`'s 1.5pt-tall wave it
/// flattens the crest and the trough.
///
/// There is deliberately no horizontal counterpart. The caller TILES a rule
/// (`inline_frame_decoration`), and empty margin on the left and right of the
/// tile is a visible GAP at every repeat — measured on a 1pt overline, where
/// the 0.5pt cap allowance either side punched a 1pt hole in the rule every
/// 78pt. What it would buy is the outer half of a stroke's end CAP at the
/// drawing's two extreme ends, which is a quarter of a point at the two places
/// a reader is least likely to look.
pub(super) fn graphics_background(elems: &[GraphicsElem], pad_y: f64) -> Option<MeasuredSvg> {
    graphics_svg(elems, pad_y, true)
}

/// The shared body of [`graphics_block`] and [`graphics_background`].
///
/// `stretch` writes `preserveAspectRatio="none"`, and it is not cosmetic: an
/// SVG asked to fill a box whose aspect ratio differs from its own defaults to
/// `xMidYMid meet`, which scales it UNIFORMLY to fit and CENTRES it. That was
/// measured, on the way here: a 92pt x 2pt wavy underline given a
/// `background-size: 100% 2pt` over a 300pt line came out 92pt long, floating
/// in the middle of the line, rather than stretched across it. It is the same
/// attribute, for the same reason, `reflow::structure`'s `retarget_svg` writes
/// for a block frame — inert where the target box happens to have the
/// drawing's own aspect ratio (a tiled rule), load-bearing where it does not
/// (a stretched panel).
fn graphics_svg(elems: &[GraphicsElem], pad_y: f64, stretch: bool) -> Option<MeasuredSvg> {
    let ((lo_x, lo_y), (hi_x, hi_y)) = elems
        .iter()
        .filter_map(rustyfi_backend::graphics_bbox)
        .reduce(|(alo, ahi), (blo, bhi)| {
            (
                (
                    rustyfi_backend::Length(alo.0 .0.min(blo.0 .0)),
                    rustyfi_backend::Length(alo.1 .0.min(blo.1 .0)),
                ),
                (
                    rustyfi_backend::Length(ahi.0 .0.max(bhi.0 .0)),
                    rustyfi_backend::Length(ahi.1 .0.max(bhi.1 .0)),
                ),
            )
        })?;
    let (lo_x, lo_y) = (lo_x.0, lo_y.0 - pad_y);
    let (hi_x, hi_y) = (hi_x.0, hi_y.0 + pad_y);
    let (w, h) = (hi_x - lo_x, hi_y - lo_y);
    if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
        return None;
    }
    let mut out = String::new();
    let par = if stretch {
        " preserveAspectRatio=\"none\""
    } else {
        ""
    };
    let _ = write!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" class=\"gfx\" role=\"img\"{par} \
         width=\"{w}pt\" height=\"{h}pt\" viewBox=\"0 0 {w} {h}\">\
         <g transform=\"translate({},{}) scale(1,-1)\">",
        -lo_x, hi_y,
    );
    // Both `after` and the nested emitter are discarded: see the note above.
    let mut after = String::new();
    let mut nested = |_: &mut String, _: &PureHorzBox, _: f64, _: f64| {};
    emit_elems(&mut out, &mut after, elems, 0.0, 0.0, &mut nested);
    out.push_str("</g></svg>");
    // `emit_elems` ends each element with a newline; a Markdown paragraph is
    // one line, so they are folded out rather than left to be reflowed by
    // whatever reads the file.
    Some((out.replace('\n', ""), (lo_x, lo_y), (hi_x, hi_y)))
}

/// The widest stroke anywhere in `elems`, HALVED — a stroke is centred on its
/// path, so this is how far its ink reaches outside the path's own bounding
/// box, and hence the [`graphics_background`] `pad_y` that keeps it.
///
/// A `Fill` contributes nothing: its ink is bounded by the path exactly.
pub(super) fn max_stroke_overhang(elems: &[GraphicsElem]) -> f64 {
    elems
        .iter()
        .map(|elem| match elem {
            GraphicsElem::Stroke(w, _, _) | GraphicsElem::DashedStroke(w, _, _, _) => w.0 / 2.0,
            GraphicsElem::Group(inner) | GraphicsElem::Clip(_, inner) => max_stroke_overhang(inner),
            _ => 0.0,
        })
        .fold(0.0, f64::max)
}

/// Percent-encode `svg` for a CSS `url("data:image/svg+xml,…")`.
///
/// Only the five characters that would actually break out are touched — `%`
/// (first, or it would double-encode the escapes that follow), `"` (which ends
/// the `url()` string), `#` (which would start a fragment identifier and cut
/// the document in half), and `<`/`>` (which no browser objects to inside a
/// `url()` but which no longer look like markup to anything that greps the
/// stylesheet). Everything else — including the spaces and the path data's
/// commas and minus signs — is left legible.
///
/// Base64 was the alternative, and `crate::base64` is already here for the
/// `<img>` data URIs. It is rejected for this one: a wavy underline over a
/// paragraph is tens of kilobytes of path data, base64 adds a third to that,
/// and the result is unreadable in the output and untestable without a
/// decoder. The `<img>` case has no choice — those are binary.
pub(super) fn svg_data_uri(svg: &str) -> String {
    let mut out = String::with_capacity(svg.len() + svg.len() / 8 + 24);
    out.push_str("data:image/svg+xml,");
    for ch in svg.chars() {
        match ch {
            '%' => out.push_str("%25"),
            '"' => out.push_str("%22"),
            '#' => out.push_str("%23"),
            '<' => out.push_str("%3C"),
            '>' => out.push_str("%3E"),
            _ => out.push(ch),
        }
    }
    out
}

/// The recursive element walker (`Group`/`Clip` reenter this, not
/// [`emit_graphics`], so a nested container never gets its own `<svg>`
/// wrapper — exactly `place_graphics`'s own `Group`/`Clip` arms, which
/// recurse into itself, not `page_content`'s `q`/`cm` prologue).
///
/// `out` is the `<svg>`'s own content; `after` is what [`emit_graphics`]
/// will append AFTER the `</svg>`, and is the only place a
/// `GraphicsElem::Text`'s nested HTML may go.
fn emit_elems(
    out: &mut String,
    after: &mut String,
    elems: &[GraphicsElem],
    tx: f64,
    ty: f64,
    nested: NestedEmitter<'_>,
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
            // `draw-text` (roadmap C1 upstream; see this module's doc
            // comment on why this is the one arm that steps OUTSIDE the
            // local `<g>` frame): re-enter the writer's own per-box emission
            // at PAGE-absolute coordinates `(tx + pt.x + dx, ty - pt.y)`.
            GraphicsElem::Text { pt, contents, .. } => {
                for (dx, bx) in contents {
                    let page_x = tx + (pt.0 + *dx).0;
                    let page_y = ty - pt.1 .0;
                    nested(after, bx, page_x, page_y);
                }
            }
            // L5b (prim-retype-sweep.md §3.3): 0.1's `graphics` collection
            // container nodes — never reached by any 0.0.6 program (see
            // `GraphicsElem`'s own doc comment), handled anyway for parity
            // with the PDF writer's exhaustive match (no wildcard arm there
            // either). A bare `<g>` — no transform of its own, since the
            // enclosing `<g transform>` from `emit_graphics` (or an outer
            // `Group`/`Clip`) already applies.
            GraphicsElem::Group(inner) => {
                out.push_str("<g>\n");
                emit_elems(out, after, inner, tx, ty, nested);
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
                emit_elems(out, after, inner, tx, ty, nested);
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

/// Process-global monotonic counter for `<clipPath id>` uniqueness (SVG IDs
/// must be unique within one document; a document can contain arbitrarily
/// many independent `Clip` elements, and the box walker has no other natural
/// "clip index" to thread through).
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

/// One glyph's own outline as an SVG `d` attribute value, in the face's
/// **design units** and y-**up** — i.e. the glyph exactly as the font draws
/// it, with no scaling applied. The caller supplies the `size/units_per_em`
/// scale and the y-flip through a per-element `transform`
/// (`reflow::inline::emit_math_svg`), which keeps this function a pure
/// font-to-path conversion and the numbers in `d` the small integers the
/// face actually stores.
///
/// **Why a path at all, when every other glyph in this backend is `<text>`.**
/// A `MathGlyph` carrying `Some(gid)` is one whose drawn form is NOT the
/// glyph its `text` cmaps to: an OpenType MATH `MathVariants` record (a
/// display-size big operator, a stretchy delimiter, one part of a
/// `GlyphAssembly`) or an `ssty` script form. `<text>∑</text>` can only ever
/// address the character, so it drew the BASE `∑` where the document had
/// placed the display one — and, because the surrounding layout was computed
/// against the variant's metrics, everything positioned relative to it landed
/// off-centre too. See [`crate::reflow`]'s `emit_math_svg` for the
/// measurement.
///
/// Returns `None` when the face exposes no outline for `gid` (a bitmap-only
/// face, or a glyph with no contours), leaving the caller on its `<text>`
/// path — which is what this backend did for every such glyph before.
pub(super) fn glyph_outline_d(face: &ttf_parser::Face<'_>, gid: u16) -> Option<String> {
    let mut b = OutlineToPath::default();
    face.outline_glyph(ttf_parser::GlyphId(gid), &mut b)?;
    let d = b.d.trim_end().to_string();
    (!d.is_empty()).then_some(d)
}

/// [`glyph_outline_d`]'s `ttf_parser::OutlineBuilder` sink: the four outline
/// callbacks map one-for-one onto SVG's `M`/`L`/`Q`/`C`/`Z`, so this is a
/// transcription rather than a conversion. Quadratics are kept as `Q` rather
/// than elevated to cubics — SVG has the segment natively, and a TrueType
/// face is nothing but quadratics.
#[derive(Default)]
struct OutlineToPath {
    d: String,
}

impl ttf_parser::OutlineBuilder for OutlineToPath {
    fn move_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.d, "M{x} {y} ");
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.d, "L{x} {y} ");
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let _ = write!(self.d, "Q{x1} {y1} {x} {y} ");
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let _ = write!(self.d, "C{x1} {y1} {x2} {y2} {x} {y} ");
    }
    fn close(&mut self) {
        self.d.push_str("Z ");
    }
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
