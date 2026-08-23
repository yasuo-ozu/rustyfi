//! Which font size each piece of CJK spacing scales against, and which pieces
//! are elastic at all.
//!
//! Two independent divergences from `convertText.ml` used to live here, and
//! they only show up together — each one alone is invisible in the corpus, and
//! correcting only the second was measured once and rejected for making things
//! worse. Both are about a line's STRETCH BUDGET, which is what fixes the
//! justified position of every glyph on a Japanese line.
//!
//! 1. **The Latin↔CJK inter-script space is RIGID.** `default_script_space_map`
//!    (`primitives.cppo.ml:488`) carries `(0.24, 0.08, 0.16)`, so the port gave
//!    the glue 0.08em of shrink and 0.16em of stretch. But
//!    `pure_space_between_scripts` (`convertText.ml:41-50`) spends the triple
//!    as
//!
//!    ```ocaml
//!    Some(LBAtom((natural (size *% r0), size *% r1, size *% r2), EvHorzEmpty))
//!    ```
//!
//!    and `LBAtom`'s first field is `metrics = length_info * length * length`
//!    (`lineBreakBox.ml:7`) — *(width info, height, depth)*. `natural wid`
//!    (`lineBreakBox.ml:54-59`) builds a width info with zero shrink and zero
//!    stretch, so `r1` and `r2` land in the height and depth slots and never
//!    reach the glue. The sibling `pure_halfwidth_space_soft`
//!    (`convertText.ml:83-85`) shows the correct shape one screen away:
//!    `make_width_info` for the elasticity, `Length.zero, Length.zero` for
//!    height and depth. The commented-out predecessor at `convertText.ml:58`
//!    misplaces them the same way, so v0.0.6 has never had an elastic
//!    inter-script space.
//!
//! 2. **The JLreq kerns and class spaces scale by the SCRIPT-CORRECTED size,
//!    `adjacent_space` by the RAW one.** `halfwidth_kern`/`quarterwidth_kern`
//!    (`convertText.ml:110-118`) and `pure_space_between_classes`
//!    (`convertText.ml:196-198`) all take `get_corrected_font_size ctx script`
//!    = font size × the script's `font_scheme` ratio; `adjacent_space`
//!    (`convertText.ml:101-106`) takes `ctx.font_size` unscaled. Scaling
//!    everything by the raw size made every class space 13.6% too elastic
//!    under stdja's 0.88 CJK ratio, and punctuation carries ten times the
//!    stretch of an ordinary inter-character gap.
//!
//! The natural width of a punctuation pair is the same either way — the kern
//! and the class space that pays it back scale together, so the error cancels
//! — which is why `nakaten_kern_natural_width_uses_corrected_size` reaches for
//! `・あ`: the nakaten is the one junction where a kern is emitted and NO class
//! space pays it back, so the size reaches a natural width. Everywhere else
//! the corrected-size bug is visible only in elasticity, and so only in a
//! JUSTIFIED line — which is exactly why it survived so long.

use rustyfi_backend::{Context, FontKey, FontMetrics, HorzBox, Length, PureHorzBox, Script};
use rustyfi_lang::eval::Interp;
use rustyfi_lang::primitives;
use rustyfi_lang::quoted::IText;
use rustyfi_lang::value::Env;

/// Every char is half an em wide (mirrors `cjk_prevented_break_glue.rs`), so no
/// real font data is needed — none of these assertions is about a glyph.
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

/// A 12pt context whose CJK faces carry DIFFERENT `font_scheme` ratios, so a
/// test can tell `size_a` from `size_b` and both from the raw font size.
/// `HanIdeographic` gets stdja's real 0.88 (a 10.56pt em); `Kana` gets 0.5 (a
/// 6pt em), which no real class file would use but which makes a wrong side
/// impossible to miss.
fn ctx_with_split_ratios() -> Context {
    let mut ctx = Context::initial(Length::pt(200.0));
    ctx.font_scheme[Script::HanIdeographic as usize].ratio = 0.88;
    ctx.font_scheme[Script::Kana as usize].ratio = 0.5;
    assert_eq!(
        ctx.font_size,
        Length::pt(12.0),
        "the arithmetic below assumes it"
    );
    ctx
}

fn glues(boxes: &[HorzBox]) -> Vec<PureHorzBox> {
    boxes
        .iter()
        .filter_map(|HorzBox::Pure(p)| match p {
            PureHorzBox::OuterEmpty { .. } => Some(p.clone()),
            _ => None,
        })
        .collect()
}

/// Every `no_break` box of every `Discretionary`, in order.
fn no_break_boxes(boxes: &[HorzBox]) -> Vec<PureHorzBox> {
    boxes
        .iter()
        .flat_map(|HorzBox::Pure(p)| match p {
            PureHorzBox::Discretionary { no_break, .. } => no_break.clone(),
            _ => Vec::new(),
        })
        .collect()
}

fn assert_pt(actual: Length, expected: f64, what: &str) {
    assert!(
        (actual.0 - expected).abs() < 1e-9,
        "{what}: expected {expected}pt, got {}pt",
        actual.0
    );
}

// ---------------------------------------------------------------------------
// 1. The inter-script space is rigid.
// ---------------------------------------------------------------------------

/// `Aあ` — a Latin/Kana junction inside one text run. One glue, natural
/// 0.24 × 12pt, and NO elasticity.
#[test]
fn interscript_space_is_rigid() {
    let ctx = Context::initial(Length::pt(200.0));
    let boxes = boxes_for(&ctx, "Aあ");
    let gs = glues(&boxes);
    assert_eq!(gs.len(), 1, "exactly one inter-script glue: {gs:?}");
    match &gs[0] {
        PureHorzBox::OuterEmpty {
            natural,
            shrinkable,
            stretchable,
        } => {
            assert_pt(*natural, 0.24 * 12.0, "inter-script natural");
            assert_eq!(
                *shrinkable,
                Length::ZERO,
                "`natural (size *% r0)` has zero shrink; r1 is a HEIGHT upstream"
            );
            assert_eq!(
                *stretchable,
                Length::ZERO,
                "…and zero stretch; r2 is a DEPTH upstream"
            );
        }
        other => panic!("expected glue, got {other:?}"),
    }
}

/// The same holds in the other direction, and the glue stays a legal break —
/// upstream wraps it in `discretionary_if_breakable` (`convertText.ml:228`)
/// exactly as it does the elastic glues, so making it rigid must not cost the
/// breaker a candidate.
#[test]
fn interscript_space_is_rigid_cjk_to_latin_and_still_breakable() {
    let ctx = Context::initial(Length::pt(200.0));
    let boxes = boxes_for(&ctx, "あA");
    let gs = glues(&boxes);
    assert_eq!(gs.len(), 1, "exactly one inter-script glue: {gs:?}");
    assert!(
        gs[0].is_break_point(),
        "a rigid glue is still a break opportunity"
    );
    match &gs[0] {
        PureHorzBox::OuterEmpty {
            natural,
            shrinkable,
            stretchable,
        } => {
            assert_pt(*natural, 0.24 * 12.0, "inter-script natural");
            assert_eq!(*shrinkable, Length::ZERO);
            assert_eq!(*stretchable, Length::ZERO);
        }
        other => panic!("expected glue, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 2. Corrected size for the JLreq layer, raw size for `adjacent_space`.
// ---------------------------------------------------------------------------

/// `、あ` — `(JLCM, _) -> hwsoft1` (`convertText.ml:211`), whose size is
/// `size1`, the LEFT character's corrected size. `、` classifies as
/// `HanIdeographic`, so that is 12 × 0.88 = 10.56pt and the elasticity is
/// 0.25 × 10.56 = 2.64pt — not 0.25 × 12 = 3.0pt (the raw size, the bug) and
/// not 0.25 × 6 = 1.5pt (the RIGHT character's corrected size, the wrong side).
#[test]
fn class_space_elasticity_uses_the_left_chars_corrected_size() {
    let ctx = ctx_with_split_ratios();
    let nb = no_break_boxes(&boxes_for(&ctx, "、あ"));
    let glue = nb
        .iter()
        .find_map(|b| match b {
            PureHorzBox::OuterEmpty {
                shrinkable,
                stretchable,
                ..
            } => Some((*shrinkable, *stretchable)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no class-space glue in {nb:?}"));
    assert_pt(glue.0, 0.25 * 10.56, "hwsoft shrink");
    assert_pt(glue.1, 0.25 * 10.56, "hwsoft stretch");
}

/// `・あ` — the nakaten is `JLMD`, which `pure_space_between_classes` matches
/// on NEITHER side, so it falls to the catch-all `None` (`convertText.ml:217`)
/// and no class space is put back. `ideographic_single`'s
/// `quarterwidth_kern` (`convertText.ml:280`) therefore stands alone, and the
/// corrected size shows up as a NATURAL width: `・` sits in the Katakana block,
/// so it classifies as `Kana` and the kern is −0.25 × (12 × 0.5) = −1.5pt, not
/// −0.25 × 12 = −3pt.
///
/// This is the only assertion here that a natural width can make. At every
/// other punctuation junction the kern and the class space paying it back
/// scale by the same size and the error cancels exactly — `、あ` below emits no
/// kern box at all, because −0.5em + 0.5em is zero whichever em you use. That
/// cancellation is why the corrected-size bug was invisible in natural
/// metrics and showed up only in a justified line's glyph positions.
///
/// `」。`, the obvious candidate (`(JLCP, JLFS)` is one of the two explicit
/// `None` rows, `convertText.ml:209`), does NOT work: LB13 forbids a break
/// before `。`, and at a prevented boundary this port deliberately emits the
/// elastic half only — see `cjk_prevented_break_glue.rs`.
#[test]
fn nakaten_kern_natural_width_uses_corrected_size() {
    let ctx = ctx_with_split_ratios();
    let nb = no_break_boxes(&boxes_for(&ctx, "・あ"));
    let kern = nb
        .iter()
        .find_map(|b| match b {
            PureHorzBox::FixedEmpty { width } => Some(*width),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no kern in {nb:?}"));
    assert_pt(kern, -0.25 * 6.0, "`・`'s quarterwidth_kern");
}

/// The mirror of `class_space_elasticity_uses_the_left_chars_corrected_size`:
/// `あ（` is `(_, JLOP) -> hwsoft2` (`convertText.ml:207`), whose size is
/// `size2`, the RIGHT character's. `（` classifies as `HanIdeographic`
/// (0.88 → 10.56pt) and `あ` as `Kana` (0.5 → 6pt), so 0.25 × 10.56 = 2.64pt
/// is right and 0.25 × 6 = 1.5pt would be the wrong side. Without this the
/// left-side test alone would pass on an implementation that always used
/// `size_a`.
#[test]
fn class_space_before_an_open_bracket_uses_the_right_chars_corrected_size() {
    let ctx = ctx_with_split_ratios();
    let nb = no_break_boxes(&boxes_for(&ctx, "あ（"));
    let glue = nb
        .iter()
        .find_map(|b| match b {
            PureHorzBox::OuterEmpty {
                shrinkable,
                stretchable,
                ..
            } => Some((*shrinkable, *stretchable)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no class-space glue in {nb:?}"));
    assert_pt(glue.0, 0.25 * 10.56, "hwsoft2 shrink");
    assert_pt(glue.1, 0.25 * 10.56, "hwsoft2 stretch");
}

/// The control that keeps the correction from spreading: `adjacent_space`
/// (`convertText.ml:101-106`) reads `ctx.font_size` RAW — `Length.max
/// ctx1.font_size ctx2.font_size`, with no `get_corrected_font_size` in sight.
/// Two plain kana at a 0.5 ratio still stretch by 12 × 0.025 = 0.3pt, not
/// 6 × 0.025 = 0.15pt.
#[test]
fn adjacent_space_still_uses_the_raw_font_size() {
    let ctx = ctx_with_split_ratios();
    let nb = no_break_boxes(&boxes_for(&ctx, "あい"));
    let glue = nb
        .iter()
        .find_map(|b| match b {
            PureHorzBox::OuterEmpty {
                natural,
                shrinkable,
                stretchable,
            } => Some((*natural, *shrinkable, *stretchable)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no adjacent_space glue in {nb:?}"));
    assert_eq!(glue.0, Length::ZERO, "adjacent_space has no natural width");
    assert_eq!(glue.1, Length::ZERO, "…and no shrink");
    assert_pt(glue.2, 12.0 * 0.025, "adjacent_stretch off the RAW size");
}
