//! Paragraph breaking. Phase 6: Knuth–Plass optimal line breaking over glue
//! breakpoints. The input model (a flat `Vec<HorzBox>` of strings and glue)
//! matches what lineBreak.ml consumes, so this function is a drop-in
//! replacement for the milestone-1 greedy breaker; callers (satysfi-lang,
//! satysfi-pdf) are unaffected.
//!
//! Deviations from lineBreak.ml (v0.0.6), noted where they matter:
//! - v0.0.6 builds a DAG over `DiscretionaryID`s (hyphenation points) and
//!   finds a shortest path through it (see `LineBreakGraph`, `update_graph`
//!   in lineBreak.ml). We have no discretionaries/hyphenation yet, so
//!   breakpoints are just glue (`OuterEmpty`/`OuterFil`) boxes, and we run
//!   the classic Knuth–Plass dynamic program directly over them instead of
//!   materializing a graph.
//! - v0.0.6 drops `LBTooShort` edges entirely (a breakpoint pair that can't
//!   stretch enough is simply unreachable that way) and only tolerates
//!   `LBTooLong` a bounded number of times with a fixed `badness_for_too_long
//!   = 100_000` (lineBreak.ml lines 985-1027). Since we must always be able
//!   to typeset *something* (an overfull unbreakable word must still
//!   produce one line, never a panic or a stuck search), every candidate
//!   line stays representable: we cap its badness at `BADNESS_INF` instead
//!   of excluding it.

use crate::context::Context;
use crate::hbox::{HorzBox, PureHorzBox};
use crate::length::Length;
use crate::vbox::VertBox;

/// Badness cap. lineBreak.ml computes `badness = |ratio^3| * 10000`
/// (lineBreak.ml:985-986, `calculate_badness`) and separately hardcodes
/// `badness_for_too_long = 100_000` for overfull lines it must still keep
/// (lineBreak.ml:989). We use the classic Knuth–Plass badness scale
/// (`100 * |ratio|^3`, TeX's `badness` function) and cap every
/// out-of-range or infeasible line at this single constant, which plays
/// the same "this line is bad but still representable" role as v0.0.6's
/// `badness_for_too_long`.
const BADNESS_INF: f64 = 10_000.0;

/// Classic Knuth–Plass default line penalty. Not a lineBreak.ml constant:
/// v0.0.6 has no flat per-line penalty in this position — its edge weights
/// are `badness + pnltybreak`, where `pnltybreak` comes from a
/// `HorzDiscretionary`'s own penalty (lineBreak.ml:1012), which does not
/// exist for us since we have no discretionaries. We adopt TeX's classic
/// default line penalty instead, folded into `demerits = (LINE_PENALTY +
/// badness)^2`.
const LINE_PENALTY: f64 = 10.0;

/// A candidate line's shape, used both to score it (badness/demerits) and
/// to lay it out once chosen.
struct LineMetrics {
    natural: Length,
    stretch: Length,
    shrink: Length,
    has_fil: bool,
}

fn measure(line: &[PureHorzBox]) -> LineMetrics {
    let mut natural = Length::ZERO;
    let mut stretch = Length::ZERO;
    let mut shrink = Length::ZERO;
    let mut has_fil = false;
    for bx in line {
        match bx {
            PureHorzBox::InnerString { width, .. } => natural += *width,
            PureHorzBox::OuterEmpty {
                natural: n,
                shrinkable,
                stretchable,
            } => {
                natural += *n;
                stretch += *stretchable;
                shrink += *shrinkable;
            }
            PureHorzBox::OuterFil => has_fil = true,
            PureHorzBox::FixedEmpty { width } => natural += *width,
            PureHorzBox::Graphics { width, .. } => natural += *width,
        }
    }
    LineMetrics {
        natural,
        stretch,
        shrink,
        has_fil,
    }
}

/// Adjustment-ratio badness for one candidate line. The ratio itself is
/// exactly lineBreak.ml's `calculate_ratios` (lines 510-548): `(target -
/// natural) / stretch` when underfull, `(target - natural) / shrink` when
/// overfull, `0` when an `inline-fil` is present and underfull (the
/// `Fils(nfil)` branch at lines 517-524 always reports ratio `0`). Unlike
/// lineBreak.ml, we don't classify a ratio beyond `ratio_stretch_limit =
/// 2.0` / `ratio_shrink_limit = -1.0` (lines 507-508) as categorically
/// "TooShort"/"TooLong" and cut it off there — those limits exist in
/// v0.0.6 to decide whether to keep a graph edge at all, which has no
/// analogue in a DP over every candidate line. Instead badness grows
/// continuously as `100 * |r|^3` (the classic Knuth–Plass/TeX badness
/// function) and simply saturates at `BADNESS_INF` once it gets there —
/// so a moderately-bad line (say `r = 2`, badness 800) is still scored
/// far better than a catastrophically-bad one, instead of both being
/// flattened to the same "TooShort" cost.
fn badness(width: Length, metrics: &LineMetrics) -> f64 {
    let slack = width - metrics.natural;
    if slack.0.abs() < 1e-9 {
        return 0.0;
    }
    if slack.is_positive() {
        // Underfull: needs to stretch.
        if metrics.has_fil {
            return 0.0;
        }
        if !metrics.stretch.is_positive() {
            return BADNESS_INF;
        }
        let ratio = slack / metrics.stretch;
        (100.0 * ratio.abs().powi(3)).min(BADNESS_INF)
    } else {
        // Overfull: needs to shrink.
        if !metrics.shrink.is_positive() {
            return BADNESS_INF;
        }
        let ratio = slack / metrics.shrink;
        (100.0 * ratio.abs().powi(3)).min(BADNESS_INF)
    }
}

fn demerits(b: f64) -> f64 {
    (LINE_PENALTY + b) * (LINE_PENALTY + b)
}

/// Break a paragraph's boxes into justified lines using Knuth–Plass
/// dynamic programming over glue breakpoints.
pub fn break_into_lines(ctx: &Context, boxes: Vec<HorzBox>) -> Vec<VertBox> {
    let pure: Vec<PureHorzBox> = boxes.into_iter().map(|HorzBox::Pure(p)| p).collect();
    let width = ctx.paragraph_width;
    let n = pure.len();

    if n == 0 {
        return Vec::new();
    }

    // Legal breakpoints: a glue box immediately following a non-glue box
    // (never at the very start of a line — "leading glue after a break is
    // dropped", matching the previous greedy's behavior). For each such
    // glue at index `g`, a line ending there spans up to (excluding) `g`,
    // and the next line starts at `g + 1` (the glue itself is discarded).
    // The end of the paragraph is always a forced final breakpoint too.
    //
    // `nodes[k] = (line_end_excl, next_line_start)`; node 0 is the
    // virtual start of the paragraph.
    let mut starts: Vec<usize> = vec![0];
    let mut ends: Vec<usize> = Vec::new();
    for g in 1..n {
        if pure[g].is_glue() && !pure[g - 1].is_glue() {
            ends.push(g);
            starts.push(g + 1);
        }
    }
    ends.push(n); // forced final break; has no "next start".

    let m = ends.len();
    // dp[k] = (best total demerits, line count) to reach node k (k in
    // 0..=m, where node k>0 means "paragraph broken through ends[k-1]").
    const EPS: f64 = 1e-6;
    let mut dp: Vec<(f64, usize)> = vec![(f64::INFINITY, usize::MAX); m + 1];
    let mut back: Vec<usize> = vec![usize::MAX; m + 1];
    dp[0] = (0.0, 0);

    for j in 1..=m {
        let raw_end = ends[j - 1];
        // Width short-circuit: once a candidate line is wildly overfull
        // for every remaining start (natural width only grows as `i`
        // decreases further back... actually grows as we consider
        // earlier starts), stop trying earlier starts for this `j`. We
        // scan `i` from the closest (largest, tightest line) backward,
        // so the break condition below is safe: once a line is far past
        // any hope of representable badness (well beyond the shrink
        // limit) trying an even earlier `i` only makes it worse.
        for i in (0..j).rev() {
            if dp[i].0.is_infinite() {
                continue;
            }
            let start = starts[i];
            if start > raw_end {
                // Can't happen (starts/ends interleave), but guard anyway.
                continue;
            }
            let metrics = measure(line_content(&pure, start, raw_end));
            let b = badness(width, &metrics);
            let d = demerits(b);
            let cand_cost = dp[i].0 + d;
            let cand_lines = dp[i].1 + 1;
            if cand_cost < dp[j].0 - EPS
                || ((cand_cost - dp[j].0).abs() <= EPS && cand_lines < dp[j].1)
            {
                dp[j] = (cand_cost, cand_lines);
                back[j] = i;
            }
            // Near-linear short-circuit: if this line is already massively
            // overfull (natural width more than double the target beyond
            // what any shrink could fix) and we're not at the very first
            // (tightest) candidate for this `j`, earlier `i` only grows
            // the line further, so stop scanning backward.
            if i + 1 != j && metrics.natural.0 > width.0 * 4.0 + 1.0 {
                break;
            }
        }
    }

    // Reconstruct the chosen breakpoints.
    let mut line_ranges: Vec<(usize, usize)> = Vec::new();
    let mut j = m;
    while j > 0 {
        let i = back[j];
        debug_assert_ne!(i, usize::MAX, "no path found to breakpoint {j}");
        line_ranges.push((starts[i], ends[j - 1]));
        j = i;
    }
    line_ranges.reverse();

    let line_count = line_ranges.len();
    line_ranges
        .into_iter()
        .enumerate()
        .map(|(idx, (start, raw_end))| {
            let content: Vec<PureHorzBox> = line_content(&pure, start, raw_end).to_vec();
            layout_line(ctx, content, width, idx + 1 == line_count)
        })
        .collect()
}

/// Trailing glue never justifies anything and is dropped from a line,
/// except a trailing `OuterFil` (which is how a paragraph's final
/// stretch is represented, and must stay so the last line can absorb
/// slack without being force-justified).
fn trim_trailing_glue(line: &[PureHorzBox]) -> &[PureHorzBox] {
    let mut end = line.len();
    while end > 0 {
        match &line[end - 1] {
            PureHorzBox::OuterEmpty { .. } => end -= 1,
            _ => break,
        }
    }
    &line[..end]
}

/// A break never leaves glue at the very start of the next line either
/// (the old greedy dropped any glue seen while `current` was still
/// empty); drop it here so a pathological run of consecutive glue boxes
/// doesn't get counted as this line's content.
fn trim_leading_glue(line: &[PureHorzBox]) -> &[PureHorzBox] {
    let mut start = 0;
    while start < line.len() && line[start].is_glue() {
        start += 1;
    }
    &line[start..]
}

/// The actual content of a line spanning `pure[start..raw_end)`, with
/// leading and trailing glue trimmed.
fn line_content(pure: &[PureHorzBox], start: usize, raw_end: usize) -> &[PureHorzBox] {
    trim_trailing_glue(trim_leading_glue(&pure[start..raw_end]))
}

/// Assign x offsets, justifying interior lines by distributing slack into
/// glue (`OuterFil` absorbs all positive slack; otherwise stretchables or
/// shrinkables share it proportionally). The last line stays ragged: it is
/// never force-*stretched* to fill the width, but it is still *shrunk* if
/// overfull, since shrink represents real interword compressibility, not
/// justification.
fn layout_line(ctx: &Context, line: Vec<PureHorzBox>, width: Length, is_last: bool) -> VertBox {
    let natural: Length = line
        .iter()
        .map(|b| b.natural_width())
        .fold(Length::ZERO, |acc, w| acc + w);
    let slack = width - natural;

    let fil_count = line
        .iter()
        .filter(|b| matches!(b, PureHorzBox::OuterFil))
        .count();
    let stretch_total: Length = line
        .iter()
        .map(|b| match b {
            PureHorzBox::OuterEmpty { stretchable, .. } => *stretchable,
            _ => Length::ZERO,
        })
        .fold(Length::ZERO, |acc, w| acc + w);
    let shrink_total: Length = line
        .iter()
        .map(|b| match b {
            PureHorzBox::OuterEmpty { shrinkable, .. } => *shrinkable,
            _ => Length::ZERO,
        })
        .fold(Length::ZERO, |acc, w| acc + w);
    // Clamp the shrink ratio at -1 (full collapse): don't let glue widths
    // go negative when a line is overfull beyond its shrink capacity —
    // mirrors lineBreak.ml's `LBTooLong` case, which subtracts each box's
    // full `shrinkable` rather than over-shrinking past it
    // (lineBreak.ml:588-590).
    let shrink_ratio = if slack.is_positive() || !shrink_total.is_positive() {
        0.0
    } else {
        (slack / shrink_total).max(-1.0)
    };

    let mut x = Length::ZERO;
    let mut contents = Vec::with_capacity(line.len());
    let mut height = Length::ZERO;
    let mut depth = Length::ZERO;

    for bx in line {
        let advance = match &bx {
            PureHorzBox::InnerString {
                width,
                height: h,
                depth: d,
                ..
            } => {
                height = height.max(*h);
                depth = depth.max(*d);
                *width
            }
            PureHorzBox::OuterEmpty {
                natural,
                shrinkable,
                stretchable,
            } => {
                let mut adv = *natural;
                if slack.is_positive() {
                    if fil_count == 0 && stretch_total.is_positive() && !is_last {
                        adv += slack * (*stretchable / stretch_total);
                    }
                } else if shrink_ratio != 0.0 {
                    adv += *shrinkable * shrink_ratio;
                }
                adv
            }
            PureHorzBox::OuterFil => {
                if fil_count > 0 && slack.is_positive() {
                    slack * (1.0 / fil_count as f64)
                } else {
                    Length::ZERO
                }
            }
            PureHorzBox::FixedEmpty { width } => *width,
            PureHorzBox::Graphics {
                width,
                height: h,
                depth: d,
                ..
            } => {
                height = height.max(*h);
                depth = depth.max(*d);
                *width
            }
        };
        contents.push((x, bx));
        x += advance;
    }

    // An all-glue line still needs sane metrics.
    if height == Length::ZERO && depth == Length::ZERO {
        height = ctx.font_size * 0.75;
        depth = ctx.font_size * 0.25;
    }

    VertBox::Line {
        height,
        depth,
        contents,
    }
}
