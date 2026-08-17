//! Slice 1 lowering unit tests (docs/plans/math-engine.md): `read_math`
//! walks an elaborated `MathElem` tree straight to a `PureHorzBox::Math`
//! (fixed superscript shift/scale, a minimal Bin/Rel spacer, no MATH table)
//! — no parser or elaborator involved, mirroring `prims_phase4.rs`'s style.

use rustyfi_backend::{Context, FontKey, FontMetrics, Length, MathGlyph, PureHorzBox};
use rustyfi_lang::ast::MathElem;
use rustyfi_lang::eval::Interp;
use rustyfi_lang::primitives::read_math;

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

fn glyphs_of(elems: &[MathElem]) -> Vec<MathGlyph> {
    let mono = Mono;
    let mut interp = Interp::new(&mono);
    let ctx = Context::initial(Length::pt(400.0));
    match read_math(&mut interp, &ctx, elems).expect("read_math should succeed") {
        PureHorzBox::Math { glyphs, .. } => glyphs,
        other => panic!("expected PureHorzBox::Math, got {other:?}"),
    }
}

#[test]
fn superscript_is_raised_and_scaled_down() {
    let ctx_font_size = Length::pt(12.0);
    let elems = vec![MathElem::Sup(
        Box::new(MathElem::Chars("x".to_string())),
        vec![MathElem::Chars("2".to_string())],
    )];
    let glyphs = glyphs_of(&elems);
    assert_eq!(glyphs.len(), 2, "one glyph for 'x', one for '2'");
    let (base, sup) = (&glyphs[0], &glyphs[1]);
    assert_eq!(base.text, "x");
    assert_eq!(sup.text, "2");
    assert_eq!(base.dy, Length::ZERO);
    assert!(
        sup.dy > Length::ZERO,
        "superscript must be raised: dy = {:?}",
        sup.dy
    );
    assert!(
        sup.info.size < ctx_font_size,
        "superscript must be set smaller: {:?} vs {:?}",
        sup.info.size,
        ctx_font_size
    );
    assert!(
        sup.dx >= base.width,
        "superscript must start at/after the base's width: {:?} vs {:?}",
        sup.dx,
        base.width
    );
}

#[test]
fn binary_operator_gets_spacing_on_both_sides() {
    let elems = vec![
        MathElem::Chars("a".to_string()),
        MathElem::Chars("+".to_string()),
        MathElem::Chars("b".to_string()),
    ];
    let glyphs = glyphs_of(&elems);
    assert_eq!(glyphs.len(), 3);
    let (a, plus, b) = (&glyphs[0], &glyphs[1], &glyphs[2]);
    assert_eq!(a.text, "a");
    assert_eq!(plus.text, "+");
    assert_eq!(b.text, "b");
    // Bin spacing on both sides of '+': it starts strictly past `a`'s own
    // width (extra advance inserted before it), and `b` starts strictly past
    // `+`'s own width (extra advance inserted after it).
    assert!(
        plus.dx > a.dx + a.width,
        "'+' should get extra space after 'a': {:?} vs {:?}",
        plus.dx,
        a.dx + a.width
    );
    assert!(
        b.dx > plus.dx + plus.width,
        "'b' should get extra space after '+': {:?} vs {:?}",
        b.dx,
        plus.dx + plus.width
    );
}
