//! Phase-2 evaluator/primitive coverage. These tests build `Ast` values
//! directly (no parser — the surface syntax for `if`/`let rec`/`match` is
//! being built in a parallel worktree) and drive them through
//! `eval::Interp` and `primitives::base_env()`.

use std::rc::Rc;

use rustyfi_backend::{Context, FontKey, FontMetrics, HorzBox, Length, PureHorzBox};
use rustyfi_lang::ast::{Ast, IText, MatchArm, Pattern};
use rustyfi_lang::eval::{self, match_pattern, EvalError};
use rustyfi_lang::value::Value;
use rustyfi_lang::primitives;
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

// ---- if / then / else ----------------------------------------------------------

#[test]
fn if_then_else_both_branches() {
    let ast_true = Ast::IfThenElse(
        Box::new(Ast::Bool(true)),
        Box::new(Ast::Int(1)),
        Box::new(Ast::Int(2)),
    );
    assert!(matches!(run(&ast_true).unwrap(), Value::Int(1)));

    let ast_false = Ast::IfThenElse(
        Box::new(Ast::Bool(false)),
        Box::new(Ast::Int(1)),
        Box::new(Ast::Int(2)),
    );
    assert!(matches!(run(&ast_false).unwrap(), Value::Int(2)));
}

#[test]
fn if_condition_type_error() {
    let ast = Ast::IfThenElse(
        Box::new(Ast::Int(1)),
        Box::new(Ast::Int(1)),
        Box::new(Ast::Int(2)),
    );
    let err = run(&ast).unwrap_err();
    assert!(err.to_string().contains("bool"));
}

// ---- let rec --------------------------------------------------------------------

#[test]
fn let_rec_factorial() {
    // let rec fact n = if n == 0 then 1 else n * fact (n - 1) in fact 5
    let body = Ast::IfThenElse(
        Box::new(app2("==", var("n"), Ast::Int(0))),
        Box::new(Ast::Int(1)),
        Box::new(app2(
            "*",
            var("n"),
            app1(var("fact"), app2("-", var("n"), Ast::Int(1))),
        )),
    );
    let fact_lambda = Rc::new(Ast::Lambda("n".to_string(), Rc::new(body)));
    let ast = Ast::LetRecIn(
        vec![("fact".to_string(), fact_lambda)],
        Box::new(app1(var("fact"), Ast::Int(5))),
    );
    assert!(matches!(run(&ast).unwrap(), Value::Int(120)));
}

#[test]
fn let_rec_mutual_even_odd() {
    // let rec even n = if n == 0 then true else odd (n - 1)
    //     and odd n = if n == 0 then false else even (n - 1)
    // in (even 10, odd 7)
    let even_body = Ast::IfThenElse(
        Box::new(app2("==", var("n"), Ast::Int(0))),
        Box::new(Ast::Bool(true)),
        Box::new(app1(var("odd"), app2("-", var("n"), Ast::Int(1)))),
    );
    let odd_body = Ast::IfThenElse(
        Box::new(app2("==", var("n"), Ast::Int(0))),
        Box::new(Ast::Bool(false)),
        Box::new(app1(var("even"), app2("-", var("n"), Ast::Int(1)))),
    );
    let bindings = vec![
        (
            "even".to_string(),
            Rc::new(Ast::Lambda("n".to_string(), Rc::new(even_body))),
        ),
        (
            "odd".to_string(),
            Rc::new(Ast::Lambda("n".to_string(), Rc::new(odd_body))),
        ),
    ];
    let body = Ast::Tuple(vec![
        app1(var("even"), Ast::Int(10)),
        app1(var("odd"), Ast::Int(7)),
    ]);
    let ast = Ast::LetRecIn(bindings, Box::new(body));
    let Value::Tuple(items) = run(&ast).unwrap() else {
        panic!("expected tuple")
    };
    assert!(matches!(items[0], Value::Bool(true)));
    assert!(matches!(items[1], Value::Bool(true)));
}

#[test]
fn let_rec_non_lambda_binding_is_an_error() {
    let ast = Ast::LetRecIn(
        vec![("x".to_string(), Rc::new(Ast::Int(1)))],
        Box::new(var("x")),
    );
    let err = run(&ast).unwrap_err();
    assert!(err.to_string().contains("x"));
}

// ---- match ------------------------------------------------------------------------

#[test]
fn match_int_literals_with_wildcard() {
    let ast = Ast::Match(
        Box::new(Ast::Int(3)),
        vec![
            MatchArm {
                pat: Pattern::Int(1),
                guard: None,
                body: Ast::Str("one".to_string()),
            },
            MatchArm {
                pat: Pattern::Int(2),
                guard: None,
                body: Ast::Str("two".to_string()),
            },
            MatchArm {
                pat: Pattern::Wild,
                guard: None,
                body: Ast::Str("other".to_string()),
            },
        ],
    );
    let Value::Str(s) = run(&ast).unwrap() else {
        panic!("expected string")
    };
    assert_eq!(s, "other");
}

#[test]
fn match_cons_and_empty_list_through_eval() {
    let non_empty = Ast::Match(
        Box::new(Ast::List(vec![Ast::Int(1), Ast::Int(2), Ast::Int(3)])),
        vec![
            MatchArm {
                pat: Pattern::EmptyList,
                guard: None,
                body: Ast::Int(-1),
            },
            MatchArm {
                pat: Pattern::Cons(
                    Box::new(Pattern::Var("h".to_string())),
                    Box::new(Pattern::Var("t".to_string())),
                ),
                guard: None,
                body: var("h"),
            },
        ],
    );
    assert!(matches!(run(&non_empty).unwrap(), Value::Int(1)));

    let empty = Ast::Match(
        Box::new(Ast::List(vec![])),
        vec![
            MatchArm {
                pat: Pattern::EmptyList,
                guard: None,
                body: Ast::Int(42),
            },
            MatchArm {
                pat: Pattern::Wild,
                guard: None,
                body: Ast::Int(-1),
            },
        ],
    );
    assert!(matches!(run(&empty).unwrap(), Value::Int(42)));
}

#[test]
fn match_pattern_cons_tail_is_rest_of_list() {
    // Unit-tests the helper directly so we can inspect the constructed tail,
    // not just the arm body's result.
    let value = Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
    let pat = Pattern::Cons(
        Box::new(Pattern::Var("h".to_string())),
        Box::new(Pattern::Var("t".to_string())),
    );
    let mut bindings = Vec::new();
    assert!(match_pattern(&pat, &value, &mut bindings));
    let h = &bindings.iter().find(|(n, _)| n == "h").unwrap().1;
    let t = &bindings.iter().find(|(n, _)| n == "t").unwrap().1;
    assert!(matches!(h, Value::Int(1)));
    let Value::List(tail) = t else {
        panic!("expected list")
    };
    assert_eq!(tail.len(), 2);
    assert!(matches!(tail[0], Value::Int(2)));
    assert!(matches!(tail[1], Value::Int(3)));
}

#[test]
fn match_ctor_some_none_with_payload() {
    let some_ast = Ast::Match(
        Box::new(Ast::Ctor("Some".to_string(), Some(Box::new(Ast::Int(5))))),
        vec![
            MatchArm {
                pat: Pattern::Ctor("None".to_string(), None),
                guard: None,
                body: Ast::Int(0),
            },
            MatchArm {
                pat: Pattern::Ctor(
                    "Some".to_string(),
                    Some(Box::new(Pattern::Var("x".to_string()))),
                ),
                guard: None,
                body: var("x"),
            },
        ],
    );
    assert!(matches!(run(&some_ast).unwrap(), Value::Int(5)));

    let none_ast = Ast::Match(
        Box::new(Ast::Ctor("None".to_string(), None)),
        vec![
            MatchArm {
                pat: Pattern::Ctor("None".to_string(), None),
                guard: None,
                body: Ast::Int(-1),
            },
            MatchArm {
                pat: Pattern::Ctor("Some".to_string(), Some(Box::new(Pattern::Wild))),
                guard: None,
                body: Ast::Int(99),
            },
        ],
    );
    assert!(matches!(run(&none_ast).unwrap(), Value::Int(-1)));
}

#[test]
fn match_guard_selects_arm() {
    // match 4 with
    // | x when x > 10 -> "big"
    // | x when x > 0  -> "small"
    // | _              -> "non-positive"
    let ast = Ast::Match(
        Box::new(Ast::Int(4)),
        vec![
            MatchArm {
                pat: Pattern::Var("x".to_string()),
                guard: Some(app2(">", var("x"), Ast::Int(10))),
                body: Ast::Str("big".to_string()),
            },
            MatchArm {
                pat: Pattern::Var("x".to_string()),
                guard: Some(app2(">", var("x"), Ast::Int(0))),
                body: Ast::Str("small".to_string()),
            },
            MatchArm {
                pat: Pattern::Wild,
                guard: None,
                body: Ast::Str("non-positive".to_string()),
            },
        ],
    );
    let Value::Str(s) = run(&ast).unwrap() else {
        panic!("expected string")
    };
    assert_eq!(s, "small");
}

#[test]
fn match_as_binding() {
    let scrutinee = Ast::Tuple(vec![Ast::Int(1), Ast::Int(2)]);
    let ast = Ast::Match(
        Box::new(scrutinee),
        vec![MatchArm {
            pat: Pattern::As(
                Box::new(Pattern::Tuple(vec![
                    Pattern::Var("a".to_string()),
                    Pattern::Var("b".to_string()),
                ])),
                "pair".to_string(),
            ),
            guard: None,
            body: var("pair"),
        }],
    );
    let Value::Tuple(items) = run(&ast).unwrap() else {
        panic!("expected tuple")
    };
    assert_eq!(items.len(), 2);
    assert!(matches!(items[0], Value::Int(1)));
    assert!(matches!(items[1], Value::Int(2)));
}

#[test]
fn non_exhaustive_match_errors() {
    let ast = Ast::Match(
        Box::new(Ast::Int(5)),
        vec![MatchArm {
            pat: Pattern::Int(1),
            guard: None,
            body: Ast::Int(0),
        }],
    );
    let err = run(&ast).unwrap_err();
    assert!(err.to_string().contains("non-exhaustive match"));
    assert!(err.to_string().contains("int"));
}

// ---- tuples -------------------------------------------------------------------------

#[test]
fn tuple_construction_and_pattern_match() {
    let ast = Ast::Tuple(vec![Ast::Int(1), Ast::Str("x".to_string()), Ast::Bool(true)]);
    let Value::Tuple(items) = run(&ast).unwrap() else {
        panic!("expected tuple")
    };
    assert_eq!(items.len(), 3);

    let match_ast = Ast::Match(
        Box::new(ast),
        vec![MatchArm {
            pat: Pattern::Tuple(vec![
                Pattern::Var("a".to_string()),
                Pattern::Wild,
                Pattern::Bool(true),
            ]),
            guard: None,
            body: var("a"),
        }],
    );
    assert!(matches!(run(&match_ast).unwrap(), Value::Int(1)));
}

// ---- arithmetic / comparison / logical primitives ------------------------------------

#[test]
fn int_arithmetic_and_comparison_prims() {
    assert!(matches!(
        run(&app2("+", Ast::Int(2), Ast::Int(3))).unwrap(),
        Value::Int(5)
    ));
    assert!(matches!(
        run(&app2("-", Ast::Int(2), Ast::Int(3))).unwrap(),
        Value::Int(-1)
    ));
    assert!(matches!(
        run(&app2("*", Ast::Int(2), Ast::Int(3))).unwrap(),
        Value::Int(6)
    ));
    assert!(matches!(
        run(&app2("/", Ast::Int(7), Ast::Int(2))).unwrap(),
        Value::Int(3)
    ));
    assert!(matches!(
        run(&app2("mod", Ast::Int(7), Ast::Int(2))).unwrap(),
        Value::Int(1)
    ));
    assert!(matches!(
        run(&app2("==", Ast::Int(2), Ast::Int(2))).unwrap(),
        Value::Bool(true)
    ));
    assert!(matches!(
        run(&app2("<>", Ast::Int(2), Ast::Int(3))).unwrap(),
        Value::Bool(true)
    ));
    assert!(matches!(
        run(&app2("<", Ast::Int(2), Ast::Int(3))).unwrap(),
        Value::Bool(true)
    ));
    assert!(matches!(
        run(&app2(">", Ast::Int(2), Ast::Int(3))).unwrap(),
        Value::Bool(false)
    ));
    assert!(matches!(
        run(&app2("<=", Ast::Int(3), Ast::Int(3))).unwrap(),
        Value::Bool(true)
    ));
    assert!(matches!(
        run(&app2(">=", Ast::Int(2), Ast::Int(3))).unwrap(),
        Value::Bool(false)
    ));
}

#[test]
fn division_and_mod_by_zero_are_errors() {
    let err = run(&app2("/", Ast::Int(1), Ast::Int(0))).unwrap_err();
    assert!(err.to_string().contains("division by zero"));
    let err = run(&app2("mod", Ast::Int(1), Ast::Int(0))).unwrap_err();
    assert!(err.to_string().contains("division by zero"));
}

#[test]
fn bool_logic_prims() {
    assert!(matches!(
        run(&app2("&&", Ast::Bool(true), Ast::Bool(false))).unwrap(),
        Value::Bool(false)
    ));
    assert!(matches!(
        run(&app2("&&", Ast::Bool(true), Ast::Bool(true))).unwrap(),
        Value::Bool(true)
    ));
    assert!(matches!(
        run(&app2("||", Ast::Bool(true), Ast::Bool(false))).unwrap(),
        Value::Bool(true)
    ));
    assert!(matches!(
        run(&app2("||", Ast::Bool(false), Ast::Bool(false))).unwrap(),
        Value::Bool(false)
    ));
    assert!(matches!(
        run(&app1(var("not"), Ast::Bool(true))).unwrap(),
        Value::Bool(false)
    ));
}

#[test]
fn float_prims() {
    let Value::Float(f) = run(&app2("+.", Ast::Float(1.5), Ast::Float(2.5))).unwrap() else {
        panic!("expected float")
    };
    assert!((f - 4.0).abs() < 1e-9);

    let Value::Float(f) = run(&app2("-.", Ast::Float(1.5), Ast::Float(2.5))).unwrap() else {
        panic!("expected float")
    };
    assert!((f - -1.0).abs() < 1e-9);

    let Value::Float(f) = run(&app2("*.", Ast::Float(1.5), Ast::Float(2.0))).unwrap() else {
        panic!("expected float")
    };
    assert!((f - 3.0).abs() < 1e-9);

    let Value::Float(f) = run(&app2("/.", Ast::Float(3.0), Ast::Float(2.0))).unwrap() else {
        panic!("expected float")
    };
    assert!((f - 1.5).abs() < 1e-9);

    let Value::Float(f) = run(&app1(var("float"), Ast::Int(3))).unwrap() else {
        panic!("expected float")
    };
    assert!((f - 3.0).abs() < 1e-9);

    // `round` in v0.0.6 is actually truncation toward zero (int_of_float).
    assert!(matches!(
        run(&app1(var("round"), Ast::Float(3.9))).unwrap(),
        Value::Int(3)
    ));
    assert!(matches!(
        run(&app1(var("round"), Ast::Float(-3.9))).unwrap(),
        Value::Int(-3)
    ));
}

#[test]
fn length_prims() {
    let Value::Length(l) = run(&app2(
        "+'",
        Ast::Length(Length::pt(1.0)),
        Ast::Length(Length::pt(2.0)),
    ))
    .unwrap() else {
        panic!("expected length")
    };
    assert_eq!(l, Length::pt(3.0));

    let Value::Length(l) = run(&app2(
        "-'",
        Ast::Length(Length::pt(5.0)),
        Ast::Length(Length::pt(2.0)),
    ))
    .unwrap() else {
        panic!("expected length")
    };
    assert_eq!(l, Length::pt(3.0));

    let Value::Length(l) = run(&app2(
        "*'",
        Ast::Length(Length::pt(2.0)),
        Ast::Float(3.0),
    ))
    .unwrap() else {
        panic!("expected length")
    };
    assert_eq!(l, Length::pt(6.0));

    let Value::Float(f) = run(&app2(
        "/'",
        Ast::Length(Length::pt(6.0)),
        Ast::Length(Length::pt(2.0)),
    ))
    .unwrap() else {
        panic!("expected float")
    };
    assert!((f - 3.0).abs() < 1e-9);

    assert!(matches!(
        run(&app2(
            "<'",
            Ast::Length(Length::pt(1.0)),
            Ast::Length(Length::pt(2.0))
        ))
        .unwrap(),
        Value::Bool(true)
    ));
    assert!(matches!(
        run(&app2(
            ">'",
            Ast::Length(Length::pt(2.0)),
            Ast::Length(Length::pt(1.0))
        ))
        .unwrap(),
        Value::Bool(true)
    ));
    assert!(matches!(
        run(&app2(
            ">'",
            Ast::Length(Length::pt(1.0)),
            Ast::Length(Length::pt(2.0))
        ))
        .unwrap(),
        Value::Bool(false)
    ));
}

#[test]
fn string_concat_and_arabic_and_same() {
    let Value::Str(s) = run(&app2(
        "^",
        Ast::Str("foo".to_string()),
        Ast::Str("bar".to_string()),
    ))
    .unwrap() else {
        panic!("expected string")
    };
    assert_eq!(s, "foobar");

    let Value::Str(s) = run(&app1(var("arabic"), Ast::Int(42))).unwrap() else {
        panic!("expected string")
    };
    assert_eq!(s, "42");

    assert!(matches!(
        run(&app2(
            "string-same",
            Ast::Str("a".to_string()),
            Ast::Str("a".to_string())
        ))
        .unwrap(),
        Value::Bool(true)
    ));
    assert!(matches!(
        run(&app2(
            "string-same",
            Ast::Str("a".to_string()),
            Ast::Str("b".to_string())
        ))
        .unwrap(),
        Value::Bool(false)
    ));
}

// ---- IText::Embed splicing ----------------------------------------------------------

#[test]
fn itext_embed_splices_inline_text() {
    let inner_elems = Rc::new(vec![IText::Text("world".to_string())]);
    let base_env = primitives::base_env();
    let embedded_value = Value::InlineText {
        elems: inner_elems,
        env: base_env.clone(),
    };
    let outer_env = base_env.child();
    outer_env.define("greeting", embedded_value);

    let elems = vec![
        IText::Text("hello ".to_string()),
        IText::Embed {
            expr: var("greeting"),
            span: Span::default(),
        },
    ];

    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let ctx = Context::initial(Length::pt(400.0));
    let boxes = primitives::read_inline(&mut interp, &ctx, &elems, &outer_env).unwrap();

    let words: Vec<&str> = boxes
        .iter()
        .filter_map(|b| match b {
            HorzBox::Pure(PureHorzBox::InnerString { text, .. }) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(words, vec!["hello", "world"]);

    // A non inline-text embed is a type error, not a panic.
    let bad_elems = vec![IText::Embed {
        expr: Ast::Int(1),
        span: Span::default(),
    }];
    let err = primitives::read_inline(&mut interp, &ctx, &bad_elems, &outer_env).unwrap_err();
    assert!(err.to_string().contains("inline-text"));
}
