//! The drawing data model — paths, colors, and `graphics` elements. The
//! analog of upstream's `GraphicBase`/`PrePath`/`GraphicD`, trimmed to what
//! this port's backend actually draws (see `docs/plans/graphics-subsystem.md`
//! §1). Everything here is already-resolved coordinates/data — no lang-side
//! closures or deferred computation crosses into this module (see
//! `PureHorzBox::Graphics`, `hbox.rs`, and the `inline-graphics` primitive,
//! `satysfi-lang/src/primitives.rs`, for how a lang `graphics list` becomes
//! one of these).

use crate::hbox::PureHorzBox;
use crate::length::Length;

/// A point in graphics space (upstream `point`; matches the runtime
/// `Value::Tuple([Length, Length])` representation — see `as_point` in
/// `satysfi-lang/src/primitives.rs`). Graphics space is y-**up** (PDF-native);
/// the PDF writer's `place_graphics` is what flips page-layout's y-down
/// convention when placing a graphics box on a line (see that function's
/// doc comment in `satysfi-pdf`).
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
/// `point path_element`. Only `Line` is produced by any Slice-1 primitive;
/// `Bezier` is carried from the start (cheap, and the renderer already
/// handles it) so `bezier-to` (roadmap A) is a pure `primitives.rs` addition
/// with no data-model or writer change needed later.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathSeg {
    Line(Point),
    Bezier(Point, Point, Point),
}

/// How a subpath closes (`graphicBase.ml`'s `path`'s `cycleopt`): left open,
/// closed with a straight segment back to the start (`close-with-line`), or
/// closed with a cubic (`close-with-bezier`, roadmap A; the destination is
/// always the subpath's own `start`, so only the two control points are
/// stored).
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
/// lists; Slice 1 always produces exactly one subpath per `terminate-path`/
/// `close-with-line`).
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub subpaths: Vec<Subpath>,
}

/// The `pre-path` value (`PrePath.t`): a start point plus forward-accumulated
/// segments, before a `terminate-path`/`close-with-line` fixes a closing.
/// Upstream accumulates in reverse and flips at close time; this port pushes
/// forward directly (`line-to` appends to the end of `segs`), an
/// implementation detail invisible to any observable behavior.
#[derive(Clone, Debug, PartialEq)]
pub struct PrePath {
    pub start: Point,
    pub segs: Vec<PathSeg>,
}

/// One `graphics` element (`GraphicD.element`). Roadmap A/B/D (bezier path
/// ops, shift/linear-transform, dashed strokes — see
/// `docs/plans/graphics-subsystem.md`) are pure coordinate maps over
/// `Fill`/`Stroke`/`DashedStroke`'s existing `Path`, so they need no new
/// variant here (see `shift_graphics`/`linear_transform_graphics` below);
/// `place_graphics` (satysfi-pdf) still matches this exhaustively without a
/// wildcard arm.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphicsElem {
    /// Filled region, even-odd rule (matches upstream's `op_f'`; see
    /// `place_graphics`'s doc comment).
    Fill(Color, Path),
    /// Stroked outline at the given line width.
    Stroke(Length, Color, Path),
    /// Dashed stroked outline (`dashed-stroke`; roadmap D). Rendered by
    /// `place_graphics` with a PDF `d` dash-array op alongside the same
    /// stroke ops as `Stroke`.
    DashedStroke(Length, Dash, Color, Path),
    /// `draw-text` (roadmap C): a text run anchored at `pt` (box-local, y-up;
    /// the run's leftmost baseline point). `contents` is the run laid out at
    /// NATURAL width (upstream `LineBreak.natural` = `determine_widths None`,
    /// `widperfil = 0`; this port: `fit_cell(boxes, natural_width)`), each box
    /// with its x offset from `pt` — the same `(Length, PureHorzBox)` shape a
    /// `PlacedLine`/`TabularCellBox` carries. `width`/`height`/`depth` are the
    /// run's `natural_metrics`, stored at construction so `graphics_bbox`
    /// needs no re-measure (upstream recomputes via
    /// `get_metrics_of_intermediate_horz_box_list`; same numbers).
    /// Rendered by each PDF writer re-entering its own per-box emission at
    /// `pt + dx` INSIDE `place_graphics`'s box-local `cm` frame (see that
    /// function's `emit_nested` parameter).
    Text {
        pt: Point,
        contents: Vec<(Length, PureHorzBox)>,
        width: Length,
        height: Length,
        depth: Length,
    },
    /// 0.1 collection node (`GraphicD.concat`, dev-0-1-0 `graphicD.ml:23`):
    /// `unite-graphics`' payload (`docs/plans/…/prim-retype-sweep.md` §3.2).
    /// Never constructed by any 0.0.6 path — no 0.0.6-visible primitive
    /// builds one, so it is unreachable from 0.0.6 rendering by construction
    /// (§4.3's golden-PDF byte-compare is the tripwire that proves it).
    Group(Vec<GraphicsElem>),
    /// 0.1 clip node (`GraphicD.make_clip`, `graphicD.ml:97-98`): render
    /// `contents` clipped to `clip` (even-odd, `Op_W'` — `graphicD.ml:331`).
    /// The port's `Path` already carries N subpaths, standing in for
    /// upstream's `path list`. Never constructed by any 0.0.6 path (same
    /// invariant as `Group` above).
    Clip(Path, Vec<GraphicsElem>),
}

// ============================================================================
// ---- Roadmap A/B: pure coordinate transforms -----------------------------
// `shift-path`/`shift-graphics`/`linear-transform-path`/
// `linear-transform-graphics` (`docs/plans/graphics-subsystem.md` §Full
// roadmap A/B) are all EAGER point remaps — no PDF-writer change, no lazy
// `LinearTrans`-wrapper element: every point in a `Path`/`GraphicsElem` is
// rewritten up front, exactly mirroring `graphicBase.ml`'s `shift_path`/
// `linear_transform_path` (`(x, y) -> (x*a + y*b, x*c + y*d)` for the 2x2
// matrix `((a, b), (c, d))`).
// ============================================================================

/// `shift_path v pt` (`graphicBase.ml`'s `(+@%)`).
pub fn shift_point(v: Point, pt: Point) -> Point {
    (pt.0 + v.0, pt.1 + v.1)
}

/// `graphicBase.ml`'s `linear_transform_point`: `(x, y) |-> (x*a + y*b, x*c +
/// y*d)` for matrix `mat = (a, b, c, d)`.
pub fn linear_transform_point(mat: (f64, f64, f64, f64), pt: Point) -> Point {
    let (a, b, c, d) = mat;
    (pt.0 * a + pt.1 * b, pt.0 * c + pt.1 * d)
}

/// Map `f` over every point of `path` (subpath starts, every segment's
/// points — including Bézier control points — and any closing control
/// points), preserving structure. The shared plumbing `shift_path`/
/// `linear_transform_path` below specialize with `f`.
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
/// `graphicD.ml`'s `shift_element`, specialized to the variants this port
/// has (no lazy `LinearTrans` wrapper to recurse through — see the roadmap
/// A/B doc comment above).
pub fn shift_graphics(v: Point, elem: &GraphicsElem) -> GraphicsElem {
    match elem {
        GraphicsElem::Fill(c, p) => GraphicsElem::Fill(*c, shift_path(v, p)),
        GraphicsElem::Stroke(w, c, p) => GraphicsElem::Stroke(*w, *c, shift_path(v, p)),
        GraphicsElem::DashedStroke(w, d, c, p) => {
            GraphicsElem::DashedStroke(*w, *d, *c, shift_path(v, p))
        }
        GraphicsElem::Text { pt, contents, width, height, depth } => GraphicsElem::Text {
            pt: shift_point(v, *pt),
            contents: contents.clone(),
            width: *width,
            height: *height,
            depth: *depth,
        },
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
/// `make_linear_trans`, applied eagerly (see the roadmap A/B doc comment).
pub fn linear_transform_graphics(mat: (f64, f64, f64, f64), elem: &GraphicsElem) -> GraphicsElem {
    match elem {
        GraphicsElem::Fill(c, p) => GraphicsElem::Fill(*c, linear_transform_path(mat, p)),
        GraphicsElem::Stroke(w, c, p) => GraphicsElem::Stroke(*w, *c, linear_transform_path(mat, p)),
        GraphicsElem::DashedStroke(w, d, c, p) => {
            GraphicsElem::DashedStroke(*w, *d, *c, linear_transform_path(mat, p))
        }
        // **Documented deviation** (extends `prim_linear_transform_graphics`'s
        // "Eager, unlike upstream" doc comment, satysfi-lang/src/
        // primitives.rs): upstream wraps in a lazy `LinearTrans` whose
        // render-time `cm` also rotates/scales the glyphs; this port's eager
        // point-map cannot transform a text run, so `rotate-graphics`/
        // `scale-graphics` over a `draw-text` moves the anchor but does not
        // rotate the glyphs. No bundled package composes them; same class of
        // deviation as the already-shipped stroke-width note.
        GraphicsElem::Text { pt, contents, width, height, depth } => GraphicsElem::Text {
            pt: linear_transform_point(mat, *pt),
            contents: contents.clone(),
            width: *width,
            height: *height,
            depth: *depth,
        },
        // Same recursing shape as `shift_graphics` above; the stroke-width
        // caveat documented on the `Text` arm's doc comment above extends
        // to any `Stroke`/`DashedStroke` nested inside a transformed `Clip`/
        // `Group` — no behavior change, just more elements it now reaches.
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
/// [`bezier_axis_extent`] rather than the (looser) control-point hull.
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

/// Union two bounding boxes (component-wise min/max of the two corners).
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
/// `graphics -> option (point * point)` (dev-0-1-0 vminst.ml:2301,
/// `docs/plans/…/prim-retype-sweep.md` §3.2 — the L8b "version-blind fix")
/// — `graphicD.ml`'s `get_bbox`/`get_element_bbox`, ignoring stroke
/// thickness (matching upstream's own documented simplification); `Text`'s
/// bbox is the run's stored `natural_metrics` at its anchor (see that
/// variant's doc comment). `Clip(paths, _)` returns the CLIP PATHS' own
/// bbox, ignoring `contents` (upstream `graphicD.ml:50-52` — deliberate:
/// the clip boundary, not what's inside it, bounds the visible ink).
/// `Group` union-folds its children (`graphicD.ml:61-74`'s fold);
/// `None` for an empty `Group` (no ink at all) or an empty top-level list —
/// the caller-visible witness of "nothing here" that v0.0.6 could never
/// produce (no 0.0.6 constructor yields `Group`/`Clip`).
pub fn graphics_bbox(elem: &GraphicsElem) -> Option<(Point, Point)> {
    match elem {
        GraphicsElem::Fill(_, p)
        | GraphicsElem::Stroke(_, _, p)
        | GraphicsElem::DashedStroke(_, _, _, p) => Some(path_bbox(p)),
        GraphicsElem::Text { pt, width, height, depth, .. } => {
            Some(((pt.0, pt.1 - *depth), (pt.0 + *width, pt.1 + *height)))
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

    /// L5b §4.2 test 4: `shift-graphics`/`linear-transform-graphics` over a
    /// `Clip`/`Group` move both the clip path and the contents (the
    /// `graphicD.ml:38` recursing-arm contract — prim-retype-sweep.md §3.2).
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

    /// `get-graphics-bbox` `Option` semantics (L8b): an empty `Group` has no
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
