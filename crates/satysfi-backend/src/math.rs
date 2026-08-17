//! Math value / box model — Slice 1 of `docs/plans/math-engine.md`. Trimmed
//! analog of `math.ml`'s `math_kind` (`horzBox.ml:134`) and `low_math_atom`
//! (`math.ml:9`), holding only what a fixed-constant super/subscript layout
//! needs (no MATH-table metrics — see the plan's roadmap §B).

use crate::hbox::HorzStringInfo;
use crate::length::Length;

/// v0.0.6 `math_kind` (horzBox.ml:134-145). `Prefix` is a SATySFi-specific
/// class (differential `d`, `\partial`); `End` is a synthetic list-boundary
/// sentinel. Full set kept so the pairwise spacing table (roadmap A) extends
/// rather than replaces it; Slice 1 only ever produces Ord/Bin/Rel/Punct.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathKind {
    Ord,
    Bin,
    Rel,
    Op,
    Punct,
    Open,
    Close,
    Prefix,
    Inner,
    End,
}

/// One already-positioned glyph inside a laid-out math run: a string set in
/// `info` (font + size — a superscript carries a *smaller* size here), placed
/// at `dx` right of the math box's origin and `dy` **above** its baseline
/// (dy < 0 = below, for subscripts). The analog of `math.ml`'s `LowMathGlyph`
/// after `horz_of_low_math` has resolved its `PHGRising` shift into an
/// offset.
#[derive(Clone, Debug, PartialEq)]
pub struct MathGlyph {
    pub info: HorzStringInfo,
    pub text: String,
    pub dx: Length,
    pub dy: Length,
    pub width: Length,
    pub height: Length,
    pub depth: Length,
}
