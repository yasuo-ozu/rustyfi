//! A source newline between two CJK characters must typeset as if it had never
//! been written — the same boxes, the same JLreq pair spacing, the same break
//! opportunity.
//!
//! Upstream normalizes the whole character list BEFORE it classifies anything:
//! `append_property` folds `normalization_rule`
//! (`chardecoder/lineBreakDataMap.ml:143-157`) over the entire list
//! (`:315-332`), and only the result is handed to `cut_into_segment_record`
//! (`:482`). So for upstream `日本語\n日本語` and `日本語日本語` are literally the
//! same text.
//!
//! This port deleted the same characters, but did it as a `continue` INSIDE the
//! box-building loop, whose two lookaheads still read the raw text:
//!
//!   * `cjk_pair_space`'s `next_char` saw the newline, decided the boundary was
//!     not CJK-CJK, and emitted no pair spacing at all;
//!   * `boundary[after]` answered for the `語`/`\n` boundary (LB6 — never break
//!     before a line feed) instead of for the `語`/`日` one (a legal break).
//!
//! Every wrapped CJK paragraph therefore lost its pair spacing at one boundary
//! per source line, and the bundled corpus is largely Japanese.
//! `normalize_source_whitespace` now runs as its own pass ahead of
//! `uax14_boundaries`, which is upstream's order.
//!
//! The equality assertions are the regression gate; the rest of the file is the
//! vacuity control, because "delete all the whitespace" would satisfy the
//! equalities too and is NOT what upstream does.

use rustyfi_backend::{Context, FontKey, FontMetrics, HorzBox, Length, PureHorzBox};
use rustyfi_lang::eval::Interp;
use rustyfi_lang::primitives;
use rustyfi_lang::quoted::IText;
use rustyfi_lang::value::Env;

/// Every char is half an em wide (mirrors `cjk_prevented_break_glue.rs`), so
/// widths are deterministic and no real font data is needed.
struct Mono;

impl FontMetrics for Mono {
    fn advance(&self, _f: FontKey, _c: char, size: Length) -> Option<Length> {
        Some(size * 0.5)
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.75
    }
    fn descender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.25
    }
}

fn boxes_for(ctx: &Context, text: &str) -> Vec<HorzBox> {
    let mono = Mono;
    let mut interp = Interp::new(&mono);
    let elems = vec![IText::Text(text.to_string())];
    primitives::read_inline(&mut interp, ctx, &elems, &Env::root()).expect("read_inline")
}

fn ctx() -> Context {
    Context::initial(Length::pt(200.0))
}

/// The bug, at its smallest: a bare newline between two CJK runs.
#[test]
fn a_newline_between_two_cjk_runs_typesets_as_if_absent() {
    let ctx = ctx();
    assert_eq!(
        boxes_for(&ctx, "日本語\n日本語"),
        boxes_for(&ctx, "日本語日本語"),
        "a newline between two CJK characters must leave no trace"
    );
}

/// A newline followed by a trailing space is deleted WHOLE: the space matches
/// `([nonspaced; set [SP; INBR]], [], [])` (its right neighbour is the newline,
/// not a character), and the newline then matches
/// `([nonspaced; exact INBR], [], [nonspaced])`.
#[test]
fn a_space_then_a_newline_between_two_cjk_runs_typesets_as_if_absent() {
    let ctx = ctx();
    assert_eq!(
        boxes_for(&ctx, "日本語 \n日本語"),
        boxes_for(&ctx, "日本語日本語"),
        "a trailing space before the newline is deleted along with it"
    );
}

/// CONTROL, and a correction to a note this fix nearly repeated: a newline
/// followed by a continuation-line INDENT does NOT vanish entirely. Upstream's
/// `normalize` matches each rule's left context against the ALREADY-NORMALIZED
/// output (`bihead :: Alist.to_list_rev biacc`) and its right context against
/// the RAW tail, so a run of whitespace between two CJK characters collapses to
/// exactly ONE space rather than to nothing: every member but the last is
/// deleted by rule 5 (its right neighbour is more whitespace), and the last one
/// then sees the CJK character directly and is preserved as `bispace` by
/// `([nonspaced; exact SP], [bispace], [nonspaced])`. The port matched this
/// before the pass moved and still matches it — moving the rewrite earlier must
/// not quietly turn it into a full collapse.
#[test]
fn a_whitespace_run_between_two_cjk_runs_collapses_to_one_space() {
    let ctx = ctx();
    assert_eq!(
        boxes_for(&ctx, "日本語\n      日本語"),
        boxes_for(&ctx, "日本語 日本語"),
        "the LAST space of the run survives as `bispace`"
    );
    assert_ne!(
        boxes_for(&ctx, "日本語\n      日本語"),
        boxes_for(&ctx, "日本語日本語"),
        "it is a collapse to one space, not a deletion"
    );
}

fn discretionaries(boxes: &[HorzBox]) -> Vec<(i32, Vec<PureHorzBox>)> {
    boxes
        .iter()
        .filter_map(|HorzBox::Pure(p)| match p {
            PureHorzBox::Discretionary {
                penalty, no_break, ..
            } => Some((*penalty, no_break.clone())),
            _ => None,
        })
        .collect()
}

/// VACUITY CONTROL for the two above: the boundary the newline sat at is not
/// merely joined, it carries the pair spacing and the break opportunity that
/// any other CJK-CJK boundary carries. `、あ` is `AllowBreak` (LB13 forbids a
/// break before a comma, not after one) and its `hwsoft` class space nets to
/// zero natural width with 0.25em of give either way — see
/// `cjk_prevented_break_glue::allowed_boundary_is_still_a_candidate`. Before
/// the normalization pass moved ahead of `uax14_boundaries` this boundary
/// emitted NOTHING: no glue and no candidate.
#[test]
fn the_boundary_under_a_deleted_newline_carries_pair_spacing() {
    let ctx = ctx();
    let discs = discretionaries(&boxes_for(&ctx, "、\nあ"));
    assert_eq!(discs.len(), 1, "one boundary box: {discs:?}");
    let (penalty, no_break) = &discs[0];
    assert_eq!(*penalty, 0, "a legal break, as `、あ` is");
    assert_eq!(
        no_break.len(),
        1,
        "net-zero kern, so glue only: {no_break:?}"
    );
    match &no_break[0] {
        PureHorzBox::OuterEmpty {
            natural,
            shrinkable,
            stretchable,
        } => {
            assert_eq!(*natural, Length::ZERO);
            assert!((shrinkable.0 - (ctx.font_size * 0.25).0).abs() < 1e-9);
            assert!((stretchable.0 - (ctx.font_size * 0.25).0).abs() < 1e-9);
        }
        other => panic!("expected hwsoft glue, got {other:?}"),
    }
}

/// VACUITY CONTROL: a SINGLE space between two CJK characters is the one piece
/// of CJK-adjacent whitespace upstream preserves
/// (`([nonspaced; exact SP], [bispace], [nonspaced])`). A fix that just deleted
/// every space would pass the equalities above and break this.
#[test]
fn a_single_space_between_two_cjk_runs_is_still_kept() {
    let ctx = ctx();
    assert_ne!(
        boxes_for(&ctx, "日本語 日本語"),
        boxes_for(&ctx, "日本語日本語"),
        "one literal space between two CJK characters survives normalization"
    );
    assert_ne!(
        boxes_for(&ctx, "日本語 日本語"),
        boxes_for(&ctx, "日本語\n日本語"),
        "a space is kept where a newline is deleted — the two are NOT equivalent"
    );
}

/// VACUITY CONTROL: whitespace that touches no CJK character is untouched, so
/// Latin prose is byte-identical to what it was before the pass existed (a
/// newline still becomes ordinary interword glue).
#[test]
fn whitespace_away_from_cjk_is_untouched() {
    let ctx = ctx();
    assert_eq!(
        boxes_for(&ctx, "Alpha\nbeta"),
        boxes_for(&ctx, "Alpha beta"),
        "a Latin newline is interword glue, as before"
    );
    assert_ne!(
        boxes_for(&ctx, "Alpha beta"),
        boxes_for(&ctx, "Alphabeta"),
        "and it is not deleted"
    );
}

/// A CJK/Latin boundary's whitespace was already DELETED before this change
/// (the 0.24em inter-script glue supplies that spacing, not the author's
/// space) — but its CLASSIFICATION was not. `boundary[after]` still answered
/// for the deleted character's offset, i.e. LB6's "never break before a line
/// feed", so `日本語\nAlpha` got the inter-script glue with no boundary
/// `Discretionary` in front of it while `日本語Alpha` got both. Same second
/// mechanism as the pair space, one script boundary over; this fails without
/// the pass too.
#[test]
fn a_cjk_latin_boundary_across_a_newline_matches_the_unspaced_one() {
    let ctx = ctx();
    assert_eq!(
        boxes_for(&ctx, "日本語\nAlpha"),
        boxes_for(&ctx, "日本語Alpha"),
    );
    assert_eq!(
        boxes_for(&ctx, "Alpha 日本語"),
        boxes_for(&ctx, "Alpha日本語"),
    );
}
