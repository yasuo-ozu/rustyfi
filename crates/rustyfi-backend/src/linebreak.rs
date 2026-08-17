//! Paragraph breaking. Phase 6: Knuth–Plass optimal line breaking over glue
//! and discretionary breakpoints. The input model (a flat `Vec<HorzBox>` of
//! strings, glue and discretionaries) matches what lineBreak.ml consumes,
//! so this function is a drop-in replacement for the milestone-1 greedy
//! breaker; callers (rustyfi-lang, rustyfi-pdf) are unaffected.
//!
//! Deviations from lineBreak.ml (v0.0.6), noted where they matter:
//! - v0.0.6 builds a DAG over `DiscretionaryID`s (hyphenation points) and
//!   finds a shortest path through it (see `LineBreakGraph`, `update_graph`
//!   in lineBreak.ml). We run the classic Knuth–Plass dynamic program
//!   directly over `is_break_point` candidates (glue or `Discretionary`)
//!   instead of materializing a graph; a forced break (penalty `<=
//!   FORCED_BREAK_PENALTY`, e.g. UAX#14 `Mandatory`) is modeled as a `floor`
//!   that ratchets forward so no later line can span back over it, rather
//!   than as a distinct graph node kind.
//! - v0.0.6 drops `LBTooShort` edges entirely (a breakpoint pair that can't
//!   stretch enough is simply unreachable that way) and only tolerates
//!   `LBTooLong` a bounded number of times with a fixed `badness_for_too_long
//!   = 100_000` (lineBreak.ml lines 985-1027). Since we must always be able
//!   to typeset *something* (an overfull unbreakable word must still
//!   produce one line, never a panic or a stuck search), every candidate
//!   line stays representable: we cap its badness at `BADNESS_INF` instead
//!   of excluding it.

use crate::context::Context;
use crate::hbox::{HorzBox, PureHorzBox, FORCED_BREAK_PENALTY};
use crate::length::Length;
use crate::vbox::VertBox;

/// A UAX#14 break opportunity's kind, reduced to the two outcomes the
/// paragraph breaker needs (v0.0.6's ~40-rule engine over `LineBreak.txt`
/// classes, `ref:src/chardecoder/lineBreakDataMap.ml`, collapses the same
/// way into `append_break_opportunity`'s direct/mandatory distinction).
/// Wraps `unicode_linebreak::BreakOpportunity` so callers depend on this
/// crate's vocabulary rather than the segmenter crate directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakKind {
    /// A break is legal but optional (an ordinary word/punctuation
    /// boundary) — a `Discretionary` candidate.
    Allowed,
    /// A break is required (e.g. a literal newline) — the paragraph
    /// breaker must end a line here (see `FORCED_BREAK_PENALTY`).
    Mandatory,
}

/// Unicode line-breaking (UAX#14) opportunities in `text`, as
/// `(byte_offset, kind)` pairs in ascending order (`unicode-linebreak`'s
/// `linebreaks`, a compiled pair table — no unidata files to ship). Does
/// *not* do v0.0.6's script/East-Asian-width segmentation or JLreq
/// tailoring (`ref:src/chardecoder/scriptDataMap.ml`); see
/// `docs/plans/text-rendering.md` §3 for the evaluated alternatives.
pub fn break_opportunities(text: &str) -> Vec<(usize, BreakKind)> {
    unicode_linebreak::linebreaks(text)
        .map(|(i, opp)| {
            let kind = match opp {
                unicode_linebreak::BreakOpportunity::Mandatory => BreakKind::Mandatory,
                unicode_linebreak::BreakOpportunity::Allowed => BreakKind::Allowed,
            };
            (i, kind)
        })
        .collect()
}

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
/// v0.0.6's edge weight is `badness + pnltybreak`, where `pnltybreak` comes
/// from a `HorzDiscretionary`'s own penalty (lineBreak.ml:1012) — the same
/// role our `Discretionary::penalty` plays via `demerits`. We adopt TeX's
/// classic default line penalty as the flat part, folded into `demerits =
/// (LINE_PENALTY + badness)^2 [+/- penalty^2]`.
const LINE_PENALTY: f64 = 10.0;

/// A candidate line's shape, used both to score it (badness/demerits) and
/// to lay it out once chosen.
struct LineMetrics {
    natural: Length,
    stretch: Length,
    shrink: Length,
    has_fil: bool,
    /// Whether the line contains any real (breakable) interword glue
    /// (`OuterEmpty`). Distinguishes a rigid-but-spaced line (monospace/`+code`
    /// via `set-space-ratio r 0 0`) from unspaced CJK (which breaks between
    /// characters, not on glue) — the former MUST wrap rather than overflow.
    has_glue: bool,
}

fn measure(line: &[PureHorzBox]) -> LineMetrics {
    let mut natural = Length::ZERO;
    let mut stretch = Length::ZERO;
    let mut shrink = Length::ZERO;
    let mut has_fil = false;
    let mut has_glue = false;
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
                has_glue = true;
            }
            PureHorzBox::OuterFil => has_fil = true,
            PureHorzBox::FixedEmpty { width } => natural += *width,
            PureHorzBox::Image { width, .. } => natural += *width,
            // §4 (hyphenation): a discretionary that does NOT end this line
            // renders its `no_break` slot (matches `hbox.rs::natural_width`
            // and upstream's `get_leftmost/rightmost` no-break choice) —
            // empty for every UAX#14-only discretionary (§3), so this is a
            // no-op until §4 fills `no_break`.
            PureHorzBox::Discretionary { no_break, .. } => {
                for b in no_break {
                    natural += b.natural_width();
                }
            }
            PureHorzBox::Graphics { width, .. } => natural += *width,
            // Counted as a fil for width purposes (upstream `Fils(1)`), same
            // as `OuterFil` above — no `natural` contribution.
            PureHorzBox::GraphicsOuter { .. } => has_fil = true,
            PureHorzBox::Math { width, .. } => natural += *width,
            // Zero-width marker; fired lang-side after placement.
            PureHorzBox::HookPageBreak { .. } => {}
            PureHorzBox::Tabular(tab) => natural += tab.width,
            PureHorzBox::EmbeddedBlock { width, .. } => natural += *width,
            PureHorzBox::Frame { width, .. } => natural += *width,
            PureHorzBox::FrameMarker { .. } => {}
            // Zero-width marker; fired to the page bottom by `chop_page`.
            PureHorzBox::Footnote { .. } => {}
            // `docs/plans/design-reflow-s4-lists.md` §4.3: inert, zero
            // contribution — read only by the reflow HTML walker.
            PureHorzBox::InlineMark(_) => {}
        }
    }
    LineMetrics {
        natural,
        stretch,
        shrink,
        has_fil,
        has_glue,
    }
}

/// `get-natural-metrics` (vminst.ml:2020 `PrimitiveGetNaturalMetrics`;
/// lineBreak.ml's `get_natural_metrics`): `boxes`' width/height/depth as if
/// laid out on a single unbroken line. A `Discretionary` contributes its
/// `no_break` slot (the same choice `get_leftmost_script`/
/// `get_rightmost_script` make in lineBreak.ml — that's what actually
/// renders when the break isn't taken). Unlike lineBreak.ml, whose depth is
/// signed (more negative = deeper, combined via `min`) and gets negated
/// before this primitive returns it, this port's `PureHorzBox` depths are
/// already non-negative "how far below the baseline" magnitudes (see
/// hbox.rs), so `depth` is combined via `.max` directly with no sign flip.
pub fn natural_metrics(boxes: &[HorzBox]) -> (Length, Length, Length) {
    fn go<'a>(
        pure: impl IntoIterator<Item = &'a PureHorzBox>,
        width: &mut Length,
        height: &mut Length,
        depth: &mut Length,
    ) {
        for bx in pure {
            match bx {
                PureHorzBox::InnerString {
                    width: w,
                    height: h,
                    depth: d,
                    ..
                } => {
                    *width += *w;
                    *height = (*height).max(*h);
                    *depth = (*depth).max(*d);
                }
                PureHorzBox::OuterEmpty { natural, .. } => *width += *natural,
                PureHorzBox::OuterFil => {}
                PureHorzBox::FixedEmpty { width: w } => *width += *w,
                PureHorzBox::Image { width: w, height: h, .. } => {
                    *width += *w;
                    *height = (*height).max(*h);
                }
                PureHorzBox::Discretionary { no_break, .. } => go(no_break, width, height, depth),
                PureHorzBox::Graphics {
                    width: w,
                    height: h,
                    depth: d,
                    ..
                } => {
                    *width += *w;
                    *height = (*height).max(*h);
                    *depth = (*depth).max(*d);
                }
                PureHorzBox::Math {
                    width: w,
                    height: h,
                    depth: d,
                    ..
                } => {
                    *width += *w;
                    *height = (*height).max(*h);
                    *depth = (*depth).max(*d);
                }
                // Zero width contribution (fil semantics); height/depth
                // still feed the run's outer metrics.
                PureHorzBox::GraphicsOuter { height: h, depth: d, .. } => {
                    *height = (*height).max(*h);
                    *depth = (*depth).max(*d);
                }
                PureHorzBox::HookPageBreak { .. } => {}
                PureHorzBox::Tabular(tab) => {
                    *width += tab.width;
                    *height = (*height).max(tab.height);
                    *depth = (*depth).max(tab.depth);
                }
                PureHorzBox::EmbeddedBlock {
                    width: w,
                    height: h,
                    depth: d,
                    ..
                } => {
                    *width += *w;
                    *height = (*height).max(*h);
                    *depth = (*depth).max(*d);
                }
                PureHorzBox::Frame {
                    width: w,
                    height: h,
                    depth: d,
                    ..
                } => {
                    *width += *w;
                    *height = (*height).max(*h);
                    *depth = (*depth).max(*d);
                }
                PureHorzBox::FrameMarker { .. } => {}
                PureHorzBox::Footnote { .. } => {}
                // `docs/plans/design-reflow-s4-lists.md` §4.3: inert, zero
                // contribution.
                PureHorzBox::InlineMark(_) => {}
            }
        }
    }
    let mut width = Length::ZERO;
    let mut height = Length::ZERO;
    let mut depth = Length::ZERO;
    go(
        boxes.iter().map(|HorzBox::Pure(p)| p),
        &mut width,
        &mut height,
        &mut depth,
    );
    (width, height, depth)
}

/// `embed-block-breakable`/`embed-block-top`'s box-sizing helper
/// (docs/plans/context-box-prims.md §3) — the block analog of
/// `natural_metrics` above, but summed rather than maxed (a block's lines
/// stack vertically, they don't compete for one shared baseline the way
/// concurrent inline boxes on a line do). Each `Line` contributes its own
/// `height`/`depth` to the running totals; each `Skip` adds its length to
/// `height` only (there is nothing below a bare skip to call "depth").
/// E.g. `measure_block(&[Line{h,d}, Skip(s)]) == (h+s, d)`.
pub fn measure_block(block: &[VertBox]) -> (Length, Length) {
    let mut height = Length::ZERO;
    let mut depth = Length::ZERO;
    for vb in block {
        match vb {
            VertBox::Line { height: h, depth: d, .. } => {
                height += *h;
                depth += *d;
            }
            VertBox::Skip(s) => height += *s,
            // `clear-page`/`hook-page-break-block`/frame markers contribute
            // zero height, same as upstream's
            // `ImVertFixedEmpty(_, Length.zero)`.
            VertBox::ClearPage
            | VertBox::HookPageBreak(_)
            | VertBox::FrameStart(_)
            | VertBox::FrameEnd(_)
            | VertBox::ListMark(_) => {}
        }
    }
    (height, depth)
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
        if metrics.stretch.is_positive() {
            let ratio = slack / metrics.stretch;
            (100.0 * ratio.abs().powi(3)).min(BADNESS_INF)
        } else {
            no_stretch_badness(slack, width)
        }
    } else {
        // Overfull: needs to shrink.
        if metrics.shrink.is_positive() {
            let ratio = slack / metrics.shrink;
            (100.0 * ratio.abs().powi(3)).min(BADNESS_INF)
        } else if metrics.has_glue {
            // Overfull with real interword glue that can't shrink (monospace/
            // `+code`, `set-space-ratio r 0 0`): breaking BEFORE the word that
            // doesn't fit is the right call, so this must dominate any
            // hyphen/line penalty. `no_stretch_badness` (below) scores overflow
            // as a cube of the overflow FRACTION — near-zero for a modest
            // overflow — which let the DP cram extra words onto an already
            // overfull line and run text clean off the page edge (visible
            // clipping in latexcmds `+code`/`\code`). Force the wrap.
            BADNESS_INF
        } else {
            // No breakable glue at all (unspaced CJK): nowhere better to break,
            // so fall back to the continuous overflow score.
            no_stretch_badness(slack, width)
        }
    }
}

/// Badness for a line with *no* elastic capacity at all to absorb its
/// shortfall/overflow — e.g. a run of zero-width discretionaries with no
/// glue, exactly what unspaced CJK looks like (`is_break_point`'s doc). Two
/// failure modes to avoid here, pulling in opposite directions:
/// - A flat "infinitely bad" (as when some stretch/shrink exists but is
///   exhausted) ties every such line at the same cost regardless of how
///   under/overfull it actually is, so the DP's fewer-lines tiebreak
///   perversely prefers cramming more onto one wildly overfull line over
///   correctly splitting it — wrong for CJK (see
///   `narrow_measure_wraps_cjk_at_ideograph_discretionaries`,
///   tests/linebreak_uax14.rs).
/// - Scoring it *too* cheaply (the same `100 * ratio^3` scale as a line
///   that does have stretch/shrink) makes an isolated unbreakable word cost
///   little enough that the DP prefers many single-word lines over the
///   correctly-combined, real-glue-justified ones — wrong for Latin (see
///   `wraps_at_glue` et al., tests/linebreak.rs).
/// There's nothing to form a ratio against but the target width itself, so
/// use that, but scaled 100x steeper (`BADNESS_INF * ratio^3`, vs. plain
/// elastic badness's `100 * ratio^3`) before capping at the same
/// `BADNESS_INF` ceiling: a `ratio` this small is a much bigger share of
/// "everything you have" when nothing is elastic at all, so it should cost
/// disproportionately more than the same ratio against real stretch/shrink.
fn no_stretch_badness(slack: Length, width: Length) -> f64 {
    let ratio = slack / width.max(Length::pt(1.0));
    (BADNESS_INF * ratio.abs().powi(3)).min(BADNESS_INF)
}

/// Fold a break's own penalty into its line's demerits, TeX's classic
/// formula (TeXbook ch.14): a positive penalty discourages breaking there
/// (`+ p^2`), a negative one encourages it (`- p^2`), and `<=
/// FORCED_BREAK_PENALTY` is scored plainly since the DP's `floor` already
/// guarantees the break is taken regardless of cost. Glue's implicit
/// penalty is always 0, so this is exactly today's formula whenever no
/// discretionary is involved.
fn demerits(b: f64, penalty: i32) -> f64 {
    let base = (LINE_PENALTY + b) * (LINE_PENALTY + b);
    if penalty <= FORCED_BREAK_PENALTY {
        base
    } else {
        let p = penalty as f64;
        if penalty > 0 {
            base + p * p
        } else {
            (base - p * p).max(0.0)
        }
    }
}

/// Break a paragraph's boxes into justified lines using Knuth–Plass
/// dynamic programming over glue and discretionary breakpoints.
pub fn break_into_lines(ctx: &Context, boxes: Vec<HorzBox>) -> Vec<VertBox> {
    let pure: Vec<PureHorzBox> = boxes.into_iter().map(|HorzBox::Pure(p)| p).collect();
    let width = ctx.paragraph_width;
    let n = pure.len();

    if n == 0 {
        return Vec::new();
    }

    // Legal breakpoints: a glue-or-discretionary box (`is_break_point`)
    // immediately following a box that isn't one (never at the very start
    // of a line — "leading glue after a break is dropped", matching the
    // previous greedy's behavior). For each such box at index `g`, a line
    // ending there spans up to (excluding) `g`, and the next line starts
    // at `g + 1` (the box itself is discarded, same as glue always was).
    // A run of several adjacent break candidates collapses to just its
    // first: the trim helpers below eat whatever of the run leaks into a
    // line's edges either way, so this loses no representable line, only
    // redundant DP states. The end of the paragraph is always a forced
    // final breakpoint too.
    //
    // `nodes[k] = (line_end_excl, next_line_start)`; node 0 is the
    // virtual start of the paragraph.
    let mut starts: Vec<usize> = vec![0];
    let mut ends: Vec<usize> = Vec::new();
    for g in 1..n {
        if pure[g].is_break_point() && !pure[g - 1].is_break_point() {
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

    // Ratchets forward past a forced break (a discretionary scoring
    // `is_forced_break`, e.g. a UAX#14 `Mandatory` newline): once `j`
    // passes one, no later line may span back over it, which is exactly
    // "the breaker must end a line here" for a DP over every candidate
    // line rather than a graph search. `dp[floor]` is always finite when
    // this ratchets (it only ever advances to a `j` just computed above),
    // so no later `dp[j]` can get stuck at infinity.
    let mut floor: usize = 0;

    for j in 1..=m {
        let raw_end = ends[j - 1];
        let penalty = if raw_end < n {
            pure[raw_end].break_penalty()
        } else {
            0
        };
        // Width short-circuit: once a candidate line is wildly overfull
        // for every remaining start (natural width only grows as `i`
        // decreases further back... actually grows as we consider
        // earlier starts), stop trying earlier starts for this `j`. We
        // scan `i` from the closest (largest, tightest line) backward,
        // so the break condition below is safe: once a line is far past
        // any hope of representable badness (well beyond the shrink
        // limit) trying an even earlier `i` only makes it worse.
        for i in (floor..j).rev() {
            if dp[i].0.is_infinite() {
                continue;
            }
            let start = starts[i];
            if start > raw_end {
                // Can't happen (starts/ends interleave), but guard anyway.
                continue;
            }
            let mut metrics = measure(&line_content(&pure, start, raw_end));
            // The break itself, if it's a chosen discretionary, carries
            // `pre_break` onto the CLOSED line (the hyphen/etc. that
            // actually prints before the break) — §4 (hyphenation);
            // empty for a UAX#14-only discretionary (§3), so a no-op then.
            if raw_end < n {
                if let PureHorzBox::Discretionary { pre_break, .. } = &pure[raw_end] {
                    for b in pre_break {
                        metrics.natural += b.natural_width();
                    }
                }
            }
            let b = badness(width, &metrics);
            let d = demerits(b, penalty);
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
        if raw_end < n && pure[raw_end].is_forced_break() {
            floor = j;
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
            let content = line_content(&pure, start, raw_end);
            layout_line(ctx, content, width, idx + 1 == line_count)
        })
        .collect()
}

/// Trailing glue-or-discretionary never justifies anything and is dropped
/// from a line, except a trailing `OuterFil` (which is how a paragraph's
/// final stretch is represented, and must stay so the last line can absorb
/// slack without being force-justified). Only the *last* line's raw range
/// can have one of these at its tail in the first place — see the
/// breakpoint-collapsing comment in `break_into_lines`.
fn trim_trailing_glue(line: &[PureHorzBox]) -> &[PureHorzBox] {
    let mut end = line.len();
    while end > 0 {
        match &line[end - 1] {
            PureHorzBox::OuterEmpty { .. } | PureHorzBox::Discretionary { .. } => end -= 1,
            _ => break,
        }
    }
    &line[..end]
}

/// A break never leaves discardable glue or an unchosen discretionary at the
/// very start of the next line either (the old greedy dropped any glue seen
/// while `current` was still empty); drop it here so a pathological run of
/// consecutive break-point boxes doesn't get counted as this line's content.
///
/// A leading `OuterFil` is explicitly NOT dropped — it is not discardable
/// inter-word glue but user-inserted fill (`inline-fil`), the left half of the
/// `inline-fil ++ ib ++ inline-fil` centering / `... ++ inline-fil` (right
/// half) `ib ++ inline-fil`-flush idiom (e.g. stdjareport's centered title
/// block). Dropping it here silently collapsed every centered/right-flushed
/// line to the left margin. `trim_trailing_glue` already keeps a trailing
/// `OuterFil` for the same reason; this is the symmetric leading case.
fn trim_leading_glue(line: &[PureHorzBox]) -> &[PureHorzBox] {
    let mut start = 0;
    while start < line.len()
        && matches!(
            line[start],
            PureHorzBox::OuterEmpty { .. } | PureHorzBox::Discretionary { .. }
        )
    {
        start += 1;
    }
    &line[start..]
}

/// The actual content of a line spanning `pure[start..raw_end)`, with
/// leading and trailing glue trimmed, and every `Discretionary` resolved
/// to what actually renders on this line (§4, hyphenation — `linebreak.rs`
/// module doc, "the first filler"):
/// - a discretionary the line does NOT end on renders its `no_break` slot
///   (spliced in place, matching `measure`'s treatment of one that survives
///   into a line's interior — shouldn't normally happen since discretionaries
///   are break candidates, but a run of several collapses to just the
///   first, see `break_into_lines`'s comment, so later ones in the run can
///   land here as ordinary un-taken candidates);
/// - the break this line WAS chosen to end on (`pure[raw_end]`, only when
///   `raw_end < pure.len()`) contributes its `pre_break` slot at the line's
///   end (the hyphen prints here);
/// - the break the PREVIOUS line was chosen to end on (`pure[start - 1]`,
///   only when `start > 0`) contributes its `post_break` slot at this
///   line's start (continuation text after the hyphen).
/// Every slot is empty for a UAX#14-only discretionary (§3), so this is
/// behavior-identical to the old borrow-only version until §4 fills them.
fn line_content(pure: &[PureHorzBox], start: usize, raw_end: usize) -> Vec<PureHorzBox> {
    let mut out = Vec::new();
    if start > 0 {
        if let PureHorzBox::Discretionary { post_break, .. } = &pure[start - 1] {
            out.extend(post_break.iter().cloned());
        }
    }
    for bx in trim_trailing_glue(trim_leading_glue(&pure[start..raw_end])) {
        if let PureHorzBox::Discretionary { no_break, .. } = bx {
            out.extend(no_break.iter().cloned());
        } else {
            out.push(bx.clone());
        }
    }
    if raw_end < pure.len() {
        if let PureHorzBox::Discretionary { pre_break, .. } = &pure[raw_end] {
            out.extend(pre_break.iter().cloned());
        }
    }
    out
}

/// Assign x offsets, justifying interior lines by distributing slack into
/// glue (`OuterFil` absorbs all positive slack; otherwise stretchables or
/// shrinkables share it proportionally). The last line stays ragged: it is
/// never force-*stretched* to fill the width, but it is still *shrunk* if
/// overfull, since shrink represents real interword compressibility, not
/// justification.
fn layout_line(ctx: &Context, line: Vec<PureHorzBox>, width: Length, is_last: bool) -> VertBox {
    let (contents, height, depth) = justify_line(line, width, is_last);
    // EXPERIMENT: an all-glue line (e.g. `line-break ctx inline-fil`, used as
    // a pure spacer with its own paragraph-margin skip) draws nothing and
    // should occupy zero vertical extent — no strut.
    VertBox::Line {
        height,
        depth,
        leading: ctx.leading,
        contents,
    }
}

/// `LineBreak.fit hblstwithpads wid` (tabular.ml:270/287,
/// docs/plans/table-subsystem.md §1) — fit `content` (already
/// padding-wrapped by the caller, `tabular::solidify_tabular`) to exactly
/// `width`, distributing slack into glue/`inline-fil` exactly as
/// `justify_line` does for an ordinary paragraph line (so `inline-fil ++ …
/// ++ inline-fil` centers a cell). Unlike `layout_line`, this takes no
/// `Context`: a table cell has no font-size fallback to lean on (the grid
/// solver never threads one, matching upstream's `BackendTabular`), so
/// height/depth come from `natural_metrics` instead of the all-glue
/// fallback. Always justifies as an interior (non-final) line — a cell's
/// content is never "ragged" the way a paragraph's last line is.
pub fn fit_cell(content: Vec<HorzBox>, width: Length) -> (Vec<(Length, PureHorzBox)>, Length, Length) {
    let (_, height, depth) = natural_metrics(&content);
    let pure: Vec<PureHorzBox> = content.into_iter().map(|HorzBox::Pure(p)| p).collect();
    let (contents, _, _) = justify_line(pure, width, false);
    (contents, height, depth)
}

/// The shared position-assignment core of `layout_line`/`fit_cell`: returns
/// each box's `x` offset alongside the line's own height/depth (computed
/// the same way `layout_line` always has — callers needing a `Context`-based
/// all-glue fallback apply it on top, see `layout_line` above).
fn justify_line(
    line: Vec<PureHorzBox>,
    width: Length,
    is_last: bool,
) -> (Vec<(Length, PureHorzBox)>, Length, Length) {
    let natural: Length = line
        .iter()
        .map(|b| b.natural_width())
        .fold(Length::ZERO, |acc, w| acc + w);
    let slack = width - natural;

    let fil_count = line
        .iter()
        .filter(|b| matches!(b, PureHorzBox::OuterFil | PureHorzBox::GraphicsOuter { .. }))
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

    for mut bx in line {
        let advance = match &mut bx {
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
            PureHorzBox::Image { width, height: h, .. } => {
                height = height.max(*h);
                // An image sits entirely on the baseline: it contributes to
                // the line's height but never its depth. `depth` only ever
                // grows via `.max` and starts at `ZERO`, so this is a no-op
                // today — kept explicit (matching the plan) so the "images
                // have zero depth" decision reads as deliberate rather than
                // an omission if `depth` ever gains a different starting
                // point.
                depth = depth.max(Length::ZERO);
                *width
            }
            // Not chosen as this line's break (it would have been excluded
            // from `line` entirely otherwise, see `line_content`), so it
            // renders as `no_break` — empty for §3, hence zero-width.
            PureHorzBox::Discretionary { .. } => Length::ZERO,
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
            // `inline-graphics-outer`: shares slack equally with real fils
            // (upstream `Fils(nfil)` counts both, `fil_count` above), and
            // WRITES the resolved per-fil share back into the box — read
            // back by the lang-side post-pass (`resolve_outer_graphics_in_
            // contents`, rustyfi-lang's primitives) once this line is done.
            PureHorzBox::GraphicsOuter {
                height: h,
                depth: d,
                width: w,
                ..
            } => {
                height = height.max(*h);
                depth = depth.max(*d);
                let adv = if fil_count > 0 && slack.is_positive() {
                    slack * (1.0 / fil_count as f64)
                } else {
                    Length::ZERO
                };
                *w = adv;
                adv
            }
            // Unlike `Image` (all height, zero depth), a math run grows
            // *both* line dimensions: a superscript raises `height`, a
            // subscript deepens `depth` (docs/plans/math-engine.md §Slice 1).
            PureHorzBox::Math {
                width,
                height: h,
                depth: d,
                ..
            } => {
                height = height.max(*h);
                depth = depth.max(*d);
                *width
            }
            // Zero-width, zero-height, zero-depth marker (`is_glue ==
            // false`, like `Image`/`FixedEmpty`); fired lang-side, after
            // placement, by `fire_hooks`.
            PureHorzBox::HookPageBreak { .. } => Length::ZERO,
            // Like `Graphics` (§4 of docs/plans/table-subsystem.md): a
            // tabular box can be tall, so it drives the line's height/depth
            // exactly the same way.
            PureHorzBox::Tabular(tab) => {
                height = height.max(tab.height);
                depth = depth.max(tab.depth);
                tab.width
            }
            // `embed-block-top`/`embed-block-breakable`'s carried block
            // (docs/plans/context-box-prims.md §Slice 1 rows 7-8): same
            // height/depth-driving shape as `Graphics`/`Tabular` above.
            PureHorzBox::EmbeddedBlock {
                width,
                height: h,
                depth: d,
                ..
            } => {
                height = height.max(*h);
                depth = depth.max(*d);
                *width
            }
            // An inline frame is exactly as "tall" as it reports (padding
            // already folded into `height`/`depth` by `make_inline_frame`),
            // same height/depth-driving shape as `Graphics`/`Tabular` above.
            PureHorzBox::Frame {
                width,
                height: h,
                depth: d,
                ..
            } => {
                height = height.max(*h);
                depth = depth.max(*d);
                *width
            }
            // Zero-width/height/depth marker (`is_glue == false`); read back
            // by `fire_hooks`, after placement.
            PureHorzBox::FrameMarker { .. } => Length::ZERO,
            // Zero-width/height/depth marker (`is_glue == false`); extracted
            // and bottom-placed by `chop_page` at page-commit time.
            PureHorzBox::Footnote { .. } => Length::ZERO,
            // Zero-width/height/depth marker (`is_glue == false`);
            // `docs/plans/design-reflow-s4-lists.md` §4.3 — read only by the
            // reflow HTML walker.
            PureHorzBox::InlineMark(_) => Length::ZERO,
        };
        contents.push((x, bx));
        x += advance;
    }

    (contents, height, depth)
}
