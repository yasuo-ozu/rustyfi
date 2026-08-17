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

/// One `graphics` element (`GraphicD.element`). Slice 1 = `Fill` + `Stroke`
/// only; further roadmap element kinds (dashed strokes, text, linear
/// transforms — see `docs/plans/graphics-subsystem.md` §B/C/D) are not yet
/// variants here, so `place_graphics` (satysfi-pdf) can match this
/// exhaustively without a wildcard arm.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphicsElem {
    /// Filled region, even-odd rule (matches upstream's `op_f'`; see
    /// `place_graphics`'s doc comment).
    Fill(Color, Path),
    /// Stroked outline at the given line width.
    Stroke(Length, Color, Path),
}
