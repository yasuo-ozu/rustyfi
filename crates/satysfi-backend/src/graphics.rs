//! The drawing data model — paths, colors, and `graphics` elements. The
//! analog of upstream's `GraphicBase`/`PrePath`/`GraphicD`, trimmed to what
//! this port's backend actually draws (see `docs/plans/graphics-subsystem.md`
//! §1). Everything here is already-resolved coordinates/data — no lang-side
//! closures or deferred computation crosses into this module (see
//! `PureHorzBox::Graphics`, `hbox.rs`, and the `inline-graphics` primitive,
//! `satysfi-lang/src/primitives.rs`, for how a lang `graphics list` becomes
//! one of these).

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
    /// `draw-text`'s anchor point (roadmap C) — **STAND-IN**: faithful text
    /// emission needs the line breaker + font metrics threaded into the PDF
    /// writer's text path (a heavier coupling than pure paths; see
    /// `docs/plans/graphics-subsystem.md`'s Risks section, "`draw-text`
    /// reaches back into layout"). This variant keeps only the anchor point,
    /// so `shift-graphics`/`get-graphics-bbox` stay faithful for it (a
    /// zero-size box at `pt`, shift-covariant like every other element); the
    /// `inline-boxes` content is dropped and `place_graphics` renders it as
    /// a no-op. Real text emission is future work (roadmap C).
    Text(Point),
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
        GraphicsElem::Text(pt) => GraphicsElem::Text(shift_point(v, *pt)),
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
        GraphicsElem::Text(pt) => GraphicsElem::Text(linear_transform_point(mat, *pt)),
    }
}

/// Bounding box of every point in `path` (start, every segment's points —
/// including Bézier control points — and any closing control points).
/// Matches `get-graphics-bbox`'s upstream *shape* (a corner-pair result) but,
/// unlike `graphicBase.ml`'s exact cubic-root `bezier_bbox`, takes the
/// control-point hull rather than the tight curve extent — a safe (if
/// occasionally looser) superset, since a cubic Bézier always lies within
/// its control points' convex hull. `gr.satyh` only ever consumes this bbox
/// to center/left-align a text run (`Gr.text-centering`/`-leftward`), where
/// the looser box is immaterial.
pub fn path_bbox(path: &Path) -> (Point, Point) {
    let mut pts: Vec<Point> = Vec::new();
    for sub in &path.subpaths {
        pts.push(sub.start);
        for seg in &sub.segs {
            match *seg {
                PathSeg::Line(p) => pts.push(p),
                PathSeg::Bezier(c1, c2, p) => {
                    pts.push(c1);
                    pts.push(c2);
                    pts.push(p);
                }
            }
        }
        if let Closing::Bezier(c1, c2) = sub.closing {
            pts.push(c1);
            pts.push(c2);
        }
    }
    if pts.is_empty() {
        return ((Length::ZERO, Length::ZERO), (Length::ZERO, Length::ZERO));
    }
    let (mut min_x, mut min_y) = pts[0];
    let (mut max_x, mut max_y) = pts[0];
    for &(x, y) in &pts[1..] {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    ((min_x, min_y), (max_x, max_y))
}

/// `get-graphics-bbox : graphics -> point * point` (vminst.ml:2466) —
/// `graphicD.ml`'s `get_element_bbox`, ignoring stroke thickness (matching
/// upstream's own documented simplification) and, for the `Text` stand-in, a
/// zero-size box at the anchor point (see that variant's doc comment).
pub fn graphics_bbox(elem: &GraphicsElem) -> (Point, Point) {
    match elem {
        GraphicsElem::Fill(_, p)
        | GraphicsElem::Stroke(_, _, p)
        | GraphicsElem::DashedStroke(_, _, _, p) => path_bbox(p),
        GraphicsElem::Text(pt) => (*pt, *pt),
    }
}
