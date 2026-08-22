//! The drawing data model — paths, colors, and `graphics` elements; the
//! analog of upstream's `GraphicBase`/`PrePath`/`GraphicD`. Everything here is
//! already-resolved coordinates/data: no lang-side closure or deferred
//! computation crosses into this module.

use crate::hbox::PureHorzBox;
use crate::length::Length;

/// A point in graphics space (upstream `point`; matches the runtime
/// `Value::Tuple([Length, Length])` representation). Graphics space is
/// y-**up** (PDF-native); the PDF writer's `place_graphics` flips
/// page-layout's y-down convention when placing a graphics box on a line.
pub type Point = (Length, Length);

/// A dash pattern (`dashed-stroke`'s 2nd argument; upstream `graphicD.ml`'s
/// `type dash = length * length * length`, `(d1, d2, d0)` = on-length,
/// off-length, phase).
pub type Dash = (Length, Length, Length);

/// `color.satyh`'s `Gray`/`RGB`/`CMYK` after extraction by `as_color`
/// (mirrors `evalUtil.ml`'s `get_color` → `DeviceGray`/`DeviceRGB`/
/// `DeviceCMYK`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Color {
    Gray(f64),
    Rgb(f64, f64, f64),
    Cmyk(f64, f64, f64, f64),
}

/// One path element: a control-point-free straight segment, or a cubic
/// Bézier (2 control points + destination) — `graphicBase.ml`'s
/// `point path_element`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathSeg {
    Line(Point),
    Bezier(Point, Point, Point),
}

/// How a subpath closes (`graphicBase.ml`'s `path`'s `cycleopt`): left open,
/// closed with a straight segment back to the start (`close-with-line`), or
/// closed with a cubic (`close-with-bezier` — the destination is always the
/// subpath's own `start`, so only the two control points are stored).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Closing {
    Open,
    Line,
    Bezier(Point, Point),
}

/// One `GraphicBase.GeneralPath(start, elems, closing)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Subpath {
    pub start: Point,
    pub segs: Vec<PathSeg>,
    pub closing: Closing,
}

/// The `path` value = upstream `path list` (`unite-path` appends subpath
/// lists).
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub subpaths: Vec<Subpath>,
}

/// The `pre-path` value (`PrePath.t`): a start point plus forward-accumulated
/// segments, before a `terminate-path`/`close-with-line` fixes a closing.
/// Upstream accumulates in reverse and flips at close time; this port pushes
/// forward directly, which is unobservable.
#[derive(Clone, Debug, PartialEq)]
pub struct PrePath {
    pub start: Point,
    pub segs: Vec<PathSeg>,
}

/// One `graphics` element (`GraphicD.element`). `place_graphics`
/// (rustyfi-pdf) matches this exhaustively, without a wildcard arm.
///
/// See [`crate::hbox::PureHorzBox`] for what the `#[subast]` list means and
/// what checks it.
#[derive(Clone, Debug, PartialEq, syan::visit::Ast)]
#[subast(crate::graphics::GraphicsElem, crate::hbox::PureHorzBox)]
pub enum GraphicsElem {
    /// Filled region, even-odd rule (upstream's `op_f'`).
    Fill(Color, Path),
    /// Stroked outline at the given line width.
    Stroke(Length, Color, Path),
    /// Dashed stroked outline (`dashed-stroke`), rendered with a PDF `d`
    /// dash-array op alongside the same stroke ops as `Stroke`.
    DashedStroke(Length, Dash, Color, Path),
    /// `draw-text`: a text run anchored at `pt` (box-local, y-up; the run's
    /// leftmost baseline point). `contents` is the run laid out at NATURAL
    /// width (upstream `LineBreak.natural` = `determine_widths None`,
    /// `widperfil = 0`; here `fit_cell(boxes, natural_width)`), each box with
    /// its x offset from `pt`. `width`/`height`/`depth` are the run's
    /// `natural_metrics`, stored at construction so `graphics_bbox` needs no
    /// re-measure. Rendered by each PDF writer re-entering its own per-box
    /// emission at `pt + dx` INSIDE `place_graphics`'s box-local `cm` frame.
    Text {
        pt: Point,
        contents: Vec<(Length, PureHorzBox)>,
        width: Length,
        height: Length,
        depth: Length,
        /// The accumulated 2×2 linear transform (`linear-transform-graphics`,
        /// row-major `(a, b, c, d)` — same convention as
        /// `linear_transform_point`) applied to the run about its local
        /// origin BEFORE the `pt` translation. `None` means identity: the run
        /// is drawn upright at `pt`. `Some` appears once
        /// `rotate-graphics`/`scale-graphics` is composed onto a `draw-text`;
        /// the writer then emits the run under a `cm` carrying this matrix
        /// (upstream's lazy `LinearTrans` render-time `cm`).
        transform: Option<(f64, f64, f64, f64)>,
    },
    /// 0.1 collection node (`GraphicD.concat`, dev-0-1-0 `graphicD.ml:23`):
    /// `unite-graphics`' payload. No 0.0.6-visible primitive builds one, so it
    /// is unreachable from 0.0.6 rendering by construction.
    Group(Vec<GraphicsElem>),
    /// 0.1 clip node (`GraphicD.make_clip`, `graphicD.ml:97-98`): render
    /// `contents` clipped to `clip` (even-odd, `Op_W'` — `graphicD.ml:331`).
    /// The port's `Path` already carries N subpaths, standing in for
    /// upstream's `path list`. Never constructed by any 0.0.6 path, as `Group`.
    Clip(Path, Vec<GraphicsElem>),
}

// `shift-path`/`shift-graphics`/`linear-transform-path`/
// `linear-transform-graphics` are all EAGER point remaps — no lazy
// `LinearTrans`-wrapper element: every point is rewritten up front, mirroring
// `graphicBase.ml`'s `shift_path`/`linear_transform_path` (`(x, y) ->
// (x*a + y*b, x*c + y*d)` for the 2x2 matrix `((a, b), (c, d))`).

/// `shift_path v pt` (`graphicBase.ml`'s `(+@%)`).
fn shift_point(v: Point, pt: Point) -> Point {
    (pt.0 + v.0, pt.1 + v.1)
}

/// `graphicBase.ml`'s `linear_transform_point`: `(x, y) |-> (x*a + y*b, x*c +
/// y*d)` for matrix `mat = (a, b, c, d)`.
fn linear_transform_point(mat: (f64, f64, f64, f64), pt: Point) -> Point {
    let (a, b, c, d) = mat;
    (pt.0 * a + pt.1 * b, pt.0 * c + pt.1 * d)
}

/// Map `f` over every point of `path` (subpath starts, every segment's
/// points — including Bézier control points — and any closing control
/// points), preserving structure.
fn map_path(path: &Path, f: impl Fn(Point) -> Point) -> Path {
    Path {
        subpaths: path
            .subpaths
            .iter()
            .map(|sub| Subpath {
                start: f(sub.start),
                segs: sub
                    .segs
                    .iter()
                    .map(|seg| match *seg {
                        PathSeg::Line(p) => PathSeg::Line(f(p)),
                        PathSeg::Bezier(c1, c2, p) => PathSeg::Bezier(f(c1), f(c2), f(p)),
                    })
                    .collect(),
                closing: match sub.closing {
                    Closing::Open => Closing::Open,
                    Closing::Line => Closing::Line,
                    Closing::Bezier(c1, c2) => Closing::Bezier(f(c1), f(c2)),
                },
            })
            .collect(),
    }
}

/// `shift-path : point -> path -> path` (vminst.ml:663) — translate every
/// point of `path` by `v`.
pub fn shift_path(v: Point, path: &Path) -> Path {
    map_path(path, |p| shift_point(v, p))
}

/// `linear-transform-path : float -> float -> float -> float -> path ->
/// path` (vminst.ml:678) — apply the 2x2 matrix `mat` to every point.
pub fn linear_transform_path(mat: (f64, f64, f64, f64), path: &Path) -> Path {
    map_path(path, |p| linear_transform_point(mat, p))
}

/// `shift-graphics : point -> graphics -> graphics` (vminst.ml:2451) —
/// `graphicD.ml`'s `shift_element`.
pub fn shift_graphics(v: Point, elem: &GraphicsElem) -> GraphicsElem {
    match elem {
        GraphicsElem::Fill(c, p) => GraphicsElem::Fill(*c, shift_path(v, p)),
        GraphicsElem::Stroke(w, c, p) => GraphicsElem::Stroke(*w, *c, shift_path(v, p)),
        GraphicsElem::DashedStroke(w, d, c, p) => {
            GraphicsElem::DashedStroke(*w, *d, *c, shift_path(v, p))
        }
        GraphicsElem::Text { pt, contents, width, height, depth, transform } => {
            GraphicsElem::Text {
                pt: shift_point(v, *pt),
                contents: contents.clone(),
                width: *width,
                height: *height,
                depth: *depth,
                // A pure translation leaves the run's own 2×2 transform intact
                // (only `pt` moves) — the affine is `transform·l + pt`.
                transform: *transform,
            }
        }
        // `graphicD.ml:38`: `Group` maps every child; `Clip` shifts its own
        // clip path AND recurses into its contents.
        GraphicsElem::Group(gs) => {
            GraphicsElem::Group(gs.iter().map(|g| shift_graphics(v, g)).collect())
        }
        GraphicsElem::Clip(path, gs) => GraphicsElem::Clip(
            shift_path(v, path),
            gs.iter().map(|g| shift_graphics(v, g)).collect(),
        ),
    }
}

/// `linear-transform-graphics : float -> float -> float -> float ->
/// graphics -> graphics` (vminst.ml:2432) — `graphicD.ml`'s
/// `make_linear_trans`, applied eagerly.
pub fn linear_transform_graphics(mat: (f64, f64, f64, f64), elem: &GraphicsElem) -> GraphicsElem {
    match elem {
        GraphicsElem::Fill(c, p) => GraphicsElem::Fill(*c, linear_transform_path(mat, p)),
        GraphicsElem::Stroke(w, c, p) => GraphicsElem::Stroke(*w, *c, linear_transform_path(mat, p)),
        GraphicsElem::DashedStroke(w, d, c, p) => {
            GraphicsElem::DashedStroke(*w, *d, *c, linear_transform_path(mat, p))
        }
        // A `draw-text` run carries the composed 2×2 matrix so the writer can
        // rotate/scale the glyphs/image at render time (upstream's lazy
        // `LinearTrans` `cm`). The affine is `transform·l + pt`; pre-composing
        // `mat` gives `mat·(transform·l + pt) = (mat·transform)·l + mat·pt`, so
        // `transform ↦ mat·transform` and `pt ↦ mat·pt`. Matrices are row-major
        // `(a, b, c, d)` = `[[a, b], [c, d]]` (the `linear_transform_point`
        // convention), so the product below is the standard 2×2 multiply.
        GraphicsElem::Text { pt, contents, width, height, depth, transform } => {
            let (ma, mb, mc, md) = mat;
            let (ta, tb, tc, td) = transform.unwrap_or((1.0, 0.0, 0.0, 1.0));
            let composed = (
                ma * ta + mb * tc,
                ma * tb + mb * td,
                mc * ta + md * tc,
                mc * tb + md * td,
            );
            GraphicsElem::Text {
                pt: linear_transform_point(mat, *pt),
                contents: contents.clone(),
                width: *width,
                height: *height,
                depth: *depth,
                transform: Some(composed),
            }
        }
        GraphicsElem::Group(gs) => GraphicsElem::Group(
            gs.iter().map(|g| linear_transform_graphics(mat, g)).collect(),
        ),
        GraphicsElem::Clip(path, gs) => GraphicsElem::Clip(
            linear_transform_path(mat, path),
            gs.iter().map(|g| linear_transform_graphics(mat, g)).collect(),
        ),
    }
}

/// One axis (x or y) of a cubic Bézier's EXACT extrema (`graphicBase.ml:88`
/// `bezier_bbox`'s per-axis `aux`): for the cubic from `r0` (current point)
/// through controls `r1`, `r2` to `r3`, the derivative's roots give the
/// interior extrema; candidates are `{r0, r3, B(t+), B(t-)}` with `t` clamped
/// to `[0, 1]` (`bezier_point`'s convention: `t < 0` snaps to `r0`, `t > 1`
/// snaps to `r3`). Returns `(min, max)` over that candidate set.
fn bezier_axis_extent(r0: f64, r1: f64, r2: f64, r3: f64) -> (f64, f64) {
    // B(t) = (1-t)^3 r0 + 3(1-t)^2 t r1 + 3(1-t) t^2 r2 + t^3 r3
    // B'(t)/3 = a t^2 + b t + c, with:
    let a = -r0 + 3.0 * (r1 - r2) + r3;
    let b = 2.0 * (r0 - 2.0 * r1 + r2);
    let c = r1 - r0;
    let bezier_point = |t: f64| -> f64 {
        if t < 0.0 {
            r0
        } else if t > 1.0 {
            r3
        } else {
            let u = 1.0 - t;
            u * u * u * r0 + 3.0 * u * u * t * r1 + 3.0 * u * t * t * r2 + t * t * t * r3
        }
    };
    let mut candidates = vec![r0, r3];
    if a.abs() < 1e-12 {
        // Linear derivative (or degenerate): at most one root, `-c/b`.
        if b.abs() > 1e-12 {
            candidates.push(bezier_point(-c / b));
        }
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc >= 0.0 {
            let sq = disc.sqrt();
            candidates.push(bezier_point((-b + sq) / (2.0 * a)));
            candidates.push(bezier_point((-b - sq) / (2.0 * a)));
        }
    }
    let min = candidates.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = candidates.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    (min, max)
}

/// `get_path_bbox`/`bezier_bbox` (`graphicBase.ml:88-127,148-171`) — the
/// EXACT bounding box of `path`: walks each subpath tracking the current
/// point (`start`; each `Line` contributes its endpoint; each
/// `Bezier(c1,c2,p)` contributes the cubic extrema of `(cur, c1, c2, p)`; a
/// `Closing::Bezier(c1,c2)` contributes the extrema of `(cur, c1, c2,
/// start)`), taking each axis's true curve extent via
/// `bezier_axis_extent` rather than the (looser) control-point hull.
pub fn path_bbox(path: &Path) -> (Point, Point) {
    fn include(bounds: &mut (f64, f64, f64, f64), p: Point) {
        bounds.0 = bounds.0.min(p.0 .0);
        bounds.1 = bounds.1.max(p.0 .0);
        bounds.2 = bounds.2.min(p.1 .0);
        bounds.3 = bounds.3.max(p.1 .0);
    }
    fn include_axis_extents(bounds: &mut (f64, f64, f64, f64), ex: (f64, f64), ey: (f64, f64)) {
        bounds.0 = bounds.0.min(ex.0);
        bounds.1 = bounds.1.max(ex.1);
        bounds.2 = bounds.2.min(ey.0);
        bounds.3 = bounds.3.max(ey.1);
    }
    // (min_x, max_x, min_y, max_y).
    let mut bounds = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for sub in &path.subpaths {
        include(&mut bounds, sub.start);
        let mut cur = sub.start;
        for seg in &sub.segs {
            match *seg {
                PathSeg::Line(p) => {
                    include(&mut bounds, p);
                    cur = p;
                }
                PathSeg::Bezier(c1, c2, p) => {
                    let ex = bezier_axis_extent(cur.0 .0, c1.0 .0, c2.0 .0, p.0 .0);
                    let ey = bezier_axis_extent(cur.1 .0, c1.1 .0, c2.1 .0, p.1 .0);
                    include_axis_extents(&mut bounds, ex, ey);
                    cur = p;
                }
            }
        }
        if let Closing::Bezier(c1, c2) = sub.closing {
            let ex = bezier_axis_extent(cur.0 .0, c1.0 .0, c2.0 .0, sub.start.0 .0);
            let ey = bezier_axis_extent(cur.1 .0, c1.1 .0, c2.1 .0, sub.start.1 .0);
            include_axis_extents(&mut bounds, ex, ey);
        }
    }
    let (min_x, max_x, min_y, max_y) = bounds;
    if min_x.is_infinite() {
        return ((Length::ZERO, Length::ZERO), (Length::ZERO, Length::ZERO));
    }
    (
        (Length(min_x), Length(min_y)),
        (Length(max_x), Length(max_y)),
    )
}

fn union_bbox((amin, amax): (Point, Point), (bmin, bmax): (Point, Point)) -> (Point, Point) {
    (
        (
            Length(amin.0 .0.min(bmin.0 .0)),
            Length(amin.1 .0.min(bmin.1 .0)),
        ),
        (
            Length(amax.0 .0.max(bmax.0 .0)),
            Length(amax.1 .0.max(bmax.1 .0)),
        ),
    )
}

/// `get-graphics-bbox : graphics -> point * point` (v0.0.6 vminst.ml:2466) /
/// `graphics -> option (point * point)` (dev-0-1-0 vminst.ml:2301, the
/// "version-blind fix") — `graphicD.ml`'s `get_bbox`/`get_element_bbox`,
/// ignoring stroke thickness (upstream's own documented simplification).
/// `Clip(paths, _)` returns the CLIP PATHS' own bbox, ignoring `contents`
/// (upstream `graphicD.ml:50-52` — deliberate: the clip boundary, not what is
/// inside it, bounds the visible ink). `Group` union-folds its children
/// (`graphicD.ml:61-74`); `None` for an empty `Group` or an empty top-level
/// list, which v0.0.6 could never produce.
pub fn graphics_bbox(elem: &GraphicsElem) -> Option<(Point, Point)> {
    match elem {
        GraphicsElem::Fill(_, p)
        | GraphicsElem::Stroke(_, _, p)
        | GraphicsElem::DashedStroke(_, _, _, p) => Some(path_bbox(p)),
        GraphicsElem::Text { pt, width, height, depth, transform, .. } => {
            match transform {
                // Upright run: the axis-aligned `[0,width]×[-depth, height]`
                // extent translated to `pt`.
                None => Some(((pt.0, pt.1 - *depth), (pt.0 + *width, pt.1 + *height))),
                // Rotated/scaled run: transform the four local corners, translate
                // by `pt`, take the axis-aligned hull — so a `rotate`d figbox
                // reserves the correct (rotated) inline size.
                Some(mat) => {
                    let corners = [
                        (Length::ZERO, -*depth),
                        (*width, -*depth),
                        (*width, *height),
                        (Length::ZERO, *height),
                    ];
                    let mut min = (f64::INFINITY, f64::INFINITY);
                    let mut max = (f64::NEG_INFINITY, f64::NEG_INFINITY);
                    for c in corners {
                        let t = linear_transform_point(*mat, c);
                        let (x, y) = (t.0 .0 + pt.0 .0, t.1 .0 + pt.1 .0);
                        min = (min.0.min(x), min.1.min(y));
                        max = (max.0.max(x), max.1.max(y));
                    }
                    Some((
                        (Length(min.0), Length(min.1)),
                        (Length(max.0), Length(max.1)),
                    ))
                }
            }
        }
        GraphicsElem::Clip(path, _) => Some(path_bbox(path)),
        GraphicsElem::Group(gs) => gs
            .iter()
            .filter_map(graphics_bbox)
            .reduce(union_bbox),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Path {
        Path {
            subpaths: vec![Subpath {
                start: (Length(x0), Length(y0)),
                segs: vec![
                    PathSeg::Line((Length(x1), Length(y0))),
                    PathSeg::Line((Length(x1), Length(y1))),
                    PathSeg::Line((Length(x0), Length(y1))),
                ],
                closing: Closing::Line,
            }],
        }
    }

    /// Over a `Clip`/`Group` both move the clip path AND the contents
    /// (the `graphicD.ml:38` recursing-arm contract).
    #[test]
    fn shift_and_transform_recurse_into_clip_and_group() {
        let fill = GraphicsElem::Fill(Color::Gray(0.0), rect(0.0, 0.0, 1.0, 1.0));
        let group = GraphicsElem::Group(vec![fill.clone(), fill.clone()]);
        let shifted_group = shift_graphics((Length(2.0), Length(3.0)), &group);
        match &shifted_group {
            GraphicsElem::Group(gs) => {
                assert_eq!(gs.len(), 2);
                for g in gs {
                    assert_eq!(
                        graphics_bbox(g),
                        Some(((Length(2.0), Length(3.0)), (Length(3.0), Length(4.0))))
                    );
                }
            }
            other => panic!("expected Group, got {other:?}"),
        }

        let clip = GraphicsElem::Clip(rect(0.0, 0.0, 5.0, 5.0), vec![fill.clone()]);
        let shifted_clip = shift_graphics((Length(1.0), Length(1.0)), &clip);
        match &shifted_clip {
            GraphicsElem::Clip(path, inner) => {
                assert_eq!(
                    path_bbox(path),
                    ((Length(1.0), Length(1.0)), (Length(6.0), Length(6.0)))
                );
                assert_eq!(
                    graphics_bbox(&inner[0]),
                    Some(((Length(1.0), Length(1.0)), (Length(2.0), Length(2.0))))
                );
            }
            other => panic!("expected Clip, got {other:?}"),
        }

        // `linear-transform-graphics` (scale by 2 on both axes) also
        // recurses into both the clip path AND the contents.
        let scaled_clip = linear_transform_graphics((2.0, 0.0, 0.0, 2.0), &clip);
        match &scaled_clip {
            GraphicsElem::Clip(path, inner) => {
                assert_eq!(
                    path_bbox(path),
                    ((Length(0.0), Length(0.0)), (Length(10.0), Length(10.0)))
                );
                assert_eq!(
                    graphics_bbox(&inner[0]),
                    Some(((Length(0.0), Length(0.0)), (Length(2.0), Length(2.0))))
                );
            }
            other => panic!("expected Clip, got {other:?}"),
        }
    }

    /// `get-graphics-bbox` `Option` semantics: an empty `Group` has no
    /// ink and returns `None`; a `Group` of two fills union-folds; a `Clip`
    /// returns the CLIP PATH's own bbox, ignoring `contents`.
    #[test]
    fn bbox_option_semantics() {
        assert_eq!(graphics_bbox(&GraphicsElem::Group(vec![])), None);

        let a = GraphicsElem::Fill(Color::Gray(0.0), rect(0.0, 0.0, 1.0, 1.0));
        let b = GraphicsElem::Fill(Color::Gray(0.0), rect(2.0, 2.0, 3.0, 3.0));
        let group = GraphicsElem::Group(vec![a.clone(), b.clone()]);
        assert_eq!(
            graphics_bbox(&group),
            Some(((Length(0.0), Length(0.0)), (Length(3.0), Length(3.0))))
        );

        let clip = GraphicsElem::Clip(rect(10.0, 10.0, 20.0, 20.0), vec![a]);
        assert_eq!(
            graphics_bbox(&clip),
            Some(((Length(10.0), Length(10.0)), (Length(20.0), Length(20.0))))
        );
    }
}
