//! Paragraph breaking. Knuth–Plass optimal line breaking over glue
//! and discretionary breakpoints. The input model (a flat `Vec<HorzBox>` of
//! strings, glue and discretionaries) matches what lineBreak.ml consumes.
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
//!   line stays representable: we cap its badness at `BADNESS_TOO_LONG`
//!   instead of excluding it.
//! - v0.0.6's badness is QUANTIZED and ours is continuous. `calculate_badness`
//!   is `(abs (int_of_float (pure_ratio ** 3.))) * 10000` (lineBreak.ml:986):
//!   `int_of_float` truncates toward zero BEFORE the scale, so every
//!   permissible line with `|ratio| < 1` costs exactly 0 — and since
//!   `ratio_shrink_limit` is `-1`, that means EVERY permissible overfull line
//!   is free. Upstream is therefore indifferent across a wide band of line
//!   shapes and lets the graph decide: `LBTooShort` deletes the edge outright,
//!   and among the surviving all-zero paths `shortest_path`'s relaxation order
//!   picks one. We have neither half of that (a DP over every candidate line
//!   has no edges to delete), so `10000 * |ratio|³` is what keeps our breaker
//!   from packing to the feasibility limit.
//!
//!   Adopting upstream's exact formula here was TRIED AND MEASURED, not
//!   assumed: it regresses 6 of the 7 layout-fidelity corpus documents
//!   (`layout-tests/fidelity.py`), and quantizing moves easytable and
//!   enumitem's line counts DOWN, because `LINE_PENALTY` becomes the only
//!   thing separating partitions whose lines are all inside the free band.
//!   figbox's page gap does not close. Do not re-derive this from
//!   lineBreak.ml alone; the cost model and the graph structure are a
//!   package, and we only have the one.
//!
//!   Do NOT argue this from line counts measured by clustering `pdftotext`
//!   GLYPH BOXES: their tops and bottoms come from the font descriptor, which
//!   the two writers do not emit alike. That metric made the port look short
//!   (easytable 556 vs 592, enumitem 869 vs 885); counted from the PDF content
//!   stream the two engines set 565 vs 565 and 882 vs 883 — easytable was never
//!   short at all. The experiment's OUTCOME above stands (it was a
//!   whole-harness comparison); "we are already short, so do not go shorter"
//!   was never a valid reason for it.
//!
//!   RETRIED after `text_to_boxes` started emitting inter-CJK glue at PREVENTED
//!   boundaries as well (upstream's `LBPure` arm), in case quantizing behaves
//!   differently once every boundary is elastic. It does not: easytable 555 ->
//!   553 lines, enumitem 869 -> 868, and the gate fails 7 ways (easytable
//!   `text_match` 0.8743 -> 0.8539). More elasticity puts MORE lines inside the
//!   free band, so `LINE_PENALTY` gets more say, not less.
//!
//!   RETRIED a third time, after the inter-script space was made rigid and the
//!   JLreq class spaces were rescaled to the corrected font size — i.e. once
//!   the stretch budget of a Japanese line was itself correct, which is the
//!   input the free band is measured against. This is the closest it has come
//!   and it still does not land: every `width_p95` improves (latexcmds 0.634 ->
//!   0.631, xpath 0.160 -> 0.138, enumitem 0.547 -> 0.527, easytable 0.650 ->
//!   0.622, figbox 0.693 -> 0.665) and no line or page count moves, but figbox
//!   DROPS a character (`chars_missing` 0 -> 1) and the gate fails on it. The
//!   diagnosis below is unchanged and is the reason: a free band is only
//!   survivable with a search that breaks ties the way upstream's does, and
//!   tightening the cost model cannot supply one.
//!
//!   And a note on what the remaining gap actually is, so the next reader does
//!   not look for a cost that closes it. Upstream's weight is `badness +
//!   pnltybreak` with NO per-line term, so within the free band every partition
//!   of a paragraph scores exactly 0 whatever its line count. Which one comes
//!   out is decided by `FlowGraph.shortest_path`: labels only ever improve on a
//!   STRICT `<` (`flowGraph.ml:200`), so each vertex keeps the first parent that
//!   reached it, and the pop order among all-zero distances comes from a
//!   `Pairing_heap` seeded by `MainTable.iter` — a `Hashtbl`. Upstream's break
//!   placement inside the free band is therefore hash order, not a preference,
//!   and this DP's "minimize |ratio|" is a substitute for indifference rather
//!   than an approximation of a target. It systematically packs to the
//!   feasibility limit; upstream lands somewhere arbitrary short of it. That is
//!   the line-packing floor the port's notes record as proven.

use crate::context::Context;
use crate::hbox::{HorzBox, PureHorzBox, FORCED_BREAK_PENALTY};
use crate::length::Length;
use crate::vbox::VertBox;

/// A UAX#14 break opportunity's kind, reduced to the two outcomes the
/// paragraph breaker needs (v0.0.6's ~40-rule engine over `LineBreak.txt`
/// classes, `ref:src/chardecoder/lineBreakDataMap.ml`, collapses the same
/// way into `append_break_opportunity`'s direct/mandatory distinction).
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
/// tailoring (`ref:src/chardecoder/scriptDataMap.ml`) for the evaluated
/// alternatives.
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

/// Classic Knuth–Plass default line penalty. NOT a lineBreak.ml constant:
/// v0.0.6's edge weight is `badness + pnltybreak`, where `pnltybreak` comes
/// from a `HorzDiscretionary`'s own penalty (lineBreak.ml:1012) — the same
/// role our `Discretionary::penalty` plays via `demerits`. We adopt TeX's
/// classic default line penalty as the flat part, folded into `demerits =
/// (LINE_PENALTY + badness)^2 [+/- penalty^2]`.
const LINE_PENALTY: f64 = 10.0;

/// SATySFi's ratio limits (lineBreak.ml:507-508): a line stretched beyond
/// `+2.0` is `LBTooShort` (dropped); shrunk beyond `-1.0` is `LBTooLong`.
const RATIO_STRETCH_LIMIT: f64 = 2.0;
const RATIO_SHRINK_LIMIT: f64 = -1.0;
/// `badness_for_too_long` (lineBreak.ml:989) — the cost of a kept `LBTooLong`
/// line; also the cap for the elastic `|ratio|³·10000` badness.
const BADNESS_TOO_LONG: f64 = 100_000.0;
/// A dropped (`LBTooShort`) line: huge so the DP never prefers it, but finite
/// so a paragraph with no feasible partition still typesets.
const BADNESS_DROPPED: f64 = 1.0e12;

/// A candidate line's shape, used both to score it (badness/demerits) and
/// to lay it out once chosen.
struct LineMetrics {
    natural: Length,
    stretch: Length,
    shrink: Length,
    has_fil: bool,
    /// Natural width contributed by CJK (ideographic/kana/CJK-punctuation)
    /// glyphs on this line. Accumulated but NOT scored.
    cjk_natural: Length,
    /// Whether the line contains any real (breakable) interword glue
    /// (`OuterEmpty`). Distinguishes a rigid-but-spaced line (monospace/`+code`
    /// via `set-space-ratio r 0 0`) from unspaced CJK (which breaks between
    /// characters, not on glue) — the former MUST wrap rather than overflow.
    has_glue: bool,
}

impl LineMetrics {
    fn empty() -> LineMetrics {
        LineMetrics {
            natural: Length::ZERO,
            stretch: Length::ZERO,
            shrink: Length::ZERO,
            has_fil: false,
            has_glue: false,
            cjk_natural: Length::ZERO,
        }
    }

    fn push(&mut self, bx: &PureHorzBox) {
        match bx {
            PureHorzBox::InnerString { width, text, .. } => {
                self.natural += *width;
                if text.chars().any(is_cjk) {
                    self.cjk_natural += *width;
                }
            }
            PureHorzBox::OuterEmpty {
                natural: n,
                shrinkable,
                stretchable,
            } => {
                self.natural += *n;
                self.stretch += *stretchable;
                self.shrink += *shrinkable;
                self.has_glue = true;
            }
            PureHorzBox::OuterFil => self.has_fil = true,
            PureHorzBox::FixedEmpty { width } => self.natural += *width,
            PureHorzBox::Image { width, .. } => self.natural += *width,
            // A discretionary that does NOT end this line renders its
            // `no_break` slot (upstream's `get_leftmost/rightmost` no-break
            // choice) — empty for every UAX#14-only discretionary.
            PureHorzBox::Discretionary { no_break, .. } => {
                for b in no_break {
                    self.natural += b.natural_width();
                }
            }
            PureHorzBox::Graphics { width, .. } => self.natural += *width,
            // Counted as a fil for width purposes (upstream `Fils(1)`).
            PureHorzBox::GraphicsOuter { .. } => self.has_fil = true,
            PureHorzBox::Math { width, .. } => self.natural += *width,
            // Zero-width marker; fired lang-side after placement.
            PureHorzBox::HookPageBreak { .. } => {}
            PureHorzBox::Tabular(tab) => self.natural += tab.width,
            PureHorzBox::EmbeddedBlock { width, .. } => self.natural += *width,
            PureHorzBox::Frame { width, .. } => self.natural += *width,
            PureHorzBox::FrameMarker { .. } => {}
            // The frame's width/stretch/shrink are whatever its spliced boxes
            // contribute, so the marker must add nothing or it double-counts.
            PureHorzBox::InlineFrameMarker { .. } => {}
            // Zero-width marker; fired to the page bottom by `chop_page`.
            PureHorzBox::Footnote { .. } => {}
            PureHorzBox::InlineMark(_) => {}
        }
    }
}

fn measure(line: &[PureHorzBox]) -> LineMetrics {
    let mut m = LineMetrics::empty();
    for bx in line {
        m.push(bx);
    }
    m
}

/// The metrics [`measure`]`(&`[`line_content`]`(pure, start, raw_end))` would
/// give, computed WITHOUT building that vector.
///
/// This is the line breaker's inner loop: the DP measures every candidate line
/// for every candidate break, and `line_content` clones each box of the span —
/// including the `String` inside every `InnerString` — purely so `measure` can
/// walk it. Measured on the corpus that was 20.6M string allocations for
/// easytable alone (23.1M boxes cloned across 426,795 calls), and the reason
/// `line-break` accounted for 92 % of that document's evaluation time.
///
/// This visits exactly the boxes `line_content` would emit, in exactly that
/// order, so every floating-point addition happens in the same order and the
/// metrics are bit-identical — not one break moves.
fn measure_range(pure: &[PureHorzBox], start: usize, raw_end: usize) -> LineMetrics {
    let mut m = LineMetrics::empty();
    if start > 0 {
        if let PureHorzBox::Discretionary { post_break, .. } = &pure[start - 1] {
            for b in post_break {
                m.push(b);
            }
        }
    }
    for bx in trim_trailing_glue(trim_leading_glue(&pure[start..raw_end])) {
        if let PureHorzBox::Discretionary { no_break, .. } = bx {
            for b in no_break {
                m.push(b);
            }
        } else {
            m.push(bx);
        }
    }
    if raw_end < pure.len() {
        if let PureHorzBox::Discretionary { pre_break, .. } = &pure[raw_end] {
            for b in pre_break {
                m.push(b);
            }
        }
    }
    m
}

/// Whether `c` is a CJK glyph — Hiragana, Katakana, CJK ideographs (incl.
/// Extension A) and CJK symbols/punctuation. The classifier behind
/// `LineMetrics::cjk_natural`.
fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{3000}'..='\u{303F}'   // CJK symbols and punctuation (、。「」…)
        | '\u{3040}'..='\u{309F}' // Hiragana
        | '\u{30A0}'..='\u{30FF}' // Katakana
        | '\u{3400}'..='\u{4DBF}' // CJK Unified Ideographs Extension A
        | '\u{4E00}'..='\u{9FFF}' // CJK Unified Ideographs
        | '\u{FF00}'..='\u{FFEF}' // Halfwidth and Fullwidth Forms
    )
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
                // Zero width (its contents are spliced siblings), but the
                // frame's padded vertical extent still feeds the outer metrics.
                PureHorzBox::InlineFrameMarker { height: h, depth: d, .. } => {
                    *height = (*height).max(*h);
                    *depth = (*depth).max(*d);
                }
                PureHorzBox::Footnote { .. } => {}
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

/// `embed-block-breakable`/`embed-block-top`'s box-sizing helper — the block
/// analog of `natural_metrics`, but SUMMED rather than maxed, since a block's
/// lines stack vertically. Each `Line` contributes its own `height`/`depth`;
/// each `Skip` adds its length to `height` only. E.g.
/// `measure_block(&[Line{h,d}, Skip(s)]) == (h+s, d)`.
pub fn measure_block(block: &[VertBox]) -> (Length, Length) {
    let mut height = Length::ZERO;
    let mut depth = Length::ZERO;
    for vb in block {
        match vb {
            VertBox::Line { height: h, depth: d, .. } => {
                height += *h;
                depth += *d;
            }
            VertBox::Skip(s) | VertBox::ParagTop(s) | VertBox::FramePad(s) => height += *s,
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
/// continuously as `10000 * |r|^3` (lineBreak.ml:986's scale, not TeX's
/// `100 * r^3`) and saturates at `BADNESS_TOO_LONG`, so a moderately-bad line
/// (`r = 2`, badness 80000) still scores far better than a catastrophic one.
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
            // `<=`, i.e. the limit ITSELF is attainable. Upstream's test is
            // `ratio_raw >= ratio_stretch_limit -> LBTooShort`
            // (lineBreak.ml:534), so upstream drops the boundary ratio; the
            // shrink side below is the mirror image (upstream `<=
            // ratio_shrink_limit`, lineBreak.ml:545, vs our `<`). Both bounds
            // are inclusive here, deliberately and symmetrically.
            //
            // This is a genuine one-value deviation, kept on evidence rather
            // than by accident. Real font metrics never land a ratio on 2.0
            // exactly, so it moves no line in any corpus document; what it does
            // move is synthetic round-number fixtures, where excluding the
            // boundary turns an ordinary justified two-word line into a dropped
            // one and leaves a single overfull line as the only representable
            // partition (`interior_lines_justify`,
            // `kp_last_line_with_fil_keeps_natural_spacing` both sit exactly on
            // it). `wraps_at_glue` documents the boundary case end to end.
            if ratio <= RATIO_STRETCH_LIMIT {
                // Within the stretch limit: SATySFi `calculate_badness`
                // (lineBreak.ml:986), `|ratio|³·10000`.
                return (10000.0 * ratio.abs().powi(3)).min(BADNESS_TOO_LONG);
            }
            // Beyond the stretch limit (`LBTooShort`): SATySFi DROPS such a
            // line, and so do we.
            //
            // A rescue here once scored a CJK-bearing line by its ABSOLUTE
            // underfullness instead, because the port modelled CJK as rigid
            // discretionaries with no inter-character glue: such a line had no
            // stretch of its own, so `LBTooShort` fired for a benign reason
            // and the DP preferred a drastically SHORT rigid line (finite
            // cost) over a near-full one (dropped, 1e12) — shredding a
            // CJK+inline-code paragraph into wildly uneven lines (a 134pt line
            // before a 442pt one). CJK now carries real `adjacent_space` glue
            // (`primitives.rs`'s `text_to_boxes`), so the premise is gone, and
            // keeping the rescue on top of real glue actively HURT: it let the
            // DP take badly underfull lines cheaply, and `layout_line` then
            // stretched them to justify, opening ~2pt gaps between adjacent CJK
            // characters where SATySFi has none. Do not reinstate it.
            return BADNESS_DROPPED;
        }
        // No elastic capacity at all. Upstream's `calculate_ratios` divides the
        // shortfall by a zero stretch, so the ratio is infinite — always past
        // `ratio_stretch_limit`, i.e. `LBTooShort`, which gets NO graph edge
        // (`lineBreak.ml:1014`). Drop it here too.
        //
        // A rigid line is NOT normally a problem: the `+code` idiom ends each
        // line with `inline-fil`, and `has_fil` above already scores those 0.
        // What this fixes is the line with neither fil NOR glue — scoring it by
        // absolute underfullness capped at `BADNESS_TOO_LONG` made a
        // DRASTICALLY short rigid line CHEAPER than a slightly overfull one, so
        // the breaker took a 108pt line on a 440pt column (badness 42_961)
        // rather than the near-perfect 434.76pt line sitting right there
        // (badness 1_144) — latexcmds' `\SATySFi;は\LaTeX;の` line.
        BADNESS_DROPPED
    } else {
        // Overfull: needs to shrink.
        if metrics.shrink.is_positive() {
            let ratio = slack / metrics.shrink;
            if ratio < RATIO_SHRINK_LIMIT {
                // `LBTooLong` (lineBreak.ml:508), scaled by the overflow — see
                // `too_long_badness` on why a flat cost is not survivable here.
                too_long_badness(slack, width)
            } else {
                (10000.0 * ratio.abs().powi(3)).min(BADNESS_TOO_LONG)
            }
        } else if metrics.has_glue {
            // Overfull with real interword glue that can't shrink (monospace/
            // `+code`, `set-space-ratio r 0 0`): breaking BEFORE the word that
            // doesn't fit is the right call, so this must dominate any
            // hyphen/line penalty. `no_stretch_badness` (below) scores overflow
            // as a cube of the overflow FRACTION — near-zero for a modest
            // overflow — which let the DP cram extra words onto an already
            // overfull line and run text clean off the page edge (visible
            // clipping in latexcmds `+code`/`\code`). Force the wrap.
            //
            // `BADNESS_TOO_LONG` deliberately: zero shrink means ANY
            // overflow is past `ratio_shrink_limit`, which is upstream's
            // `LBTooLong` — scored `badness_for_too_long = 100000`
            // (`lineBreak.ml:989/1027`). `BADNESS_INF` is only 10_000, i.e.
            // CHEAPER than a merely mediocre permissible line (ratio 1 scores
            // 10_000), so it read as "mildly loose" rather than "off the page"
            // and the DP happily overran the margin.
            too_long_badness(slack, width)
        } else {
            // No breakable glue at all (unspaced CJK) AND overfull: SATySFi's
            // `ratio_shrink_limit = -1.0` (lineBreak.ml:508) excludes any line
            // that overflows beyond its shrink capacity — with zero shrink that
            // is ANY overflow, so the breaker is forced to end the line BEFORE
            // the char that doesn't fit. The port's continuous
            // `no_stretch_badness` scored a modest CJK overflow near-zero and
            // let the DP cram, packing CJK ~0.6 line/page fuller than SATySFi
            // (easytable 18 vs 19). Force the earlier break — at upstream's
            // `LBTooLong` cost, see the sibling branch above on why
            // `BADNESS_INF` is too cheap to mean "overfull".
            too_long_badness(slack, width)
        }
    }
}

/// Whether a box puts anything on the page. Glue, kerns and the zero-width
/// markers do not; everything else does.
fn carries_ink(b: &PureHorzBox) -> bool {
    !matches!(
        b,
        PureHorzBox::OuterEmpty { .. }
            | PureHorzBox::OuterFil
            | PureHorzBox::FixedEmpty { .. }
            | PureHorzBox::FrameMarker { .. }
            | PureHorzBox::InlineFrameMarker { .. }
            | PureHorzBox::HookPageBreak { .. }
            | PureHorzBox::Discretionary { .. }
    )
}

/// Whether a candidate line is upstream's `LBTooLong` — overfull past what its
/// shrink can absorb (`calculate_ratios`, `lineBreak.ml:538-548`). Separate
/// from [`badness`] because the DP needs the CLASSIFICATION, not just the cost.
fn is_too_long(width: Length, m: &LineMetrics) -> bool {
    let slack = width - m.natural;
    if slack.0 >= 0.0 {
        return false;
    }
    if m.shrink.is_positive() {
        (slack / m.shrink) < RATIO_SHRINK_LIMIT
    } else {
        true
    }
}

/// Cost of an overfull line SATySFi would call `LBTooLong`.
///
/// Upstream scores these at a FLAT `badness_for_too_long = 100000`
/// (`lineBreak.ml:989`) and gets away with it because of a structural rule this
/// DP has no analogue for: from a given start point it adds only the FIRST
/// too-long edge and then abandons that start entirely (`is_already_too_long` /
/// `RemovalSet`, `lineBreak.ml:1017-1027`), so a longer overfull line from the
/// same start is never even evaluated.
///
/// Scoring every overfull line the same flat cost here is catastrophic: a line
/// 675pt past the margin costs exactly what one 4pt past costs, and since the
/// DP prefers fewer lines (`LINE_PENALTY`, and the fewer-lines tiebreak), it
/// swallowed an ENTIRE PARAGRAPH into one 1115pt line rather than pay for a
/// second line — the whole of latexcmds' `もしどうしても…` paragraph ran off the
/// page edge. Growing the cost with the overflow restores upstream's effective
/// ordering (the least-overfull option wins) while keeping every line
/// representable, and stays far below `BADNESS_DROPPED` so any feasible
/// partition still beats any overfull one.
fn too_long_badness(slack: Length, width: Length) -> f64 {
    let overflow = -slack.0;
    BADNESS_TOO_LONG * (1.0 + (overflow / width.0).max(0.0))
}


/// Fold a break's own penalty into its line's demerits, TeX's classic
/// formula (TeXbook ch.14): a positive penalty discourages breaking there
/// (`+ p^2`), a negative one encourages it (`- p^2`), and `<=
/// FORCED_BREAK_PENALTY` is scored plainly since the DP's `floor` already
/// guarantees the break is taken regardless of cost. Glue's implicit
/// penalty is always 0, so this is exactly today's formula whenever no
/// discretionary is involved.
fn demerits(b: f64, penalty: i32) -> f64 {
    // SATySFi's edge weight is LINEAR (`badness + pnltybreak`, lineBreak.ml:1013),
    // not TeX's squared demerit.
    let base = LINE_PENALTY + b;
    if penalty <= FORCED_BREAK_PENALTY {
        base
    } else {
        (base + penalty as f64).max(0.0)
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

    // SATySFi's UNREACHABLE fallback (lineBreak.ml:1122-1133). When the whole
    // paragraph fits on one line at its natural width but has too little
    // stretch to justify to the column, SATySFi's graph has NO permissible
    // path to the terminal — every candidate line is `LBTooShort`, which adds
    // no edge (lineBreak.ml:1015) — so `shortest_path` returns `None` and it
    // emits a SINGLE natural-width (ragged) line rather than a justified split.
    //
    // The port's DP scores rather than drops, so it would instead pick a
    // pathological word-per-line split: the single full line is over-stretched
    // (`BADNESS_DROPPED`), while each short one-word piece is merely expensive
    // (`no_stretch_badness`, bounded), and the pieces' aggregate undercuts the
    // dropped single line — putting each word on its own line (a raw
    // `line-break ... ` with no trailing `inline-fil`; every doc-class `+p`
    // appends one, so real prose and the whole corpus never reach here). Match
    // SATySFi: if the whole paragraph fits on one line yet that line can't be
    // justified (`BADNESS_DROPPED`), emit it as one natural ragged line.
    //
    // Guards: a paragraph ending in `inline-fil` justifies with badness 0 (the
    // `has_fil` branch of `badness`) so it never satisfies the condition; and a
    // paragraph carrying a forced break (mandatory newline) must keep that
    // break, so it is excluded here.
    {
        let whole = line_content(&pure, 0, n);
        let wm = measure(&whole);
        let has_forced_break = pure.iter().any(PureHorzBox::is_forced_break);
        if !has_forced_break && wm.natural <= width && badness(width, &wm) >= BADNESS_DROPPED {
            return vec![layout_line(ctx, whole, width, true)];
        }
    }

    // Legal breakpoints: a glue-or-discretionary box (`is_break_point`)
    // immediately following a box that isn't one (never at the very start
    // of a line — leading glue after a break is dropped). For each such box
    // at index `g`, a line ending there spans up to (excluding) `g`, and the
    // next line starts at `g + 1` (the box itself is discarded, as glue is).
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
    // Index of the last box that actually marks the page (see the `g > last_ink`
    // guard below).
    let last_ink = pure.iter().rposition(carries_ink).unwrap_or(0);
    for g in 1..n {
        // `is_break_point`, not a bare `matches!`: a `NO_BREAK_PENALTY`
        // discretionary is upstream's `LBPure(glue)` (`convertText.ml:190`) and
        // must not become a candidate through the `|| is_disc` clause below.
        let is_disc =
            matches!(pure[g], PureHorzBox::Discretionary { .. }) && pure[g].is_break_point();
        // A break candidate is the FIRST box of a run of break points
        // (`is_break_point && !prev-is-break-point`) — breaking at glue
        // discards that glue. BUT a `Discretionary` is ALSO a candidate even
        // when it immediately follows glue: unlike glue, breaking at a
        // discretionary does NOT discard the glue *before* it. This is exactly
        // what the `+code` idiom `text ++ inline-fil ++ discretionary` needs —
        // the line must keep its trailing `inline-fil` (to justify) and break
        // at the discretionary. Without the discretionary as its own candidate
        // the run `[fil, disc]` collapsed to the `fil`, so the break discarded
        // the fil, leaving an underfull line the DP then declined to break —
        // merging code lines and shoving them off-page via the discretionary's
        // 2×width no-break skip (whole code blocks rendered a few lines).
        // A break with NO INK after it is not a real alternative — it just
        // moves the paragraph's trailing glue onto a blank line. The terminal
        // break below already covers "end the paragraph here", so offering
        // these as well let the breaker split `[word, fil]` into an overfull
        // line PLUS an empty one once the one-overfull-edge rule (below) made
        // the single-line option unreachable.
        if g > last_ink {
            continue;
        }
        if (pure[g].is_break_point() && !pure[g - 1].is_break_point()) || is_disc {
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

    // Upstream adds at most ONE `LBTooLong` edge per source node and then drops
    // that node entirely (`is_already_too_long` / `RemovalSet`,
    // `lineBreak.ml:1017-1027`). Because destinations are visited in order of
    // increasing line width, the one edge it keeps is the LEAST overfull.
    //
    // That rule is structural, and no per-line COST can stand in for it: an
    // overfull line is worth ~`BADNESS_TOO_LONG` whatever its overflow, so a
    // partition with fewer overfull lines always wins, however badly each one
    // overruns. latexcmds' `+code` block was set as 3 lines ending at 589.0 /
    // 544.9 / 513.4 on a column ending at 515 — one line 74pt past the margin —
    // where SATySFi takes 4 lines at 519.7 / 526.0 / 519.7, each only a few
    // points over. Same for the paragraph that got swallowed into a single
    // 1115pt line.
    //
    // `j` ascends, so the first overfull `(i, j)` we meet for a given start `i`
    // is that start's least-overfull option; every later one is unreachable.
    let mut spent_overfull: Vec<bool> = vec![false; m + 1];

    for j in 1..=m {
        let raw_end = ends[j - 1];
        let penalty = if raw_end < n {
            pure[raw_end].break_penalty()
        } else {
            0
        };
        // `i` is scanned from the closest (tightest line) backward, so an
        // earlier `i` only ever makes a candidate line wider — which is what
        // makes the width short-circuit at the end of this loop safe.
        for i in (floor..j).rev() {
            if dp[i].0.is_infinite() {
                continue;
            }
            let start = starts[i];
            if start > raw_end {
                // Can't happen (starts/ends interleave), but guard anyway.
                continue;
            }
            let mut metrics = measure_range(&pure, start, raw_end);
            // The break itself, if it's a chosen discretionary, carries
            // `pre_break` onto the CLOSED line (the hyphen/etc. that
            // actually prints before the break) — hyphenation;
            // empty for a UAX#14-only discretionary, so a no-op then.
            if raw_end < n {
                if let PureHorzBox::Discretionary { pre_break, .. } = &pure[raw_end] {
                    for b in pre_break {
                        metrics.natural += b.natural_width();
                    }
                }
            }
            if is_too_long(width, &metrics) {
                if spent_overfull[i] {
                    continue; // this start already used its single overfull edge
                }
                spent_overfull[i] = true;
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
        .flat_map(|(idx, (start, raw_end))| {
            let content = line_content(&pure, start, raw_end);
            // `LBEmbeddedVertBreakable` (`lineBreak.ml:809-818`): a breakable
            // embedded block is NOT laid out as a line. Upstream flushes the
            // line accumulated so far, splices the block's own vertical boxes
            // into the vertical list as `AlreadyVert`, then starts fresh — the
            // block's vertical extent IS the gap, with no line leading of its
            // own. `prim_embed_block_breakable` fences each such block between
            // forced breaks, so it always lands alone on its own "line" here,
            // which is exactly the segment to splice.
            //
            // Wrapping it in a `layout_line` instead gave it a full leading on
            // top of its own height: latexcmds' `\linebreak` (whose block is a
            // `block-skip` of `leading - font_size`) then advanced 36.0pt where
            // SATySFi advances ~15.5pt — double-spacing every hard-broken line.
            if let Some(block) = sole_breakable_block(&content) {
                return block;
            }
            vec![layout_line(ctx, content, width, idx + 1 == line_count)]
        })
        .collect()
}

/// Trailing glue-or-discretionary never justifies anything and is dropped
/// from a line, except a trailing `OuterFil` (which is how a paragraph's
/// final stretch is represented, and must stay so the last line can absorb
/// slack without being force-justified). Only the *last* line's raw range
/// can have one of these at its tail in the first place — see the
/// breakpoint-collapsing comment in `break_into_lines`.
///
/// A `NO_BREAK_PENALTY` discretionary is exempt: it is not a breakpoint at all
/// but upstream's `LBPure(glue)`, which is never discardable.
fn trim_trailing_glue(line: &[PureHorzBox]) -> &[PureHorzBox] {
    let mut end = line.len();
    while end > 0 {
        match &line[end - 1] {
            PureHorzBox::OuterEmpty { .. } => end -= 1,
            b @ PureHorzBox::Discretionary { .. } if b.is_break_point() => end -= 1,
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
/// `NO_BREAK_PENALTY` discretionaries are exempt here too, for the reason given
/// on `trim_trailing_glue`.
fn trim_leading_glue(line: &[PureHorzBox]) -> &[PureHorzBox] {
    let mut start = 0;
    while start < line.len()
        && match &line[start] {
            PureHorzBox::OuterEmpty { .. } => true,
            b @ PureHorzBox::Discretionary { .. } => b.is_break_point(),
            _ => false,
        }
    {
        start += 1;
    }
    &line[start..]
}

/// The actual content of a line spanning `pure[start..raw_end)`, with
/// leading and trailing glue trimmed, and every `Discretionary` resolved
/// to what actually renders on this line (hyphenation — `linebreak.rs`
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
/// Every slot is empty for a UAX#14-only discretionary, so this is
/// behavior-identical to the old borrow-only version until hyphenation fills
/// them.
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

/// The inner vertical boxes of a line that holds NOTHING but one breakable
/// embedded block (plus inert zero-width markers and glue), or `None`.
/// See its caller in [`break_into_lines`].
fn sole_breakable_block(content: &[PureHorzBox]) -> Option<Vec<VertBox>> {
    let mut found: Option<&Vec<VertBox>> = None;
    for bx in content {
        match bx {
            PureHorzBox::EmbeddedBlock {
                block,
                breakable: true,
                ..
            } => {
                if found.is_some() {
                    return None; // two blocks: lay the line out normally
                }
                found = Some(block);
            }
            // Inert: carries no ink and no width of its own.
            PureHorzBox::FrameMarker { .. }
            | PureHorzBox::InlineFrameMarker { .. }
            | PureHorzBox::HookPageBreak { .. }
            | PureHorzBox::OuterEmpty { .. }
            | PureHorzBox::OuterFil
            | PureHorzBox::FixedEmpty { .. } => {}
            _ => return None,
        }
    }
    found.cloned()
}

/// Assign x offsets, justifying interior lines by distributing slack into
/// glue (`OuterFil` absorbs all positive slack; otherwise stretchables or
/// shrinkables share it proportionally). The last line stays ragged: it is
/// never force-*stretched* to fill the width, but it is still *shrunk* if
/// overfull, since shrink represents real interword compressibility, not
/// justification.
fn layout_line(ctx: &Context, line: Vec<PureHorzBox>, width: Length, is_last: bool) -> VertBox {
    let (contents, height, depth) = justify_line(line, width, is_last);
    // An all-glue line (e.g. `line-break ctx inline-fil`, used as a pure
    // spacer with its own paragraph-margin skip) draws nothing and occupies
    // zero vertical extent — no strut.
    VertBox::Line {
        height,
        depth,
        leading: ctx.leading,
        contents,
    }
}

/// `LineBreak.fit hblstwithpads wid` (tabular.ml:270/287) — fit `content`
/// (already padding-wrapped by the caller, `tabular::solidify_tabular`) to
/// exactly `width`, distributing slack into glue/`inline-fil` exactly as
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
                // today — kept explicit so the "images have zero depth"
                // decision reads as deliberate rather than an omission if
                // `depth` ever gains a different starting point.
                depth = depth.max(Length::ZERO);
                *width
            }
            // Not chosen as this line's break (it would have been excluded
            // from `line` entirely otherwise, see `line_content`), so it
            // renders as `no_break` — empty for a UAX#14-only discretionary,
            // hence zero-width.
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
            // subscript deepens `depth`.
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
            PureHorzBox::Tabular(tab) => {
                height = height.max(tab.height);
                depth = depth.max(tab.depth);
                tab.width
            }
            // `embed-block-top`/`embed-block-breakable`'s carried block
            // (rows 7-8): same height/depth-driving shape as
            // `Graphics`/`Tabular` above.
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
            // Zero-width marker; read back by `fire_hooks` after placement.
            PureHorzBox::FrameMarker { .. } => Length::ZERO,
            // Zero-WIDTH bracket whose contents are spliced siblings on this
            // same line, so it advances nothing — but it does carry the
            // frame's padded vertical extent (see the variant's doc comment),
            // which is how `paddingT`/`paddingB` reach the line's height and
            // depth now that the frame is no longer one atomic box.
            PureHorzBox::InlineFrameMarker { height: h, depth: d, .. } => {
                height = height.max(*h);
                depth = depth.max(*d);
                Length::ZERO
            }
            // Zero-width marker; extracted and bottom-placed by `chop_page`
            // at page-commit time.
            PureHorzBox::Footnote { .. } => Length::ZERO,
            // Zero-width marker; read only by the reflow HTML walker.
            PureHorzBox::InlineMark(_) => Length::ZERO,
        };
        contents.push((x, bx));
        x += advance;
    }

    (contents, height, depth)
}
