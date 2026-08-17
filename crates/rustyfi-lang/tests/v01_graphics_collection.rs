//! L5b (`…/tmp/prim-retype-sweep.md` §4.2): the graphics-collection sweep
//! — `unite-graphics`/`clip-graphics-by-path` (A12/A13), the `get-graphics-
//! bbox` `Option` fork (R3), and the shared `coerce_graphics_result` per-
//! version callback-result coercion (H1-H6/R2). Harness: direct
//! `Interp::apply` chains against `base_env`/`base_env_with_version` for
//! the pure prims (the `v01_prims_scalar.rs` pattern), plus small hand-built
//! `Ast::Lambda` closures (the `frame_deco_firing.rs`/
//! `inline_graphics_outer.rs` pattern) for the callback-coercion tests,
//! which need a real closure value to apply.

use rustyfi_backend::{Color, FontKey, FontMetrics, GraphicsElem, Length};
use rustyfi_lang::ast::Ast;
use rustyfi_lang::eval::Interp;
use rustyfi_lang::value::{BaseEnv, Value};
use rustyfi_lang::{prim_types, primitives};
use rustyfi_syntax::{RustyfiVersion, Span};
use std::rc::Rc;

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

/// Apply a named primitive (looked up in `env`) to `args`, left to right —
/// the `v01_prims_scalar.rs` helper.
fn call(interp: &mut Interp, env: &BaseEnv, name: &str, args: Vec<Value>) -> Value {
    let mut f = env
        .lookup(name)
        .unwrap_or_else(|| panic!("{name} is not bound"));
    for a in args {
        f = interp
            .apply(f, a)
            .unwrap_or_else(|e| panic!("{name} application failed: {e}"));
    }
    f
}

fn try_call(
    interp: &mut Interp,
    env: &BaseEnv,
    name: &str,
    args: Vec<Value>,
) -> Result<Value, rustyfi_lang::eval::EvalError> {
    let mut f = env
        .lookup(name)
        .unwrap_or_else(|| panic!("{name} is not bound"));
    for a in args {
        f = interp.apply(f, a)?;
    }
    Ok(f)
}

fn point_val(x: f64, y: f64) -> Value {
    Value::Tuple(vec![
        Value::Length(Length::pt(x)),
        Value::Length(Length::pt(y)),
    ])
}

fn gray_val(g: f64) -> Value {
    Value::Ctor("Gray".to_string(), Some(Box::new(Value::Float(g))))
}

/// Build a `w` x `h` rectangle path anchored at `(ox, oy)`, via the actual
/// `start-path`/`line-to`/`close-with-line` primitives (not a hand-built
/// `Path` struct) — exercises the same code every `.saty` source would.
fn rect_path(interp: &mut Interp, env: &BaseEnv, ox: f64, oy: f64, w: f64, h: f64) -> Value {
    let p0 = call(interp, env, "start-path", vec![point_val(ox, oy)]);
    let p1 = call(interp, env, "line-to", vec![point_val(ox + w, oy), p0]);
    let p2 = call(interp, env, "line-to", vec![point_val(ox + w, oy + h), p1]);
    let p3 = call(interp, env, "line-to", vec![point_val(ox, oy + h), p2]);
    call(interp, env, "close-with-line", vec![p3])
}

fn fill_val(interp: &mut Interp, env: &BaseEnv, path: Value) -> Value {
    call(interp, env, "fill", vec![gray_val(0.0), path])
}

fn as_point(v: Value) -> (f64, f64) {
    match v {
        Value::Tuple(vs) if vs.len() == 2 => {
            let x = match &vs[0] {
                Value::Length(l) => l.0,
                other => panic!("expected a length, got {other:?}"),
            };
            let y = match &vs[1] {
                Value::Length(l) => l.0,
                other => panic!("expected a length, got {other:?}"),
            };
            (x, y)
        }
        other => panic!("expected a point, got {other:?}"),
    }
}

fn as_bbox_option(v: Value) -> Option<((f64, f64), (f64, f64))> {
    match v {
        Value::Ctor(name, payload) => match (name.as_str(), payload.map(|b| *b)) {
            ("None", None) => None,
            ("Some", Some(Value::Tuple(vs))) if vs.len() == 2 => {
                let mut it = vs.into_iter();
                Some((as_point(it.next().unwrap()), as_point(it.next().unwrap())))
            }
            (other, _) => panic!("expected a bbox option, got variant '{other}'"),
        },
        other => panic!("expected an option, got {other:?}"),
    }
}

// ============================================================================
// 1. `unite-graphics` (A12) / `clip-graphics-by-path` (A13) construct the
//    new `GraphicsElem` container variants, and are unbound under V0_0.
// ============================================================================

#[test]
fn unite_and_clip_construct() {
    let mono = Mono;
    let mut interp = Interp::new(&mono);
    interp.version = RustyfiVersion::V0_1;
    let env = primitives::base_env_with_version(RustyfiVersion::V0_1);

    let path = rect_path(&mut interp, &env, 0.0, 0.0, 10.0, 10.0);
    let f1 = fill_val(&mut interp, &env, path.clone());
    let f2 = fill_val(&mut interp, &env, path.clone());

    let group = call(
        &mut interp,
        &env,
        "unite-graphics",
        vec![Value::List(vec![f1, f2])],
    );
    match &group {
        Value::Graphics(GraphicsElem::Group(gs)) => assert_eq!(gs.len(), 2),
        other => panic!("expected a Group, got {other:?}"),
    }

    let clip = call(
        &mut interp,
        &env,
        "clip-graphics-by-path",
        vec![path, group],
    );
    match &clip {
        Value::Graphics(GraphicsElem::Clip(_, inner)) => assert_eq!(inner.len(), 1),
        other => panic!("expected a Clip, got {other:?}"),
    }

    // Both unbound under V0_0 — env AND type mirror.
    let env006 = primitives::base_env();
    for name in ["unite-graphics", "clip-graphics-by-path"] {
        assert!(
            env006.lookup(name).is_none(),
            "{name} must be unbound under V0_0"
        );
        assert!(
            prim_types::primitive_type(name).is_none(),
            "{name} must have no type under V0_0"
        );
        assert!(
            prim_types::primitive_type_with_version(name, RustyfiVersion::V0_1).is_some(),
            "{name} must have a type under V0_1"
        );
    }
}

// ============================================================================
// 2. `get-graphics-bbox`'s `Option` fork (R3).
// ============================================================================

#[test]
fn bbox_option_semantics() {
    let mono = Mono;
    let mut interp = Interp::new(&mono);
    interp.version = RustyfiVersion::V0_1;
    let env = primitives::base_env_with_version(RustyfiVersion::V0_1);

    // `unite-graphics []` -> an empty `Group` -> bbox `None`.
    let empty_group = call(
        &mut interp,
        &env,
        "unite-graphics",
        vec![Value::List(vec![])],
    );
    let bbox = call(&mut interp, &env, "get-graphics-bbox", vec![empty_group]);
    assert_eq!(as_bbox_option(bbox), None);

    // A `Group` of two disjoint fills -> `Some` of their union.
    let p1 = rect_path(&mut interp, &env, 0.0, 0.0, 10.0, 10.0);
    let f1 = fill_val(&mut interp, &env, p1);
    let p2 = rect_path(&mut interp, &env, 20.0, 20.0, 10.0, 10.0);
    let f2 = fill_val(&mut interp, &env, p2);
    let group = call(
        &mut interp,
        &env,
        "unite-graphics",
        vec![Value::List(vec![f1, f2])],
    );
    let bbox = call(&mut interp, &env, "get-graphics-bbox", vec![group]);
    assert_eq!(
        as_bbox_option(bbox),
        Some(((0.0, 0.0), (30.0, 30.0))),
        "union of (0,0)-(10,10) and (20,20)-(30,30)"
    );

    // A `Clip`'s bbox is the CLIP PATH's own bbox, ignoring `contents`
    // (upstream `graphicD.ml:50-52` parity).
    let clip_path = rect_path(&mut interp, &env, 100.0, 100.0, 50.0, 50.0);
    let inner_path = rect_path(&mut interp, &env, 0.0, 0.0, 10.0, 10.0);
    let inner = fill_val(&mut interp, &env, inner_path);
    let clip = call(
        &mut interp,
        &env,
        "clip-graphics-by-path",
        vec![clip_path, inner],
    );
    let bbox = call(&mut interp, &env, "get-graphics-bbox", vec![clip]);
    assert_eq!(as_bbox_option(bbox), Some(((100.0, 100.0), (150.0, 150.0))));

    // V0_0 `get-graphics-bbox` stays un-optioned (existing production
    // callers `stdlib_tier0.rs`/`math_fraction_radical.rs` keep passing).
    let mut interp006 = Interp::new(&mono);
    let env006 = primitives::base_env();
    let p006 = rect_path(&mut interp006, &env006, 0.0, 0.0, 10.0, 10.0);
    let f006 = fill_val(&mut interp006, &env006, p006);
    let bbox006 = call(&mut interp006, &env006, "get-graphics-bbox", vec![f006]);
    match bbox006 {
        Value::Tuple(vs) => assert_eq!(vs.len(), 2, "v0.0.6 side must stay a bare pair"),
        other => panic!("expected an un-optioned tuple, got {other:?}"),
    }
}

// ============================================================================
// 3. `coerce_graphics_result`: a graphics-producing callback's result forks
//    per version (H1: `inline-graphics`; R2: `tabular`'s rules callback).
// ============================================================================

fn var(name: &str) -> Ast {
    Ast::Var(name.to_string(), Span::default())
}

fn app1(f: Ast, a: Ast) -> Ast {
    Ast::Apply(Box::new(f), Box::new(a))
}

fn apply_all(name: &str, args: Vec<Ast>) -> Ast {
    args.into_iter().fold(var(name), app1)
}

fn point_ast(x: f64, y: f64) -> Ast {
    Ast::Tuple(vec![Ast::Length(Length::pt(x)), Ast::Length(Length::pt(y))])
}

/// `fill (Gray 0.) (start-path (0,0) |> line-to (10,0) |> line-to (10,10) |>
/// close-with-line)` — a bare `graphics` value (no `|>` in this port's
/// frontend, so an application chain — the `inline_graphics_outer.rs`
/// pattern).
fn fill_ast() -> Ast {
    let path = apply_all(
        "close-with-line",
        vec![apply_all(
            "line-to",
            vec![
                point_ast(10.0, 10.0),
                apply_all(
                    "line-to",
                    vec![
                        point_ast(10.0, 0.0),
                        apply_all("start-path", vec![point_ast(0.0, 0.0)]),
                    ],
                ),
            ],
        )],
    );
    apply_all(
        "fill",
        vec![
            Ast::Ctor("Gray".to_string(), Some(Box::new(Ast::Float(0.0)))),
            path,
        ],
    )
}

/// `fun _pt -> <body>` — a 1-ary closure ignoring its argument (matches
/// `inline-graphics`'s eager-at-origin callback shape).
fn closure1(interp: &mut Interp, env: &BaseEnv, body: Ast) -> Value {
    let ast = Ast::Lambda("_pt".to_string(), Rc::new(body));
    interp.eval(env, &ast).expect("closure AST must evaluate")
}

/// `fun _xs _ys -> <body>` — a 2-ary closure ignoring both arguments
/// (matches `tabular`'s rules-callback shape).
fn closure2(interp: &mut Interp, env: &BaseEnv, body: Ast) -> Value {
    let ast = Ast::Lambda(
        "_xs".to_string(),
        Rc::new(Ast::Lambda("_ys".to_string(), Rc::new(body))),
    );
    interp.eval(env, &ast).expect("closure AST must evaluate")
}

#[test]
fn inline_graphics_callback_result_coerces_per_version() {
    let mono = Mono;

    // V0_1: the callback returns ONE `graphics` value (no list wrapper) —
    // this is what a 0.1 program's `inline-graphics` callback must do.
    let mut interp01 = Interp::new(&mono);
    interp01.version = RustyfiVersion::V0_1;
    let env01 = primitives::base_env_with_version(RustyfiVersion::V0_1);
    let cb01 = closure1(&mut interp01, &env01, fill_ast());
    let v = call(
        &mut interp01,
        &env01,
        "inline-graphics",
        vec![
            Value::Length(Length::pt(10.0)),
            Value::Length(Length::pt(10.0)),
            Value::Length(Length::ZERO),
            cb01,
        ],
    );
    match v {
        Value::InlineBoxes(boxes) => {
            assert_eq!(boxes.len(), 1);
            match &boxes[0] {
                rustyfi_backend::HorzBox::Pure(rustyfi_backend::PureHorzBox::Graphics {
                    elems,
                    ..
                }) => assert_eq!(elems.len(), 1, "one bare graphics -> one elem"),
                other => panic!("expected a Graphics box, got {other:?}"),
            }
        }
        other => panic!("expected inline-boxes, got {other:?}"),
    }

    // V0_1 STRICT: a callback that returns a LIST (the v0.0.6 shape) is a
    // runtime type error under V0_1 — `coerce_graphics_result` doesn't
    // tolerantly decode across versions (the type checker would already
    // have rejected this in a real program).
    let mut interp01b = Interp::new(&mono);
    interp01b.version = RustyfiVersion::V0_1;
    let cb01_list = closure1(&mut interp01b, &env01, Ast::List(vec![fill_ast()]));
    let err = try_call(
        &mut interp01b,
        &env01,
        "inline-graphics",
        vec![
            Value::Length(Length::pt(10.0)),
            Value::Length(Length::pt(10.0)),
            Value::Length(Length::ZERO),
            cb01_list,
        ],
    )
    .unwrap_err();
    assert!(err.msg.contains("graphics"), "got: {}", err.msg);

    // V0_0 twin: the callback returns a LIST — unchanged existing shape.
    let mut interp006 = Interp::new(&mono);
    let env006 = primitives::base_env();
    let cb006 = closure1(&mut interp006, &env006, Ast::List(vec![fill_ast()]));
    let v = call(
        &mut interp006,
        &env006,
        "inline-graphics",
        vec![
            Value::Length(Length::pt(10.0)),
            Value::Length(Length::pt(10.0)),
            Value::Length(Length::ZERO),
            cb006,
        ],
    );
    match v {
        Value::InlineBoxes(boxes) => match &boxes[0] {
            rustyfi_backend::HorzBox::Pure(rustyfi_backend::PureHorzBox::Graphics {
                elems,
                ..
            }) => assert_eq!(elems.len(), 1),
            other => panic!("expected a Graphics box, got {other:?}"),
        },
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

#[test]
fn tabular_rules_callback_result_coerces_per_version() {
    let mono = Mono;
    let empty_row = Value::List(vec![Value::List(vec![Value::Ctor(
        "EmptyCell".to_string(),
        None,
    )])]);

    // V0_1: rules callback returns ONE `graphics` value.
    let mut interp01 = Interp::new(&mono);
    interp01.version = RustyfiVersion::V0_1;
    let env01 = primitives::base_env_with_version(RustyfiVersion::V0_1);
    let rulesf01 = closure2(&mut interp01, &env01, fill_ast());
    let v = call(
        &mut interp01,
        &env01,
        "tabular",
        vec![empty_row.clone(), rulesf01],
    );
    match v {
        Value::InlineBoxes(boxes) => match &boxes[0] {
            rustyfi_backend::HorzBox::Pure(rustyfi_backend::PureHorzBox::Tabular(tab)) => {
                assert_eq!(tab.rules.len(), 1)
            }
            other => panic!("expected a Tabular box, got {other:?}"),
        },
        other => panic!("expected inline-boxes, got {other:?}"),
    }

    // V0_0 twin: rules callback returns a LIST — unchanged.
    let mut interp006 = Interp::new(&mono);
    let env006 = primitives::base_env();
    let rulesf006 = closure2(&mut interp006, &env006, Ast::List(vec![fill_ast()]));
    let v = call(
        &mut interp006,
        &env006,
        "tabular",
        vec![empty_row, rulesf006],
    );
    match v {
        Value::InlineBoxes(boxes) => match &boxes[0] {
            rustyfi_backend::HorzBox::Pure(rustyfi_backend::PureHorzBox::Tabular(tab)) => {
                assert_eq!(tab.rules.len(), 1)
            }
            other => panic!("expected a Tabular box, got {other:?}"),
        },
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

// ============================================================================
// H3-H6: the deco family's coercion happens at `apply_deco`'s deferred-fire
// time (far from any prim body) — exercised through an actually-fired
// inline-frame deco, the `frame_deco_firing.rs` harness.
// ============================================================================

#[test]
fn deco_callback_result_coerces_per_version_when_fired() {
    use rustyfi_backend::{DecoId, Page, PageGeometry, PlacedLine, PureHorzBox};
    use rustyfi_lang::eval::DecoEntry;
    use rustyfi_lang::value::DocumentValue;

    fn lambda4(body: Ast) -> Ast {
        Ast::Lambda(
            "pt".to_string(),
            Rc::new(Ast::Lambda(
                "w".to_string(),
                Rc::new(Ast::Lambda(
                    "h".to_string(),
                    Rc::new(Ast::Lambda("d".to_string(), Rc::new(body))),
                )),
            )),
        )
    }

    fn geometry() -> PageGeometry {
        PageGeometry {
            paper_width: Length::pt(400.0),
            paper_height: Length::pt(300.0),
            text_origin: (Length::pt(0.0), Length::pt(0.0)),
            text_width: Length::pt(400.0),
            text_height: Length::pt(300.0),
        }
    }

    fn doc_with_pages(pages: Vec<Page>) -> DocumentValue {
        DocumentValue {
            geometry: geometry(),
            pages,
            images: Vec::new(),
            extras: Default::default(),
            reflow_source: None,
            reflow_links: Vec::new(),
            reflow_dests: Vec::new(),
        }
    }

    fn frame_page() -> Page {
        let frame = PureHorzBox::Frame {
            width: Length::pt(30.0),
            height: Length::pt(10.0),
            depth: Length::pt(2.0),
            deco: DecoId(0),
            contents: Vec::new(),
        };
        Page {
            body_lines: usize::MAX,
            lines: vec![PlacedLine {
                x: Length::pt(50.0),
                baseline_y: Length::pt(100.0),
                contents: vec![(Length::ZERO, frame)],
            }],
        }
    }

    let mono = Mono;

    // V0_1: the deco closure returns ONE `graphics` value.
    let mut interp01 = Interp::new(&mono);
    interp01.version = RustyfiVersion::V0_1;
    let env01 = primitives::base_env_with_version(RustyfiVersion::V0_1);
    let deco01 = interp01
        .eval(&env01, &lambda4(fill_ast()))
        .expect("deco AST must evaluate");
    interp01.decos.push(DecoEntry::Inline { deco: deco01 });
    let doc01 = doc_with_pages(vec![frame_page()]);
    rustyfi_lang::fire_hooks(&mut interp01, &doc01).expect("fire_hooks must succeed under V0_1");
    assert_eq!(interp01.page_graphics[0].len(), 1);
    assert!(matches!(
        interp01.page_graphics[0][0],
        GraphicsElem::Fill(Color::Gray(_), _)
    ));

    // V0_0 twin: the deco closure returns a LIST — unchanged.
    let mut interp006 = Interp::new(&mono);
    let env006 = primitives::base_env();
    let deco006 = interp006
        .eval(&env006, &lambda4(Ast::List(vec![fill_ast()])))
        .expect("deco AST must evaluate");
    interp006.decos.push(DecoEntry::Inline { deco: deco006 });
    let doc006 = doc_with_pages(vec![frame_page()]);
    rustyfi_lang::fire_hooks(&mut interp006, &doc006)
        .expect("fire_hooks must succeed under V0_0");
    assert_eq!(interp006.page_graphics[0].len(), 1);
    assert!(matches!(
        interp006.page_graphics[0][0],
        GraphicsElem::Fill(Color::Gray(_), _)
    ));

    // V0_1 STRICT: a deco closure returning a LIST is a runtime type error
    // under V0_1 (same `coerce_graphics_result` strictness as
    // `inline-graphics` above).
    let mut interp01b = Interp::new(&mono);
    interp01b.version = RustyfiVersion::V0_1;
    let deco01_list = interp01b
        .eval(&env01, &lambda4(Ast::List(vec![fill_ast()])))
        .expect("deco AST must evaluate");
    interp01b
        .decos
        .push(DecoEntry::Inline { deco: deco01_list });
    let doc01b = doc_with_pages(vec![frame_page()]);
    let err = rustyfi_lang::fire_hooks(&mut interp01b, &doc01b).unwrap_err();
    assert!(err.msg.contains("graphics"), "got: {}", err.msg);
}
