//! Slice 1 unit coverage: the
//! `hook-page-break`/`register-cross-reference`/`get-cross-reference`
//! primitives, and the `fire_hooks` seam itself invoked directly against a
//! hand-built `DocumentValue` — the same `Ast`-apply-chain style
//! `prims_phase4.rs`/`images.rs` use, no parser involved. The full 2-pass
//! fixpoint round trip (compile → fire hooks → re-run) is covered
//! separately by `rustyfi`'s e2e `hook-page.saty` fixture.

use rustyfi_backend::{
    FontKey, FontMetrics, HookId, HorzBox, Length, Page, PageGeometry, PlacedLine, PureHorzBox,
};
use rustyfi_lang::ast::Ast;
use rustyfi_lang::crossref::Verdict;
use rustyfi_lang::eval;
use rustyfi_lang::primitives;
use rustyfi_lang::value::{DocumentValue, Value};
use rustyfi_syntax::Span;
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

fn var(name: &str) -> Ast {
    Ast::Var(name.to_string(), Span::default())
}

fn app1(f: Ast, a: Ast) -> Ast {
    Ast::Apply(Box::new(f), Box::new(a))
}

fn app2(name: &str, a: Ast, b: Ast) -> Ast {
    app1(app1(var(name), a), b)
}

fn str_lit(s: &str) -> Ast {
    Ast::Str(s.to_string())
}

/// `fun pbinfo _ -> unit` — a minimal, well-typed-enough (this is the
/// untyped tree-walker, no typecheck involved) hook closure body.
fn trivial_hook_closure() -> Ast {
    Ast::Lambda(
        "pbinfo".to_string(),
        Rc::new(Ast::Lambda("_".to_string(), Rc::new(Ast::Unit))),
    )
}

#[test]
fn hook_page_break_pushes_a_closure_and_returns_a_hookid_box() {
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let ast = app1(var("hook-page-break"), trivial_hook_closure());
    let v = interp.eval(&env, &ast).expect("evaluation should succeed");
    let Value::InlineBoxes(boxes) = v else {
        panic!("expected inline-boxes")
    };
    assert_eq!(
        boxes,
        vec![HorzBox::Pure(PureHorzBox::HookPageBreak { id: HookId(0) })]
    );
    assert_eq!(
        interp.hooks.len(),
        1,
        "the closure must be pushed onto the hook table"
    );
}

#[test]
fn register_then_get_cross_reference_round_trips() {
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let ast = Ast::Sequential(
        Box::new(app2("register-cross-reference", str_lit("k"), str_lit("v"))),
        Box::new(app1(var("get-cross-reference"), str_lit("k"))),
    );
    let v = interp.eval(&env, &ast).expect("evaluation should succeed");
    match v {
        Value::Ctor(name, Some(payload)) => {
            assert_eq!(name, "Some");
            assert!(matches!(*payload, Value::Str(s) if s == "v"));
        }
        other => panic!("expected Some(\"v\"), got {other:?}"),
    }
}

#[test]
fn get_cross_reference_miss_returns_none_and_marks_it_unresolved() {
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let ast = app1(var("get-cross-reference"), str_lit("missing"));
    let v = interp.eval(&env, &ast).expect("evaluation should succeed");
    match v {
        Value::Ctor(name, None) => assert_eq!(name, "None"),
        other => panic!("expected None, got {other:?}"),
    }
    // No `register` ever happened, so the table isn't "changed" — but the
    // miss must still surface in `CanTerminate`'s unresolved list.
    assert_eq!(
        interp.crossrefs.borrow_mut().verdict(),
        Verdict::CanTerminate(vec!["missing".to_string()])
    );
}

/// `probe-cross-reference` on a missing key must NOT record a miss — unlike
/// `get-cross-reference`'s miss test above, whose `verdict()` payload carries
/// the missed key, this one's payload must be empty.
#[test]
fn probe_miss_records_no_unresolved_and_does_not_retrial() {
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let ast = app1(var("probe-cross-reference"), str_lit("missing"));
    let v = interp.eval(&env, &ast).expect("evaluation should succeed");
    match v {
        Value::Ctor(name, None) => assert_eq!(name, "None"),
        other => panic!("expected None, got {other:?}"),
    }
    assert_eq!(
        interp.crossrefs.borrow_mut().verdict(),
        Verdict::CanTerminate(vec![]),
        "probing an absent key must not force another fixpoint trial"
    );
}

#[test]
fn probe_hit_after_register_round_trips() {
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let ast = Ast::Sequential(
        Box::new(app2("register-cross-reference", str_lit("k"), str_lit("v"))),
        Box::new(app1(var("probe-cross-reference"), str_lit("k"))),
    );
    let v = interp.eval(&env, &ast).expect("evaluation should succeed");
    match v {
        Value::Ctor(name, Some(payload)) => {
            assert_eq!(name, "Some");
            assert!(matches!(*payload, Value::Str(s) if s == "v"));
        }
        other => panic!("expected Some(\"v\"), got {other:?}"),
    }
}

/// (iii) `fire_hooks` over a hand-built one-hook `DocumentValue` invokes the
/// closure with `page-number = 1` — no compile/fixpoint driver involved, so
/// this pins the seam itself: `PureHorzBox::HookPageBreak`'s `HookId` looks
/// up the right closure in `interp.hooks`, and the `pbinfo` record it's
/// called with carries the placed page's 1-based number.
#[test]
fn fire_hooks_invokes_the_closure_with_the_correct_page_number() {
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);

    // `fun pbinfo _ -> register-cross-reference "seen" (arabic pbinfo#page-number)`
    let closure_ast = Ast::Lambda(
        "pbinfo".to_string(),
        Rc::new(Ast::Lambda(
            "_".to_string(),
            Rc::new(app2(
                "register-cross-reference",
                str_lit("seen"),
                app1(
                    var("arabic"),
                    Ast::AccessField(
                        Box::new(var("pbinfo")),
                        "page-number".to_string(),
                        Span::default(),
                    ),
                ),
            )),
        )),
    );
    let closure = interp
        .eval(&env, &closure_ast)
        .expect("closure should evaluate to a Value::CompiledClosure");
    assert!(matches!(closure, Value::CompiledClosure { .. }));
    interp.hooks.push(closure);

    let doc = DocumentValue {
        geometry: PageGeometry::default(),
        pages: vec![Page {
            body_lines: usize::MAX,
            lines: vec![PlacedLine {
                x: Length::ZERO,
                baseline_y: Length::pt(100.0),
                contents: vec![(Length::ZERO, PureHorzBox::HookPageBreak { id: HookId(0) })],
            }],
        }],
        images: Vec::new(),
        extras: Default::default(),
        reflow_source: None,
        reflow_links: Vec::new(),
        reflow_dests: Vec::new(),
    };

    rustyfi_lang::fire_hooks(&mut interp, &doc).expect("fire_hooks should succeed");
    assert_eq!(
        interp.crossrefs.borrow().probe("seen"),
        Some("1".to_string()),
        "the hook must have seen page-number = 1"
    );
}

/// Same as above but with a second page, pinning that `fire_hooks` numbers
/// pages 1-based in document order rather than, say, always reporting 1.
#[test]
fn fire_hooks_numbers_pages_one_based_in_document_order() {
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);

    let closure_ast = Ast::Lambda(
        "pbinfo".to_string(),
        Rc::new(Ast::Lambda(
            "_".to_string(),
            Rc::new(app2(
                "register-cross-reference",
                str_lit("seen"),
                app1(
                    var("arabic"),
                    Ast::AccessField(
                        Box::new(var("pbinfo")),
                        "page-number".to_string(),
                        Span::default(),
                    ),
                ),
            )),
        )),
    );
    let closure = interp.eval(&env, &closure_ast).unwrap();
    interp.hooks.push(closure);

    let hook_free_page = Page {
        body_lines: usize::MAX,
        lines: vec![PlacedLine {
            x: Length::ZERO,
            baseline_y: Length::pt(50.0),
            contents: vec![],
        }],
    };
    let hook_page = Page {
        body_lines: usize::MAX,
        lines: vec![PlacedLine {
            x: Length::ZERO,
            baseline_y: Length::pt(50.0),
            contents: vec![(Length::ZERO, PureHorzBox::HookPageBreak { id: HookId(0) })],
        }],
    };
    let doc = DocumentValue {
        geometry: PageGeometry::default(),
        pages: vec![hook_free_page, hook_page],
        images: Vec::new(),
        extras: Default::default(),
        reflow_source: None,
        reflow_links: Vec::new(),
        reflow_dests: Vec::new(),
    };

    rustyfi_lang::fire_hooks(&mut interp, &doc).expect("fire_hooks should succeed");
    assert_eq!(
        interp.crossrefs.borrow().probe("seen"),
        Some("2".to_string()),
        "the hook sits on the 2nd page, so it must see page-number = 2"
    );
}
