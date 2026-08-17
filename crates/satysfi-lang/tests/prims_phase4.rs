//! Phase-4-part-1 primitive inventory tests: the context-op and
//! box-combinator primitives added ahead of a future `.saty`-defined
//! `document`/`+p`/`\emph`. Follows `eval_phase2.rs`'s style — `Ast` apply
//! chains driven through `eval::Interp` and `primitives::base_env()`, no
//! parser involved.

use satysfi_backend::{FontKey, FontMetrics, HorzBox, Length, PureHorzBox, VertBox};
use satysfi_lang::ast::Ast;
use satysfi_lang::eval;
use satysfi_lang::prim_types;
use satysfi_lang::primitives;
use satysfi_lang::value::Value;
use satysfi_syntax::Span;

struct Mono;

impl FontMetrics for Mono {
    fn advance(&self, _f: FontKey, c: char, size: Length) -> Option<Length> {
        if c.is_ascii() {
            Some(size * 0.5)
        } else {
            None
        }
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.75
    }
    fn descender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.25
    }
}

// ---- small Ast-builder helpers (mirrors eval_phase2.rs) --------------------

fn var(name: &str) -> Ast {
    Ast::Var(name.to_string(), Span::default())
}

fn app1(f: Ast, a: Ast) -> Ast {
    Ast::Apply(Box::new(f), Box::new(a))
}

fn app2(name: &str, a: Ast, b: Ast) -> Ast {
    app1(app1(var(name), a), b)
}

fn app3(name: &str, a: Ast, b: Ast, c: Ast) -> Ast {
    app1(app1(app1(var(name), a), b), c)
}

fn len(pt: f64) -> Ast {
    Ast::Length(Length::pt(pt))
}

/// `get-initial-context width ()` — the math-command argument is ignored
/// at runtime (see primitives.rs's `prim_get_initial_context`), so `Unit`
/// stands in for it.
fn initial_ctx(width_pt: f64) -> Ast {
    app2("get-initial-context", len(width_pt), Ast::Unit)
}

fn run(ast: &Ast) -> Value {
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    interp.eval(&env, ast).expect("evaluation should succeed")
}

fn assert_len_close(v: Value, expect_pt: f64) {
    match v {
        Value::Length(l) => assert!(
            (l.0 - expect_pt).abs() < 1e-9,
            "expected {expect_pt}pt, got {}pt",
            l.0
        ),
        other => panic!("expected a length, got {other:?}"),
    }
}

// ============================================================================
// Context ops
// ============================================================================

#[test]
fn set_and_get_font_size_round_trip() {
    // get-font-size (set-font-size 20pt (get-initial-context 100pt ()))
    let ast = app1(
        var("get-font-size"),
        app2("set-font-size", len(20.0), initial_ctx(100.0)),
    );
    assert_len_close(run(&ast), 20.0);
}

#[test]
fn get_initial_context_seeds_font_size_default() {
    // v0.0.6's `get_pdf_mode_initial_context` defaults font_size to 12pt
    // (primitives.cppo.ml:500), same as `Context::initial`.
    let ast = app1(var("get-font-size"), initial_ctx(100.0));
    assert_len_close(run(&ast), 12.0);
}

#[test]
fn set_leading_round_trip_via_context_value() {
    let ast = app2("set-leading", len(24.0), initial_ctx(100.0));
    match run(&ast) {
        Value::Context(ctx) => assert_eq!(ctx.leading, Length::pt(24.0)),
        other => panic!("expected a context, got {other:?}"),
    }
}

#[test]
fn get_initial_context_text_width_matches_the_given_length() {
    // get-text-width (get-initial-context 345pt ())
    let ast = app1(var("get-text-width"), initial_ctx(345.0));
    assert_len_close(run(&ast), 345.0);
}

#[test]
fn get_initial_context_defaults_paragraph_margins_to_eighteen_points() {
    // Same default as v0.0.6's `get_pdf_mode_initial_context`
    // (primitives.cppo.ml:514-515).
    match run(&initial_ctx(100.0)) {
        Value::Context(ctx) => {
            assert_eq!(ctx.paragraph_top, Length::pt(18.0));
            assert_eq!(ctx.paragraph_bottom, Length::pt(18.0));
        }
        other => panic!("expected a context, got {other:?}"),
    }
}

#[test]
fn set_paragraph_margin_sets_top_and_bottom_independently() {
    // set-paragraph-margin 5pt 9pt (get-initial-context 100pt ())
    let ast = app3(
        "set-paragraph-margin",
        len(5.0),
        len(9.0),
        initial_ctx(100.0),
    );
    match run(&ast) {
        Value::Context(ctx) => {
            assert_eq!(ctx.paragraph_top, Length::pt(5.0));
            assert_eq!(ctx.paragraph_bottom, Length::pt(9.0));
            // Untouched fields survive the update.
            assert_eq!(ctx.paragraph_width, Length::pt(100.0));
        }
        other => panic!("expected a context, got {other:?}"),
    }
}

// ============================================================================
// Box combinators
// ============================================================================

fn as_fixed_widths(v: Value) -> Vec<f64> {
    match v {
        Value::InlineBoxes(boxes) => boxes
            .into_iter()
            .map(|b| match b {
                HorzBox::Pure(PureHorzBox::FixedEmpty { width }) => width.0,
                other => panic!("expected FixedEmpty, got {other:?}"),
            })
            .collect(),
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

#[test]
fn inline_concat_preserves_left_to_right_order() {
    // (inline-skip 5pt) ++ (inline-skip 7pt)
    let ast = app2(
        "++",
        app1(var("inline-skip"), len(5.0)),
        app1(var("inline-skip"), len(7.0)),
    );
    assert_eq!(as_fixed_widths(run(&ast)), vec![5.0, 7.0]);
}

#[test]
fn block_concat_preserves_left_to_right_order() {
    // (block-skip 3pt) +++ (block-skip 4pt)
    let ast = app2(
        "+++",
        app1(var("block-skip"), len(3.0)),
        app1(var("block-skip"), len(4.0)),
    );
    match run(&ast) {
        Value::BlockBoxes(boxes) => {
            assert_eq!(
                boxes,
                vec![VertBox::Skip(Length::pt(3.0)), VertBox::Skip(Length::pt(4.0))]
            );
        }
        other => panic!("expected block-boxes, got {other:?}"),
    }
}

#[test]
fn inline_nil_is_an_empty_inline_boxes_value() {
    assert_eq!(as_fixed_widths(run(&var("inline-nil"))), Vec::<f64>::new());
}

#[test]
fn block_nil_is_an_empty_block_boxes_value() {
    match run(&var("block-nil")) {
        Value::BlockBoxes(boxes) => assert!(boxes.is_empty()),
        other => panic!("expected block-boxes, got {other:?}"),
    }
}

#[test]
fn block_skip_produces_a_vert_box_skip() {
    let ast = app1(var("block-skip"), len(12.0));
    match run(&ast) {
        Value::BlockBoxes(boxes) => assert_eq!(boxes, vec![VertBox::Skip(Length::pt(12.0))]),
        other => panic!("expected block-boxes, got {other:?}"),
    }
}

/// `inline-glue`'s params are `(widnat, widshrink, widstretch)`
/// (vminst.ml:1771 `BackendOuterEmpty`) — assert the resulting
/// `OuterEmpty`'s three fields land in exactly that order, not shuffled.
#[test]
fn inline_glue_field_order_matches_vminst_param_order() {
    let ast = app3("inline-glue", len(10.0), len(2.0), len(6.0));
    match run(&ast) {
        Value::InlineBoxes(boxes) => {
            assert_eq!(boxes.len(), 1);
            match &boxes[0] {
                HorzBox::Pure(PureHorzBox::OuterEmpty {
                    natural,
                    shrinkable,
                    stretchable,
                }) => {
                    assert_eq!(*natural, Length::pt(10.0));
                    assert_eq!(*shrinkable, Length::pt(2.0));
                    assert_eq!(*stretchable, Length::pt(6.0));
                }
                other => panic!("expected OuterEmpty, got {other:?}"),
            }
        }
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

// ============================================================================
// inline-skip participates in line breaking like any other fixed box
// ============================================================================

#[test]
fn inline_skip_width_is_accounted_for_by_line_breaking() {
    // A single wide line (no wrapping): two inline-skips back to back should
    // lay out at x=0 and x=50 respectively, and the line's own natural
    // width should include both.
    let ast = app3(
        "line-break",
        Ast::Bool(false),
        Ast::Bool(false),
        initial_ctx(1000.0),
    );
    // `line-break` is curried as bool -> bool -> context -> inline-boxes ->
    // block-boxes, so apply the inline-boxes argument on top of the app3
    // above.
    let boxes_ast = app2(
        "++",
        app1(var("inline-skip"), len(50.0)),
        app1(var("inline-skip"), len(30.0)),
    );
    let ast = app1(ast, boxes_ast);

    match run(&ast) {
        Value::BlockBoxes(mut lines) => {
            assert_eq!(lines.len(), 1, "should fit on a single line");
            match lines.remove(0) {
                VertBox::Line { contents, .. } => {
                    assert_eq!(contents.len(), 2);
                    let (x0, b0) = &contents[0];
                    let (x1, b1) = &contents[1];
                    assert_eq!(*x0, Length::pt(0.0));
                    assert_eq!(*b0, PureHorzBox::FixedEmpty { width: Length::pt(50.0) });
                    assert_eq!(*x1, Length::pt(50.0));
                    assert_eq!(*b1, PureHorzBox::FixedEmpty { width: Length::pt(30.0) });
                }
                other => panic!("expected a Line, got {other:?}"),
            }
        }
        other => panic!("expected block-boxes, got {other:?}"),
    }
}

// ============================================================================
// Registration coverage: every new name resolves in base_env AND typechecks
// ============================================================================

const NEW_NAMES: &[&str] = &[
    "set-font-size",
    "get-font-size",
    "set-leading",
    "set-paragraph-margin",
    "get-text-width",
    "get-initial-context",
    "++",
    "+++",
    "inline-nil",
    "block-nil",
    "inline-skip",
    "inline-glue",
    "block-skip",
];

#[test]
fn every_new_primitive_resolves_in_base_env() {
    let env = primitives::base_env();
    for name in NEW_NAMES {
        assert!(
            env.lookup(name).is_some(),
            "primitive `{name}` is not bound in base_env()"
        );
    }
}

#[test]
fn every_new_primitive_has_a_registered_type() {
    for name in NEW_NAMES {
        assert!(
            prim_types::primitive_type(name).is_some(),
            "primitive `{name}` has no registered type"
        );
    }
}

// Sanity: Context isn't otherwise reachable as an Ast literal; confirm the
// helper actually produces one (guards the other tests' assumptions).
#[test]
fn initial_ctx_helper_produces_a_context_value() {
    match run(&initial_ctx(42.0)) {
        Value::Context(ctx) => assert_eq!(ctx.paragraph_width, Length::pt(42.0)),
        other => panic!("expected a context, got {other:?}"),
    }
}

