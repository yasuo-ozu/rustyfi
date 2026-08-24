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
use rustyfi_pdf::TtfFontStore;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The nested-emitter callback type — the `&mut String` analogue of
/// `rustyfi-pdf`'s own `NestedEmitter` (`lib.rs`). [`emit_graphics`] invokes
/// this only for `GraphicsElem::Text`'s sub-boxes (see this module's doc
/// comment on why they can't stay inside the SVG's local coordinate frame);
/// every other variant is handled directly by this module.
pub(super) type NestedEmitter<'a> =
    &'a mut dyn FnMut(&mut String, &PureHorzBox, f64, f64, Option<CssMatrix>);

/// A `GraphicsElem::Text`'s 2×2 linear transform, restated for CSS: the
/// `matrix(A, B, C, D, 0, 0)` a caller writes to reproduce it, about the run's
/// own anchor. `None` everywhere the run is upright.
///
/// It is a CHANGE OF BASIS away from what `graphics.rs` stores, not a copy of
/// it. The stored matrix is row-major `(a, b, c, d)` acting on the box-local,
/// y-**UP** point `(x, y) |-> (ax + by, cx + dy)`
/// (`graphicBase.ml`'s `linear_transform_point`); CSS acts on a y-**DOWN**
/// point. Substituting `Y = -y` into the y-up map and solving for `Y'` gives
///
/// ```text
/// X' =  a ·X + (-b)·Y
/// Y' = (-c)·X +   d ·Y      i.e. matrix(a, -c, -b, d, 0, 0)
/// ```
///
/// — a flip conjugation, `S M S` with `S = diag(1, -1)`, which for a rotation
/// is just the sign of the angle (a counter-clockwise turn in the port's y-up
/// frame is `rotate(-θ)` on a y-down screen, which is the SAME turn visually).
/// Getting this wrong mirrors the run rather than failing, so it is spelled
/// out here rather than inlined at the one call site that builds it.
pub(super) type CssMatrix = (f64, f64, f64, f64);

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
/// **`GraphicsElem::Text` sub-boxes become SVG `<text>`**, drawn by
/// [`emit_text_layer`] alongside the paths. This is what the comment here used
/// to say, and it was wrong in its conclusion:
///
/// > sub-boxes are DROPPED here — they are HTML, and an HTML child ejects the
/// > remainder of the drawing from the `<svg>`. The caller is responsible for
/// > their text.
///
/// The premise is right and still holds — an HTML `<span>` inside `<svg>` ends
/// the parser's foreign-content mode and throws the rest of the drawing out of
/// the document (this module's own doc comment) — but the caller never took
/// that responsibility, and could not sensibly have: `markdown::inline`'s
/// `Graphics` arm has the drawing as one opaque `<svg>` string with nowhere to
/// put text that belongs INSIDE it. So every `draw-text` in a Markdown figure
/// vanished. Measured on `figbox`: `\fig-inline(textbox {boxed} |> hvmargin
/// 3pt |> frame …)` emitted a rectangle and no `boxed`, and a `+fig-center` of
/// two framed captions emitted two rectangles and neither caption.
///
/// `<text>` is the answer the premise actually points at: it is SVG, not HTML,
/// so it composes with the drawing instead of ending it, and it is real
/// selectable, searchable text.
pub(super) fn graphics_block(elems: &[GraphicsElem], fonts: Option<&TtfFontStore>) -> Option<String> {
    // `pad_y = 0.0`, `stretch = false`: byte-for-byte what this always was.
    // The caller that wants the other settings is [`graphics_background`].
    graphics_svg(elems, 0.0, false, true, fonts).map(|(svg, _, _)| svg)
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
///
/// No `<text>` layer, unlike [`graphics_block`]: this drawing is a
/// DECORATION, replayed under text the reflow backend is writing as real HTML
/// a few characters later, and a `draw-text` inside one would print the same
/// words a second time. (Nothing in the corpus puts one there; the argument is
/// why it stays that way rather than why it cannot happen.)
pub(super) fn graphics_background(elems: &[GraphicsElem], pad_y: f64) -> Option<MeasuredSvg> {
    graphics_svg(elems, pad_y, true, false, None)
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
///
/// `text` writes the `<text>` layer ([`emit_text_layer`]) after the paths;
/// `fonts` only names families within it, so the two are separate — a
/// base-14 render has no store and still has words to draw.
fn graphics_svg(
    elems: &[GraphicsElem],
    pad_y: f64,
    stretch: bool,
    text: bool,
    fonts: Option<&TtfFontStore>,
) -> Option<MeasuredSvg> {
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
         width=\"{w}pt\" height=\"{h}pt\" viewBox=\"0 0 {w} {h}\">",
    );
    emit_flipped_group(&mut out, elems, -lo_x, hi_y);
    // The `<text>` layer goes OUTSIDE that group, and that is not a style
    // choice: SVG `<text>` is not orientation-independent the way a filled
    // path is, so a glyph inside `scale(1,-1)` renders MIRRORED upside-down.
    // `TextFrame::text_pen` therefore maps box-local y-up to SVG-native
    // y-down by hand, exactly as `mathsvg`'s `pen` does for a math glyph.
    if text {
        emit_text_layer(&mut out, elems, lo_x, hi_y, fonts);
    }
    out.push_str("</svg>");
    // `emit_elems` ends each element with a newline; a Markdown paragraph is
    // one line, so they are folded out rather than left to be reflowed by
    // whatever reads the file.
    Some((out.replace('\n', ""), (lo_x, lo_y), (hi_x, hi_y)))
}

/// Every `draw-text` run in `elems`, written as SVG `<text>` at the position
/// the document placed it — the layer that keeps a Markdown figure's words.
///
/// `(lo_x, hi_y)` is the viewport crop [`graphics_svg`] computed, so a
/// box-local y-**up** point `(px, py)` lands at SVG-native
/// `(px - lo_x, hi_y - py)`. That mapping is [`text_pen`], the direct analogue
/// of `reflow::inline`'s math `pen`, and it is done by hand for the same
/// reason: `<text>` may not sit inside the `scale(1,-1)` group, which would
/// mirror it.
///
/// **What it can place.** A run's `contents` are ordinary `PureHorzBox`es, so
/// this is a small box walker: text runs, the two frame kinds, an embedded
/// block (whose inner lines it places with `place_block_at`, the same call
/// `rustyfi-pdf`'s `place_embedded_block` makes), a table's cells, a math
/// run's glyphs, and a nested drawing's own `draw-text`s. Anything else
/// contributes no text and is skipped.
///
/// **What it does not.** A nested `Graphics`' own PATHS are not drawn — a
/// drawing inside a `draw-text` inside a drawing. That gap is older than this
/// function and unchanged by it: `emit_elems` has never descended into a
/// run's sub-boxes either, so those paths were already missing. Only the text
/// is recovered here.
pub(super) fn emit_text_layer(
    out: &mut String,
    elems: &[GraphicsElem],
    lo_x: f64,
    hi_y: f64,
    fonts: Option<&TtfFontStore>,
) {
    let f = TextFrame {
        lo_x,
        hi_y,
        fonts,
        pending: None,
    };
    let mut f = f;
    walk_text_elems(out, elems, &mut f);
    f.flush(out);
}

/// The viewport mapping and font store [`emit_text_layer`]'s walk carries
/// down, plus the one piece of state it keeps: the `<text>` currently open for
/// APPENDING — see [`TextFrame::push`].
struct TextFrame<'a> {
    lo_x: f64,
    hi_y: f64,
    fonts: Option<&'a TtfFontStore>,
    pending: Option<Pending>,
}

/// A `<text>` that has been started and may still grow. `end_x`/`y` are in the
/// caller's box-local y-**up** frame, so the next box can be tested for being
/// exactly where this one ends.
struct Pending {
    px: f64,
    py: f64,
    end_x: f64,
    y: f64,
    style: RunStyle,
    text: String,
}

/// Everything about a run that is NOT its position: the CSS declarations and
/// the enclosing `draw-text`'s transform. Together because they are together
/// the JOIN KEY — two runs may share a `<text>` only if both match — and
/// because keeping them as one value is what holds [`TextFrame::push`] inside
/// clippy's argument limit.
#[derive(PartialEq)]
struct RunStyle {
    css: String,
    mat: Option<CssMatrix>,
}

impl TextFrame<'_> {
    /// A box-local y-**up** point as SVG-native `(x, y)`. `y` is a BASELINE:
    /// `<text>`'s default `dominant-baseline` is the alphabetic one, which is
    /// exactly what a `PureHorzBox`'s own anchor means.
    fn text_pen(&self, x: f64, y: f64) -> (f64, f64) {
        (x - self.lo_x, self.hi_y - y)
    }

    fn family(&self, font: rustyfi_backend::FontKey) -> Option<String> {
        let store = self.fonts?;
        let family = store.file_family_name(store.file_index(font))?;
        Some(crate::fonts::reflow_font_stack(&family))
    }

    /// Add one text run, EXTENDING the open `<text>` when this run begins
    /// exactly where that one ended, on the same baseline, in the same style.
    ///
    /// Without the join a hyphenation point cuts a word in two. The line
    /// breaker offers one inside almost every long word — `left|ward`,
    /// `cap|tion` — and a `Discretionary` it did NOT take still stands in the
    /// box stream between two `InnerString`s. Emitted as two `<text>`s the
    /// word renders correctly (the halves abut) and is nonetheless broken for
    /// every purpose that reads it: selection, find-in-page, a grep of the
    /// file, a screen reader. The measured symptom was a `figbox` caption that
    /// no search for `caption` could find, because the file said `cap` and
    /// `tion`.
    ///
    /// The join is on POSITION, not on the discretionary: `x == end_x` to
    /// within a rounding tolerance means there is no gap, whatever stood
    /// between. A real word space fails the test and starts a new `<text>`,
    /// which keeps the words at the x the document chose rather than at
    /// wherever the reader's own space width would put them.
    fn push(
        &mut self,
        out: &mut String,
        text: &str,
        x: f64,
        y: f64,
        width: f64,
        style: RunStyle,
    ) {
        // A quarter of a point: below a device pixel at any sane zoom, and far
        // above the accumulated error of summing box widths.
        const JOIN_EPS: f64 = 0.25;
        if let Some(p) = &mut self.pending {
            if (x - p.end_x).abs() < JOIN_EPS && p.y == y && p.style == style {
                p.text.push_str(text);
                p.end_x = x + width;
                return;
            }
        }
        self.flush(out);
        let (px, py) = self.text_pen(x, y);
        self.pending = Some(Pending {
            px,
            py,
            end_x: x + width,
            y,
            style,
            text: text.to_string(),
        });
    }

    /// Write the open `<text>`, if any.
    ///
    /// `font-size` is written in `px`, which SVG fixes at one user unit, and
    /// this viewport's user unit is one point — so a 12pt run comes out 12pt.
    /// Spelling it `12pt` would be 16 user units (see `reflow::inline`'s
    /// `math_font_size_uu`, which measured all four spellings), and a bare
    /// `12` is invalid CSS that Gecko drops.
    ///
    /// `xml:space="preserve"` is deliberate: a run may legitimately begin or
    /// end in a space, and XML's default collapses it.
    ///
    /// A TRANSFORMED run is placed by the transform rather than by `x`/`y`:
    /// SVG's `transform` is about the element's own user-space origin, so the
    /// pen position becomes the translate and the matrix follows it. The
    /// matrix is the same y-down conversion [`CssMatrix`] documents, SVG user
    /// space being y-down as CSS is.
    fn flush(&mut self, out: &mut String) {
        let Some(p) = self.pending.take() else {
            return;
        };
        let (px, py, style) = (p.px, p.py, &p.style.css);
        let body = crate::escape_html(&p.text);
        match p.style.mat {
            Some((a, b, c, d)) => {
                let _ = write!(
                    out,
                    "<text transform=\"translate({px},{py}) matrix({a},{},{},{d},0,0)\" \
                     x=\"0\" y=\"0\" xml:space=\"preserve\" style=\"{style}\">{body}</text>",
                    -c, -b,
                );
            }
            None => {
                let _ = write!(
                    out,
                    "<text x=\"{px}\" y=\"{py}\" xml:space=\"preserve\" \
                     style=\"{style}\">{body}</text>",
                );
            }
        }
    }

    /// The [`RunStyle`] one run wants.
    fn run_style(
        &self,
        size: f64,
        font: rustyfi_backend::FontKey,
        color: Color,
        mat: Option<CssMatrix>,
    ) -> RunStyle {
        let mut css = format!("font-size:{size}px;");
        if let Some(stack) = self.family(font) {
            let _ = write!(css, "font-family:{stack};");
        }
        if color != Color::Gray(0.0) {
            let _ = write!(css, "fill:{};", css_color(color));
        }
        RunStyle { css, mat }
    }
}

fn walk_text_elems(out: &mut String, elems: &[GraphicsElem], f: &mut TextFrame) {
    for elem in elems {
        match elem {
            // The same per-box offset arithmetic the nested-HTML emitter does
            // (`emit_elems`' own `Text` arm) — a transformed run's boxes are
            // offset by the TRANSFORMED `(dx, 0)`.
            GraphicsElem::Text {
                pt,
                contents,
                transform,
                ..
            } => {
                for (dx, bx) in contents {
                    let (ox, oy) = match transform {
                        Some((a, _, c, _)) => (a * dx.0, c * dx.0),
                        None => (dx.0, 0.0),
                    };
                    place_text_box(out, bx, pt.0 .0 + ox, pt.1 .0 + oy, *transform, f);
                }
            }
            GraphicsElem::Group(inner) | GraphicsElem::Clip(_, inner) => {
                walk_text_elems(out, inner, f)
            }
            _ => {}
        }
    }
}

/// One placed sub-box's text, at box-local y-up `(x, y)` — `x` its left edge,
/// `y` its baseline. `mat` is the enclosing run's transform, carried down so a
/// rotated `\rotatebox`/`figbox` run turns its words rather than leaving them
/// upright in the middle of a turned drawing.
fn place_text_box(
    out: &mut String,
    bx: &PureHorzBox,
    x: f64,
    y: f64,
    mat: Option<(f64, f64, f64, f64)>,
    f: &mut TextFrame,
) {
    match bx {
        PureHorzBox::InnerString {
            info, text, width, ..
        } => {
            if text.is_empty() {
                return;
            }
            let style = f.run_style(info.size.0, info.font, info.color, mat);
            f.push(out, text, x, y + info.rising.0, width.0, style);
        }
        // Both frame kinds hold their contents on their own baseline, with
        // x-offsets from the frame's left edge (`PureHorzBox::Frame`'s doc
        // comment).
        PureHorzBox::Frame { contents, .. } => {
            for (dx, cbx) in contents {
                place_text_box(out, cbx, x + dx.0, y, mat, f);
            }
        }
        // `place_block_at` + the anchor line, exactly as `rustyfi-pdf`'s
        // `place_embedded_block` resolves it: the FIRST inner line's baseline
        // coincides with the box's own for `embed-block-top`, the LAST for
        // `-bottom`, and every other line is offset by the difference of their
        // placed baselines (a larger `baseline_y` is LOWER, hence the
        // subtraction in this y-up frame).
        PureHorzBox::EmbeddedBlock {
            block, anchor_last, ..
        } => {
            let placed = rustyfi_backend::place_block_at(
                (rustyfi_backend::Length::ZERO, rustyfi_backend::Length::ZERO),
                block.clone(),
            );
            let anchor = if *anchor_last {
                placed.last()
            } else {
                placed.first()
            };
            let Some(anchor) = anchor.map(|l| l.baseline_y) else {
                return;
            };
            for line in &placed {
                let ly = y - (line.baseline_y - anchor).0;
                for (dx, cbx) in &line.contents {
                    place_text_box(out, cbx, x + line.x.0 + dx.0, ly, mat, f);
                }
            }
        }
        // A cell's `baseline_y` is measured UPWARD from the tabular box's own
        // baseline, the same convention `rustyfi-lang`'s `fire_inline_frame`
        // reads it in.
        PureHorzBox::Tabular(tab) => {
            for cell in &tab.cells {
                for (dx, cbx) in &cell.contents {
                    place_text_box(out, cbx, x + cell.x.0 + dx.0, y + cell.baseline_y.0, mat, f);
                }
            }
        }
        // A math run inside a drawing — `figbox`'s `textbox {${…}}`. Its
        // glyphs carry box-local y-up offsets from the run's baseline, the
        // same convention everything else here uses; its `rules` (fraction
        // bars, radicals) are paths and are NOT drawn, for the same reason a
        // nested drawing's paths are not (see [`emit_text_layer`]).
        PureHorzBox::Math { glyphs, .. } => {
            for g in glyphs {
                if g.text.is_empty() {
                    continue;
                }
                let style = f.run_style(g.info.size.0, g.info.font, g.info.color, mat);
                // A math glyph carries no advance of its own here — every one
                // is positioned absolutely — so its run "ends" where it began
                // and the next glyph never joins it. That is what the layout
                // asked for: inter-atom spacing in an equation is not a font's
                // advance width.
                f.push(out, &g.text, x + g.dx.0, y + g.dy.0 + g.info.rising.0, 0.0, style);
            }
        }
        // A drawing nested inside a run: its own `draw-text`s are in ITS
        // box-local frame, anchored here — which is a shift of the viewport
        // mapping, not a new one, so the join state travels with it.
        PureHorzBox::Graphics { elems, .. } => {
            let (lo_x, hi_y) = (f.lo_x, f.hi_y);
            f.flush(out);
            f.lo_x = lo_x - x;
            f.hi_y = hi_y - y;
            walk_text_elems(out, elems, f);
            f.flush(out);
            f.lo_x = lo_x;
            f.hi_y = hi_y;
        }
        _ => {}
    }
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

/// `elems` as one `<g>` that carries the y-up-to-y-down flip, with no `<svg>`
/// of its own — the reusable inside of [`graphics_block`], for a caller that
/// already has a viewport open.
///
/// Split out for [`crate::mathsvg::math_block`], which draws a math run's
/// fraction bars and radical signs INSIDE the same `<svg>` as the glyph
/// outlines. It cannot use [`emit_graphics`] for that: that helper opens an
/// `<svg>` of its own and positions it absolutely, which is right for the
/// reflow backend's positioned wrapper and wrong for a self-contained one.
///
/// `(tx, ty)` is the translate the flip is composed with — `(0, height)` to
/// put a box-local origin on the box's baseline, or `(-lo_x, hi_y)` to fit a
/// drawing's own bounding box. A local point `(px, py)` lands at
/// `(tx + px, ty - py)`.
///
/// `GraphicsElem::Text` sub-boxes DRAW NOTHING here, and must not: this group
/// carries the y-flip, and SVG `<text>` inside `scale(1,-1)` renders mirrored
/// upside-down. Their text is a SEPARATE layer, written by the caller outside
/// this group — [`emit_text_layer`], which [`graphics_block`] calls straight
/// after this. (An HTML `<span>` here would be worse still: it ends the
/// parser's foreign-content mode and ejects the rest of the drawing from the
/// `<svg>` — see this module's own doc comment.)
pub(super) fn emit_flipped_group(out: &mut String, elems: &[GraphicsElem], tx: f64, ty: f64) {
    let _ = write!(out, "<g transform=\"translate({tx},{ty}) scale(1,-1)\">");
    emit_elems_simple(out, elems);
    out.push_str("</g>");
}

/// [`emit_elems`] with no nested-box emitter and no `after` buffer — the walk
/// for a caller that has already opened its own coordinate frame and has no
/// HTML to place (`GraphicsElem::Text` sub-boxes draw nothing, as in
/// [`emit_flipped_group`]; their text is [`emit_text_layer`]'s).
///
/// Exposed for `crate::mathsvg`'s `--svg-math` rule emitter, which draws the
/// shapes it can name itself and delegates the rest here rather than
/// reimplementing clips and dash patterns.
pub(super) fn emit_elems_simple(out: &mut String, elems: &[GraphicsElem]) {
    // Both `after` and the nested emitter are discarded: see the note above.
    let mut after = String::new();
    let mut nested = |_: &mut String, _: &PureHorzBox, _: f64, _: f64, _: Option<CssMatrix>| {};
    emit_elems(out, &mut after, elems, 0.0, 0.0, &mut nested);
}

/// `path` as `(x, y, width, height)` when it is a single axis-aligned
/// rectangle, in the path's own y-up coordinates.
///
/// A fraction bar, a radical's overbar and every `\overline`/`\underline` in
/// the corpus is exactly this shape — `layout_math_atom` draws them as a
/// filled quadrilateral — so recognising it turns the great majority of a math
/// box's rules into a `<rect>` a reader can understand at a glance.
///
/// Deliberately STRICT: four or five points (a closing point repeating the
/// start is accepted), every segment a straight line, and each one purely
/// horizontal or purely vertical. Anything else answers `None` and is drawn as
/// the path it is. A near-rectangle silently squared off would move ink.
pub(super) fn axis_aligned_rect(path: &Path) -> Option<(f64, f64, f64, f64)> {
    let [sub] = &path.subpaths[..] else {
        return None;
    };
    if sub.closing == Closing::Open {
        return None;
    }
    let mut pts = vec![(sub.start.0 .0, sub.start.1 .0)];
    for seg in &sub.segs {
        match seg {
            PathSeg::Line(p) => pts.push((p.0 .0, p.1 .0)),
            PathSeg::Bezier(..) => return None,
        }
    }
    // A path that returns to its start explicitly has one point too many.
    if pts.len() == 5 && pts[0] == pts[4] {
        pts.pop();
    }
    if pts.len() != 4 {
        return None;
    }
    // Every edge, including the implicit closing one, must be axis-parallel.
    for i in 0..4 {
        let (ax, ay) = pts[i];
        let (bx, by) = pts[(i + 1) % 4];
        if ax != bx && ay != by {
            return None;
        }
    }
    let xs: Vec<f64> = pts.iter().map(|p| p.0).collect();
    let ys: Vec<f64> = pts.iter().map(|p| p.1).collect();
    let (x0, x1) = (
        xs.iter().copied().fold(f64::INFINITY, f64::min),
        xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    );
    let (y0, y1) = (
        ys.iter().copied().fold(f64::INFINITY, f64::min),
        ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    );
    // Exactly two distinct x and two distinct y, or it is some other
    // quadrilateral that happens to have axis-parallel edges.
    if xs.iter().any(|v| *v != x0 && *v != x1) || ys.iter().any(|v| *v != y0 && *v != y1) {
        return None;
    }
    let (w, h) = (x1 - x0, y1 - y0);
    (w > 0.0 && h > 0.0).then_some((x0, y0, w, h))
}

/// `path` as `(x1, y1, x2, y2)` when it is one straight line segment.
///
/// The `<line>` counterpart of [`axis_aligned_rect`], for a rule the document
/// STROKED rather than filled — a user's own `\overline`-style drawing.
/// Unlike the rectangle test this accepts any direction, since a `<line>`
/// expresses a diagonal exactly as well as an axis-parallel one.
pub(super) fn straight_segment(path: &Path) -> Option<(f64, f64, f64, f64)> {
    let [sub] = &path.subpaths[..] else {
        return None;
    };
    if sub.closing != Closing::Open {
        return None;
    }
    let [PathSeg::Line(end)] = &sub.segs[..] else {
        return None;
    };
    Some((sub.start.0 .0, sub.start.1 .0, end.0 .0, end.1 .0))
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
            //
            // `transform` is the run's own 2×2 about its anchor, applied
            // BEFORE the `pt` translation (`GraphicsElem::Text`'s doc
            // comment) — `latexcmds`' `\rotatebox`, `figbox`'s `rotate`/
            // `scale`. The PDF writer pushes it as a `cm` and then emits each
            // box at its plain local `(dx, 0)` inside that frame
            // (`rustyfi-pdf/src/lib.rs:941`); there is no such enclosing frame
            // here, because these boxes leave the `<svg>` entirely, so the
            // same composition is done by hand instead:
            //
            //   translate(M·(dx, 0)) ∘ M  ==  M ∘ translate((dx, 0))
            //
            // i.e. each box is offset by the TRANSFORMED `(dx, 0)` and then
            // carries the matrix itself, about its own left-baseline point.
            // Equal by associativity, so it is the same placement the PDF
            // gets, distributed over the boxes.
            GraphicsElem::Text {
                pt,
                contents,
                transform,
                ..
            } => {
                for (dx, bx) in contents {
                    let (off_x, off_y) = match transform {
                        Some((a, _, c, _)) => (a * dx.0, c * dx.0),
                        None => (dx.0, 0.0),
                    };
                    let page_x = tx + pt.0 .0 + off_x;
                    let page_y = ty - pt.1 .0 - off_y;
                    let css = transform.map(|(a, b, c, d)| (a, -c, -b, d));
                    nested(after, bx, page_x, page_y, css);
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
pub(super) fn path_d(path: &Path) -> String {
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
