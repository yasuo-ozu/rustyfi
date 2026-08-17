//! Phase-2b evaluator/primitive coverage: `let-mutable`/`<-`/`while`,
//! `before` (Sequential), `#label`/`(| with |)` field access/update, quoted
//! math text, and the primitives that came along for the ride ("!",
//! `line-break`'s real 4-ary signature). Like `eval_phase2.rs`, these build
//! `Ast` values directly — the surface syntax for these constructs is being
//! built in a parallel worktree.

use std::rc::Rc;

use rustyfi_backend::{Context, FontKey, FontMetrics, HorzBox, Length, PureHorzBox};
use rustyfi_lang::ast::{Ast, MathElem as AstMathElem};
use rustyfi_lang::quoted::{IText, MathElem};
use rustyfi_lang::eval::{self, EvalError};
use rustyfi_lang::primitives;
use rustyfi_lang::value::{Env, Value};
use rustyfi_syntax::Span;

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

// ---- small Ast-builder helpers -------------------------------------------------

fn var(name: &str) -> Ast {
    Ast::Var(name.to_string(), Span::default())
}

fn app1(f: Ast, a: Ast) -> Ast {
    Ast::Apply(Box::new(f), Box::new(a))
}

/// `name a b` — a curried two-argument application of a (primitive) name.
fn app2(name: &str, a: Ast, b: Ast) -> Ast {
    app1(app1(var(name), a), b)
}

fn run(ast: &Ast) -> Result<Value, EvalError> {
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    interp.eval(&env, ast)
}

// ---- let-mutable / overwrite / while -------------------------------------------

#[test]
fn counter_loop_reaches_three() {
    // let-mutable x <- 0 in
    //   while !x < 3 do x <- !x + 1;
    //   !x
    let ast = Ast::LetMutableIn(
        "x".to_string(),
        Box::new(Ast::Int(0)),
        Box::new(Ast::Sequential(
            Box::new(Ast::WhileDo(
                Box::new(app2("<", app1(var("!"), var("x")), Ast::Int(3))),
                Box::new(Ast::Overwrite(
                    "x".to_string(),
                    Span::default(),
                    Box::new(app2("+", app1(var("!"), var("x")), Ast::Int(1))),
                )),
            )),
            Box::new(app1(var("!"), var("x"))),
        )),
    );
    assert!(matches!(run(&ast).unwrap(), Value::Int(3)));
}

#[test]
fn overwrite_of_immutable_variable_errors() {
    // let x = 0 in x <- 1
    let ast = Ast::LetIn(
        "x".to_string(),
        Box::new(Ast::Int(0)),
        Box::new(Ast::Overwrite(
            "x".to_string(),
            Span::default(),
            Box::new(Ast::Int(1)),
        )),
    );
    let err = run(&ast).unwrap_err();
    assert!(err.to_string().contains("immutable"));
}

#[test]
fn overwrite_of_unbound_variable_errors() {
    let ast = Ast::Overwrite("nope".to_string(), Span::default(), Box::new(Ast::Int(1)));
    let err = run(&ast).unwrap_err();
    assert!(err.to_string().contains("nope"));
    assert!(err.to_string().contains("unbound"));
}

// ---- sequential (`before`) ------------------------------------------------------

#[test]
fn sequential_discards_first_value() {
    let ast = Ast::Sequential(Box::new(Ast::Int(1)), Box::new(Ast::Int(2)));
    assert!(matches!(run(&ast).unwrap(), Value::Int(2)));
}

// ---- access-field / update-field ------------------------------------------------

fn sample_record() -> Ast {
    Ast::Record(vec![
        ("a".to_string(), Ast::Int(1)),
        ("b".to_string(), Ast::Int(2)),
    ])
}

#[test]
fn access_field_hit() {
    let ast = Ast::AccessField(Box::new(sample_record()), "a".to_string(), Span::default());
    assert!(matches!(run(&ast).unwrap(), Value::Int(1)));
}

#[test]
fn access_field_missing_label_error_names_label_and_available_keys() {
    let ast = Ast::AccessField(Box::new(sample_record()), "z".to_string(), Span::default());
    let err = run(&ast).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains('z'), "message should name the missing label: {msg}");
    assert!(msg.contains('a') && msg.contains('b'), "message should list available fields: {msg}");
}

#[test]
fn update_field_replaces_existing_label() {
    let ast = Ast::UpdateField(
        Box::new(sample_record()),
        "a".to_string(),
        Box::new(Ast::Int(99)),
    );
    let Value::Record(map) = run(&ast).unwrap() else {
        panic!("expected record")
    };
    assert!(matches!(map.get("a"), Some(Value::Int(99))));
    assert!(matches!(map.get("b"), Some(Value::Int(2))));
}

#[test]
fn update_field_absent_label_errors() {
    // v0.0.6 (evaluator.cppo.ml `UpdateField`) requires the field to already
    // exist: `Assoc.find_opt asc1 fldnm` is matched `None -> report_bug_reduction
    // "UpdateField: field '...' not found" | Some(_) -> Assoc.add ...` — i.e.
    // updating an absent label is a bug, not a way to add a field.
    let ast = Ast::UpdateField(
        Box::new(sample_record()),
        "z".to_string(),
        Box::new(Ast::Int(1)),
    );
    let err = run(&ast).unwrap_err();
    assert!(err.to_string().contains('z'));
}

// ---- quoted math ------------------------------------------------------------------

#[test]
fn math_text_quotes_without_evaluating() {
    let ast = Ast::MathText(Rc::new(vec![AstMathElem::Chars("x".to_string())]));
    match run(&ast).unwrap() {
        // The element tree is COMPILED into the value (see `quoted`), so this
        // is no longer the same `Rc` the AST held — but nothing in it was
        // evaluated, which is what "quotes" means here.
        Value::MathText { elems: got, .. } => {
            assert!(matches!(got.as_slice(), [MathElem::Chars(s)] if s == "x"));
        }
        other => panic!("expected MathText, got {}", other.type_name()),
    }
}

/// `docs/plans/math-engine.md` §Slice 1: `read_inline`'s `EmbedMath` arm no
/// longer errors — it walks the `MathElem` tree into a `PureHorzBox::Math`
/// (see `math_slice1.rs` for the box's own glyph-shift/-scale assertions;
/// this just checks the seam from `IText::EmbedMath` through `read_inline`).
#[test]
fn itext_embed_math_renders_through_read_inline() {
    let elems = vec![IText::EmbedMath {
        elems: Rc::new(vec![MathElem::Chars("x".to_string())]),
        span: Span::default(),
    }];
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let ctx = Context::initial(Length::pt(400.0));
    let boxes = primitives::read_inline(&mut interp, &ctx, &elems, &Env::root())
        .expect("EmbedMath must render, not error, as of Slice 1");
    assert_eq!(boxes.len(), 1);
    match &boxes[0] {
        HorzBox::Pure(PureHorzBox::Math { glyphs, .. }) => {
            assert_eq!(glyphs.len(), 1);
            assert_eq!(glyphs[0].text, "x");
        }
        other => panic!("expected a single Math box, got {other:?}"),
    }
}

// ---- "!" dereference primitive -----------------------------------------------------

#[test]
fn deref_of_non_ref_errors() {
    let ast = app1(var("!"), Ast::Int(5));
    let err = run(&ast).unwrap_err();
    assert!(err.to_string().contains("mutable"));
}

// ---- line-break's real 4-ary signature --------------------------------------------

#[test]
fn line_break_arity_four_through_apply_chain() {
    let base = primitives::base_env();
    let mut env = base.child();
    let ctx = Context::initial(Length::pt(400.0));
    env.define("ctx0", Value::Context(Box::new(ctx)));
    env.define(
        "ib0",
        Value::InlineBoxes(vec![HorzBox::Pure(PureHorzBox::OuterFil)]),
    );

    let ast = app1(
        app1(
            app1(app1(var("line-break"), Ast::Bool(true)), Ast::Bool(false)),
            var("ctx0"),
        ),
        var("ib0"),
    );

    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let result = interp.eval(&env, &ast).unwrap();
    assert!(matches!(result, Value::BlockBoxes(_)));
}
