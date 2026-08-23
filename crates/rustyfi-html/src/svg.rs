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
//! **`GraphicsElem::Text` breaks that decomposition on purpose.** A
//! `draw-text` run's placed sub-boxes are ordinary `PureHorzBox`es (text
//! runs, possibly nested graphics/images) emitted through the SAME
//! `emit_box` used everywhere else, which positions `<span>`/`<svg>`
//! children via CSS `position:absolute` relative to the nearest positioned
//! ancestor — the `.page` div, NOT this box's local `<g transform>` (CSS
//! absolute positioning does not compose with an SVG sibling's coordinate
//! transform the way a PDF content-stream operator composes with the active
//! CTM). So `Text` is handled OUTSIDE the `<svg>`/`<g>` nest entirely: its
//! nested boxes are placed at page-absolute coordinates computed by hand
//! (`tx + pt.x + dx`, `ty - pt.y`) via the `nested` callback, the one
//! documented divergence from the PDF writer's `place_graphics` (whose
//! `emit_nested` callback runs INSIDE the `q`/`cm` block precisely because
//! PDF text ops DO compose with the CTM).

use rustyfi_backend::{Closing, Color, GraphicsElem, Path, PathSeg, PureHorzBox};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The nested-emitter callback type — the `&mut String` analogue of
/// `rustyfi-pdf`'s `NestedEmitter` (`lib.rs:681`). [`emit_graphics`] invokes
/// this only for `GraphicsElem::Text`'s sub-boxes (see this module's doc
/// comment on why they can't stay inside the SVG's local coordinate frame);
/// every other variant is handled directly by this module.
pub(super) type NestedEmitter<'a> = &'a mut dyn FnMut(&mut String, &PureHorzBox, f64, f64);

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
    emit_elems(out, elems, tx, ty, nested);
    out.push_str("</g>\n</svg>\n");
}

/// The recursive element walker (`Group`/`Clip` reenter this, not
/// [`emit_graphics`], so a nested container never gets its own `<svg>`
/// wrapper — exactly `place_graphics`'s own `Group`/`Clip` arms, which
/// recurse into itself, not `page_content`'s `q`/`cm` prologue).
fn emit_elems(
    out: &mut String,
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
                    nested(out, bx, page_x, page_y);
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
                emit_elems(out, inner, tx, ty, nested);
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
                emit_elems(out, inner, tx, ty, nested);
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
