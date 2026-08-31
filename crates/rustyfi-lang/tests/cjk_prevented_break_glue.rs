//! Inter-chunk spacing at a UAX#14-PREVENTED CJK boundary.
//!
//! `discretionary_if_breakable` (`convertText.ml:183-190`) computes a chunk
//! boundary's spacing the same way whichever the breakability, and only its
//! CONTAINER depends on the break opportunity:
//!
//! ```text
//! AllowBreak    -> LBDiscretionary(badns, id, [glue], [], [])
//! PreventBreak  -> LBPure(glue)
//! ```
//!
//! Emitting the first arm alone would give a CJK line elasticity only at the
//! subset of boundaries UAX#14 happens to allow a break at — and in Japanese
//! prose LB13 forbids a break before `、`/`。`/`」`/`）` and LB14 after
//! `（`/`「`, which is one boundary in several.
//!
//! What is pinned here:
//!
//! 1. a prevented boundary now carries a `Discretionary` whose penalty is
//!    `NO_BREAK_PENALTY`, i.e. `is_break_point() == false` — a pure box, not a
//!    candidate;
//! 2. its `no_break` slot is pure ELASTICITY: natural width zero, so the
//!    paragraph's natural metrics are untouched (the deliberate deviation
//!    documented at that emission site — upstream's rigid half is a
//!    per-CHARACTER kern this port still models per-PAIR; the rewrite-ordering
//!    half of that blocker has since been fixed, see
//!    `normalize_source_whitespace`, but the asymmetry has not);
//! 3. an allowed boundary still gets an ordinary penalty-0 `Discretionary` that
//!    IS a break point;
//! 4. the breaker consequently never breaks before `、`, however much a narrow
//!    measure would like to.

use rustyfi_backend::{
    break_into_lines, natural_metrics, Context, FontKey, FontMetrics, HorzBox, Length,
    PureHorzBox, VertBox, NO_BREAK_PENALTY,
};
use rustyfi_lang::eval::Interp;
use rustyfi_lang::primitives;
use rustyfi_lang::quoted::IText;
use rustyfi_lang::value::Env;

/// Every char is half an em wide (mirrors `rustyfi-backend/tests/linebreak.rs`),
/// so widths are deterministic and no real font data is needed.
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

fn discretionaries(boxes: &[HorzBox]) -> Vec<(i32, bool, Vec<PureHorzBox>)> {
    boxes
        .iter()
        .filter_map(|HorzBox::Pure(p)| match p {
            PureHorzBox::Discretionary {
                penalty, no_break, ..
            } => Some((*penalty, p.is_break_point(), no_break.clone())),
            _ => None,
        })
        .collect()
}

fn strings(boxes: &[HorzBox]) -> Vec<String> {
    boxes
        .iter()
        .filter_map(|HorzBox::Pure(p)| match p {
            PureHorzBox::InnerString { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// `あ、` — LB13 forbids a break before a fullwidth comma, so this boundary is
/// `PreventBreak`. It must still carry the pair's elasticity, as an unbreakable
/// pure box.
#[test]
fn prevented_boundary_carries_unbreakable_elastic_glue() {
    let ctx = Context::initial(Length::pt(200.0));
    let boxes = boxes_for(&ctx, "あ、");
    assert_eq!(strings(&boxes), vec!["あ", "、"]);

    let discs = discretionaries(&boxes);
    assert_eq!(discs.len(), 1, "exactly one boundary box: {discs:?}");
    let (penalty, is_break, no_break) = &discs[0];
    assert_eq!(
        *penalty, NO_BREAK_PENALTY,
        "a PreventBreak boundary is upstream's LBPure, not a candidate"
    );
    assert!(!*is_break, "NO_BREAK_PENALTY is not a break point");

    // Pure elasticity: one glue box, natural zero, `adjacent_stretch` of stretch.
    assert_eq!(no_break.len(), 1, "elastic half only: {no_break:?}");
    match &no_break[0] {
        PureHorzBox::OuterEmpty {
            natural,
            shrinkable,
            stretchable,
        } => {
            assert_eq!(*natural, Length::ZERO);
            assert_eq!(*shrinkable, Length::ZERO);
            // `adjacent_space`: `font_size * adjacent_stretch`, 12pt * 0.025.
            assert!(
                (stretchable.0 - (ctx.font_size * 0.025).0).abs() < 1e-9,
                "adjacent_space stretch, got {stretchable:?}"
            );
        }
        other => panic!("expected pure glue, got {other:?}"),
    }
}

/// The natural metrics of a CJK run are UNCHANGED by the prevented-boundary
/// box: it contributes zero natural width, by construction. This is what makes
/// the change elasticity-only — the rigid (kern) half of `cjk_pair_space` is
/// deliberately still omitted here.
#[test]
fn prevented_boundary_adds_no_natural_width() {
    let ctx = Context::initial(Length::pt(200.0));
    // `」。` is the extreme case: `(Close, FullStop)` is one of
    // `pure_space_between_classes`'s two `None` rows, so upstream's kern gets no
    // class space paying it back and the pair really is half an em tighter than
    // its glyphs. The port does not model that yet — pins that this wasn't
    // started by accident.
    let (w, _, _) = natural_metrics(&boxes_for(&ctx, "あ」。い"));
    // Four half-em glyphs at 12pt, nothing else.
    assert!(
        (w.0 - (ctx.font_size * 2.0).0).abs() < 1e-9,
        "natural width must be 4 * 0.5em, got {w:?}"
    );
}

/// The mirror case: `、あ` is `AllowBreak` (LB13 forbids a break BEFORE a
/// comma, not after one), so the boundary is an ordinary candidate — and it
/// carries the kern, which is where the port already applied it.
#[test]
fn allowed_boundary_is_still_a_candidate() {
    let ctx = Context::initial(Length::pt(200.0));
    let discs = discretionaries(&boxes_for(&ctx, "、あ"));
    assert_eq!(discs.len(), 1, "{discs:?}");
    let (penalty, is_break, no_break) = &discs[0];
    assert_eq!(*penalty, 0);
    assert!(*is_break);
    // `、`'s trailing kern (-0.5em) and the `(Comma, _) -> hwsoft` class space
    // (+0.5em) cancel exactly, so no rigid box is emitted — but hwsoft's shrink
    // and stretch (0.25em each) are there, which is the elasticity a Japanese
    // line mostly justifies with.
    assert_eq!(no_break.len(), 1, "net-zero kern, so glue only: {no_break:?}");
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

fn line_texts(lines: &[VertBox]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|vb| match vb {
            VertBox::Line { contents, .. } => Some(
                contents
                    .iter()
                    .filter_map(|(_, b)| match b {
                        PureHorzBox::InnerString { text, .. } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<String>(),
            ),
            _ => None,
        })
        .collect()
}

/// End to end: a measure narrow enough to want a break right before the comma
/// still breaks after it, because the prevented boundary offers no edge.
#[test]
fn breaker_never_breaks_before_a_comma() {
    let mono = Mono;
    // 6pt per glyph. A 20pt measure fits three glyphs (18pt) and not four.
    let ctx = Context::initial(Length::pt(20.0));
    let mut boxes = boxes_for(&ctx, "ああ、ああ");
    boxes.push(HorzBox::Pure(PureHorzBox::OuterFil));
    let lines = line_texts(&break_into_lines(&ctx, boxes));
    assert!(
        lines.iter().all(|l| !l.starts_with('、')),
        "no line may open with a comma: {lines:?}"
    );
    assert!(
        lines.len() > 1,
        "the measure is narrow enough to wrap: {lines:?}"
    );
    let _ = &mono;
}
