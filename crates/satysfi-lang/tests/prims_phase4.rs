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
            // line-break now brackets the paragraph with paragraph_top/bottom
            // margin Skips (design-silent-fields FIX 3); find the formed Line.
            let line = lines
                .into_iter()
                .find(|vb| matches!(vb, VertBox::Line { .. }))
                .expect("should fit on a single line");
            match line {
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

// ============================================================================
// frontend-completion.md §Slice 1.A: the ~18 pure primitives (`|>` excluded
// — it has no `Ast`-level identity at all to apply here; see
// `elaborate_phase2.rs`'s operator-precedence section for its coverage).
// ============================================================================

fn run_err(ast: &Ast) -> eval::EvalError {
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    interp
        .eval(&env, ast)
        .expect_err("evaluation should have failed")
}

fn assert_float_close(v: Value, expect: f64) {
    match v {
        Value::Float(x) => assert!(
            (x - expect).abs() < 1e-9,
            "expected {expect}, got {x}"
        ),
        other => panic!("expected a float, got {other:?}"),
    }
}

fn str_val(v: Value) -> String {
    match v {
        Value::Str(s) => s,
        other => panic!("expected a string, got {other:?}"),
    }
}

#[test]
fn float_trig_functions_match_std_libm() {
    assert_float_close(run(&app1(var("sin"), Ast::Float(0.0))), 0.0);
    assert_float_close(run(&app1(var("cos"), Ast::Float(0.0))), 1.0);
    assert_float_close(run(&app1(var("tan"), Ast::Float(0.0))), 0.0);
    assert_float_close(
        run(&app1(var("asin"), Ast::Float(1.0))),
        std::f64::consts::FRAC_PI_2,
    );
    assert_float_close(run(&app1(var("acos"), Ast::Float(1.0))), 0.0);
    assert_float_close(
        run(&app1(var("atan"), Ast::Float(1.0))),
        std::f64::consts::FRAC_PI_4,
    );
}

/// `atan2`'s vminst.ml param order is `flt1` then `flt2`, body `atan2 flt1
/// flt2` — i.e. `flt1.atan2(flt2)`, NOT the arguments swapped.
#[test]
fn atan2_argument_order_matches_vminst_param_order() {
    assert_float_close(
        run(&app2("atan2", Ast::Float(1.0), Ast::Float(0.0))),
        std::f64::consts::FRAC_PI_2,
    );
    assert_float_close(
        run(&app2("atan2", Ast::Float(1.0), Ast::Float(1.0))),
        std::f64::consts::FRAC_PI_4,
    );
}

#[test]
fn log_is_natural_logarithm_not_log10() {
    assert_float_close(run(&app1(var("log"), Ast::Float(std::f64::consts::E))), 1.0);
}

#[test]
fn exp_of_zero_is_one() {
    assert_float_close(run(&app1(var("exp"), Ast::Float(0.0))), 1.0);
}

/// `ceil`/`floor` return `float`, unlike `round` (which returns `int`) —
/// pin down both the numeric result AND the runtime `Value` variant.
#[test]
fn ceil_and_floor_return_floats_not_ints() {
    match run(&app1(var("ceil"), Ast::Float(1.2))) {
        Value::Float(x) => assert_eq!(x, 2.0),
        other => panic!("expected a float (not int) from ceil, got {other:?}"),
    }
    match run(&app1(var("floor"), Ast::Float(1.8))) {
        Value::Float(x) => assert_eq!(x, 1.0),
        other => panic!("expected a float (not int) from floor, got {other:?}"),
    }
}

#[test]
fn show_float_matches_ocaml_string_of_float_on_ordinary_values() {
    assert_eq!(str_val(run(&app1(var("show-float"), Ast::Float(1.0)))), "1.");
    assert_eq!(
        str_val(run(&app1(var("show-float"), Ast::Float(100.0)))),
        "100."
    );
    assert_eq!(str_val(run(&app1(var("show-float"), Ast::Float(3.5)))), "3.5");
    assert_eq!(
        str_val(run(&app1(var("show-float"), Ast::Float(-2.0)))),
        "-2."
    );
    assert_eq!(str_val(run(&app1(var("show-float"), Ast::Float(0.0)))), "0.");
}

#[test]
fn string_byte_length_counts_utf8_bytes_not_scalar_values() {
    // "café" is 4 Unicode scalar values but 5 UTF-8 bytes ('é' is 2 bytes).
    let byte_len = app1(var("string-byte-length"), Ast::Str("café".to_string()));
    match run(&byte_len) {
        Value::Int(n) => assert_eq!(n, 5),
        other => panic!("expected an int, got {other:?}"),
    }
    // Cross-check against `string-length` (scalar-value count) on the same
    // input, pinning down the exact divergence this primitive exists for.
    let scalar_len = app1(var("string-length"), Ast::Str("café".to_string()));
    match run(&scalar_len) {
        Value::Int(n) => assert_eq!(n, 4),
        other => panic!("expected an int, got {other:?}"),
    }
}

#[test]
fn string_sub_bytes_is_byte_indexed() {
    // "café" = 'c' 'a' 'f' (1 byte each) then 'é' (bytes 3-4, 2 bytes).
    let ast = app3(
        "string-sub-bytes",
        Ast::Str("café".to_string()),
        Ast::Int(3),
        Ast::Int(2),
    );
    assert_eq!(str_val(run(&ast)), "é");
}

#[test]
fn string_sub_bytes_rejects_a_non_char_boundary_split() {
    // Splitting inside "é"'s 2-byte encoding must error, not panic.
    let ast = app3(
        "string-sub-bytes",
        Ast::Str("café".to_string()),
        Ast::Int(3),
        Ast::Int(1),
    );
    assert!(run_err(&ast).to_string().contains("illegal index"));
}

#[test]
fn string_sub_bytes_rejects_a_negative_index() {
    let ast = app3(
        "string-sub-bytes",
        Ast::Str("abc".to_string()),
        Ast::Int(-1),
        Ast::Int(1),
    );
    assert!(run_err(&ast).to_string().contains("illegal index"));
}

#[test]
fn string_unexplode_is_the_inverse_of_string_explode() {
    let exploded = app1(var("string-explode"), Ast::Str("café".to_string()));
    let roundtrip = app1(var("string-unexplode"), exploded);
    assert_eq!(str_val(run(&roundtrip)), "café");
}

#[test]
fn string_unexplode_rejects_an_invalid_code_point() {
    // 0xD800 is a UTF-16 surrogate half — not a valid Unicode scalar value.
    let ast = app1(var("string-unexplode"), Ast::List(vec![Ast::Int(0xD800)]));
    assert!(run_err(&ast).to_string().contains("Unicode scalar value"));
}

#[test]
fn display_message_prints_and_returns_unit() {
    let ast = app1(var("display-message"), Ast::Str("hello from a test".to_string()));
    assert!(matches!(run(&ast), Value::Unit));
}

#[test]
fn abort_with_message_errors_with_the_given_message() {
    let ast = app1(var("abort-with-message"), Ast::Str("boom".to_string()));
    assert!(run_err(&ast).to_string().contains("boom"));
}

/// `abort-with-message`'s result type is polymorphic (`'a`) — it must
/// resolve in `base_env`/typecheck no matter what the caller expects the
/// (never-produced) result to be; exercised end-to-end via `+` here.
#[test]
fn abort_with_message_unifies_with_any_expected_type() {
    let ast = app2(
        "+",
        Ast::Int(1),
        app1(var("abort-with-message"), Ast::Str("boom".to_string())),
    );
    assert!(run_err(&ast).to_string().contains("boom"));
}

const NEW_NAMES_SLICE1: &[&str] = &[
    "sin",
    "asin",
    "cos",
    "acos",
    "tan",
    "atan",
    "atan2",
    "log",
    "exp",
    "ceil",
    "floor",
    "show-float",
    "string-byte-length",
    "string-sub-bytes",
    "string-unexplode",
    "display-message",
    "abort-with-message",
];

#[test]
fn every_slice1_primitive_resolves_in_base_env() {
    let env = primitives::base_env();
    for name in NEW_NAMES_SLICE1 {
        assert!(
            env.lookup(name).is_some(),
            "primitive `{name}` is not bound in base_env()"
        );
    }
}

#[test]
fn every_slice1_primitive_has_a_registered_type() {
    for name in NEW_NAMES_SLICE1 {
        assert!(
            prim_types::primitive_type(name).is_some(),
            "primitive `{name}` has no registered type"
        );
    }
}

