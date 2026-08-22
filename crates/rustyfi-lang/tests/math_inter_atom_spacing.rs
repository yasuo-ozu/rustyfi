//! `space_between_math_kinds` (`math.ml:319-410`) as a TABLE, not a
//! two-branch stand-in.
//!
//! Every width below is an exact arithmetic identity under the `Mono`
//! metrics stub (advance = size/2 for every char, no OpenType MATH table, so
//! `MathC::script_scale` takes its 0.7 fallback and `superscript_kern`
//! contributes exactly zero) — so each one pins ONE row of the table and
//! fails if that row is dropped, reordered past an earlier row, or given the
//! wrong ratio.
//!
//! The three properties that distinguish the real table from the stand-in it
//! replaced, and the reason each is here:
//!
//!   * the RATIOS are `primitives.cppo.ml:528-533`'s (`bin` 0.25, `rel`
//!     0.375, the other four 0.125), not 0.22/0.28 — `math_variant_class.rs`
//!     covers those two;
//!   * `Op`, `Punct`, `Inner` and `Prefix` have rows AT ALL, and the table is
//!     ASYMMETRIC — `(Rel, Close)` yields nothing because `(_, MathClose)` is
//!     matched before the relation rows, where the stand-in gave a relation
//!     space on the strength of `prev == Rel` alone;
//!   * inside a sub/superscript the whole table is SUPPRESSED except the five
//!     operator pairs, and those measure against the SCRIPT's font size.
//!     This is the error that showed up in a script-dense real document: the
//!     stand-in applied base-level spacing at the base-level size at every
//!     depth.

use rustyfi_backend::{FontKey, FontMetrics, HorzBox, Length, PureHorzBox};
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck, CompileError};

/// Advance = half the size for every char, no MATH table — the same stub
/// `math_variant_class.rs` uses, so the two files' arithmetic agrees.
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

/// The document font size every expectation below is written against, and
/// the script size the 0.7 no-MATH-table fallback derives from it.
const FS: f64 = 12.0;
const SCRIPT_FS: f64 = FS * 0.7;
/// One glyph at each of those sizes, under `Mono`.
const GLYPH: f64 = FS * 0.5;
const SCRIPT_GLYPH: f64 = SCRIPT_FS * 0.5;

/// `Length` is an `f64` newtype with no approximate comparison, and the
/// engine reaches each width by a different association of the same
/// products than an expectation written left-to-right does (a script size is
/// `12 * 0.7` there and `0.7 * 12 * 0.125` here). One ULP of slack, which is
/// four orders of magnitude below the smallest quantity any of these tests
/// distinguishes — the narrowest is a 1.05 pt operator space.
#[track_caller]
fn assert_pt(actual: Length, expected_pt: f64, what: &str) {
    assert!(
        (actual.0 - expected_pt).abs() < 1e-9,
        "{what}: expected {expected_pt} pt, got {} pt",
        actual.0
    );
}

fn width_of(body: &str) -> Length {
    let src = format!(
        "let-inline ctx \\dummy m = inline-nil\n\
         in\n\
         let ctx = get-initial-context 200pt (command \\dummy) in\n\
         {body}"
    );
    let v = run(&src).unwrap_or_else(|e| panic!("{body} should compile and evaluate: {e}"));
    match v {
        Value::InlineBoxes(boxes) => {
            assert_eq!(boxes.len(), 1, "expected exactly one box, got {boxes:?}");
            match boxes.into_iter().next().unwrap() {
                HorzBox::Pure(PureHorzBox::Math { width, .. }) => width,
                other => panic!("expected a PureHorzBox::Math, got {other:?}"),
            }
        }
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

fn run(src: &str) -> Result<Value, CompileError> {
    let file = rustyfi_syntax::parse_file(src)?;
    let env = primitives::base_env();
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = elaborate::Scope::new(&store, env.names());
    let program = elaborate::elaborate_program(&file, &scope)?;
    typecheck::typecheck(&program)?;
    let mut interp = eval::Interp::new(&Mono);
    Ok(interp.eval(&env, &rustyfi_lang::ast::debrand(&program.body, &store))?)
}

// ---------------------------------------------------------------------------
// Rows the stand-in did not have at all.
// ---------------------------------------------------------------------------

/// `(MathPunct, _)` is the FIRST arm of upstream's match, and it is one-sided:
/// `${a,b}` gets punctuation space AFTER the comma and none before it,
/// because `(Ord, Punct)` has no row and falls through to `_`. The stand-in
/// had no punctuation row at all and set all three glyphs flush.
#[test]
fn punct_spaces_after_itself_and_not_before() {
    assert_pt(
        width_of("embed-math ctx ${a,b}"),
        3.0 * GLYPH + FS * 0.125,
        "3 glyphs + ONE punct space, after the comma only",
    );
}

/// `(Ord, Op)` and `(Op, Ord)` both take `space_math_op`. `math-char MathOp`
/// is the only way to reach the class — no character in
/// `default_math_class_map` is an operator — which is why the stand-in's
/// missing operator row went unnoticed until a document that uses `\sum`,
/// `\prod` and `\int` was measured.
#[test]
fn operator_takes_op_space_on_both_sides() {
    assert_pt(
        width_of(
            "embed-math ctx (math-concat (math-char MathOrd `a`) \
             (math-concat (math-char MathOp `f`) (math-char MathOrd `b`)))"
        ),
        3.0 * GLYPH + 2.0 * FS * 0.125,
        "3 glyphs + 2 op spaces",
    );
}

/// `(MathInner, MathOrd)` takes `space_math_inner`, and `(Ord, Inner)` takes
/// it too — a fraction or a radical is `Inner`, so this is the row that
/// spaces a `\frac` away from what surrounds it.
#[test]
fn inner_takes_inner_space_on_both_sides() {
    assert_pt(
        width_of(
            "embed-math ctx (math-concat (math-char MathOrd `a`) \
             (math-concat (math-group MathInner MathInner (math-char MathOrd `i`)) \
             (math-char MathOrd `b`)))"
        ),
        3.0 * GLYPH + 2.0 * FS * 0.125,
        "3 glyphs + 2 inner spaces",
    );
}

// ---------------------------------------------------------------------------
// The table is asymmetric — ORDER of the arms is load-bearing.
// ---------------------------------------------------------------------------

/// A relation followed by a closing delimiter gets NO space: `(_, MathClose)`
/// matches first, and with `corr = NoSpace` there is not even an italics
/// correction to append — and the relation rows do not list `(Rel, Close)`
/// either, so both readings agree. The stand-in keyed on `prev == Rel` alone
/// and inserted a relation space here.
#[test]
fn close_takes_no_space_even_after_a_relation() {
    assert_pt(
        width_of(
            "embed-math ctx (math-concat (math-char MathRel `=`) \
             (math-group MathClose MathClose (math-char MathOrd `x`)))"
        ),
        2.0 * GLYPH,
        "a closing delimiter is reached before the relation rows: no space",
    );
}

/// `(MathPunct, _)` outranks `(_, MathClose)` — this is the ONE pair in the
/// whole table whose value depends on the ARMS' ORDER rather than on which
/// arms exist, since no later row lists a `Close` on the right. Move the
/// `(_, Close)` arm above the punctuation row and this reads 12 pt.
#[test]
fn punct_outranks_close_because_its_arm_comes_first() {
    assert_pt(
        width_of(
            "embed-math ctx (math-concat (math-char MathPunct `,`) \
             (math-group MathClose MathClose (math-char MathOrd `x`)))"
        ),
        2.0 * GLYPH + FS * 0.125,
        "2 glyphs + one punct space, NOT the `(_, Close)` arm's nothing",
    );
}

/// The mirror of `close_takes_no_space_even_after_a_relation`:
/// `(MathRel, MathOpen)` DOES have a row, so the asymmetry is real rather
/// than a blanket "nothing next to a delimiter".
#[test]
fn open_after_a_relation_still_takes_relation_space() {
    assert_pt(
        width_of(
            "embed-math ctx (math-concat (math-char MathRel `=`) \
             (math-group MathOpen MathOpen (math-char MathOrd `x`)))"
        ),
        2.0 * GLYPH + FS * 0.375,
        "2 glyphs + one relation space",
    );
}

// ---------------------------------------------------------------------------
// Script level.
// ---------------------------------------------------------------------------

/// Inside a superscript upstream suppresses every row but the operator ones,
/// so `${a^{b+c}}`'s script sets three glyphs flush. The stand-in applied
/// binary spacing there — and at the BASE font size, so the error was
/// 2 x 0.22 x 12 pt inside an 8.4 pt script.
#[test]
fn script_level_suppresses_binary_spacing() {
    assert_pt(
        width_of("embed-math ctx ${a^{b+c}}"),
        GLYPH + 3.0 * SCRIPT_GLYPH,
        "a script's own atoms set flush: no binary space inside a script",
    );
}

/// …with the five operator pairs as the sole exception, and they measure
/// against the SCRIPT's font size, not the ambient one — upstream's
/// `FontInfo.actual_math_font_size mathctx`. Both halves of that sentence
/// fail this test if dropped: suppressing the row gives `18.6pt`, keeping the
/// ambient size gives `21.6pt`, and the right answer is `20.7pt`.
#[test]
fn script_level_keeps_op_spacing_at_the_script_size() {
    assert_pt(
        width_of(
            "embed-math ctx (math-sup (math-char MathOrd `a`) \
             (math-concat (math-char MathOrd `b`) \
             (math-concat (math-char MathOp `f`) (math-char MathOrd `c`))))"
        ),
        GLYPH + 3.0 * SCRIPT_GLYPH + 2.0 * SCRIPT_FS * 0.125,
        "the base + 3 script glyphs + 2 op spaces AT THE SCRIPT SIZE",
    );
}

/// The control for the pair above: the identical atom list at BASE level
/// takes the identical two operator spaces, but sized by the base font — so
/// the script test above is measuring the size and not merely the row.
#[test]
fn the_same_list_at_base_level_scales_its_op_spacing_by_the_base_size() {
    assert_pt(
        width_of(
            "embed-math ctx (math-concat (math-char MathOrd `b`) \
             (math-concat (math-char MathOp `f`) (math-char MathOrd `c`)))"
        ),
        3.0 * GLYPH + 2.0 * FS * 0.125,
        "3 glyphs + 2 op spaces, sized by the BASE font",
    );
}
