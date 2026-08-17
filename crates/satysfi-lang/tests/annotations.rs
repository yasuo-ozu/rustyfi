//! `docs/plans/hooks-annotations-crossref.md` §B/§D: the prim surface
//! `annot.satyh` needs (`get-leftmost-script`/`get-rightmost-script`,
//! `inline-frame-breakable`, `register-destination`,
//! `register-link-to-uri`, `register-link-to-location`) — build-order step
//! 9, stdja's last unported `@require`. Two halves, mirroring
//! `tests/context_box.rs`'s split:
//! - **Typecheck** (real source text through `parse_file` ->
//!   `elaborate::elaborate_program` -> `typecheck::typecheck`) — pins each
//!   new prim's declared signature end-to-end, including the surface syntax
//!   `annot.satyh` actually uses.
//! - **Eval** (direct `Ast` apply chains through `eval::Interp` +
//!   `primitives::base_env()`, mirroring `tests/prims_phase4.rs`'s style) —
//!   `inline-frame-breakable` builds a real atomic `PureHorzBox::Frame` and
//!   interns its decoset's `decoS`; the register-* prims are FAITHFUL as of
//!   roadmap Group A (docs/plans/hooks-annotations-crossref.md §B): they
//!   error outside `fire_hooks`' `current_page` window (§0.5) and record a
//!   real `Annot`/`NamedDest` inside it — see `primitives.rs`'s
//!   `prim_register_destination`/`register_link` doc comments.
//!
//! `annot.satyh` itself is loaded by the capstone suite (`register-*`/
//! `inline-frame-breakable` reach the PDF end-to-end via `crates/satysfi-cli/
//! tests/fixtures/href.saty`) — this file only proves the primitive surface
//! type-checks against `vminstdef.yaml` and evaluates correctly in
//! isolation.

use satysfi_backend::{FontKey, FontMetrics, HorzBox, Length, PureHorzBox};
use satysfi_lang::ast::Ast;
use satysfi_lang::eval;
use satysfi_lang::prim_types;
use satysfi_lang::primitives;
use satysfi_lang::value::Value;
use satysfi_lang::{elaborate, typecheck, CompileError};
use satysfi_syntax::Span;

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

// ============================================================================
// Registration coverage
// ============================================================================

const NEW_NAMES: &[&str] = &[
    "get-leftmost-script",
    "get-rightmost-script",
    "inline-frame-breakable",
    "register-destination",
    "register-link-to-uri",
    "register-link-to-location",
];

#[test]
fn every_new_primitive_resolves_in_base_env_and_has_a_registered_type() {
    let env = primitives::base_env();
    for name in NEW_NAMES {
        assert!(
            env.lookup(name).is_some(),
            "primitive `{name}` is not bound in base_env()"
        );
        assert!(
            prim_types::primitive_type(name).is_some(),
            "primitive `{name}` has no registered type"
        );
    }
}

// ============================================================================
// Typecheck half
// ============================================================================

fn typecheck_str(src: &str) -> Result<(), CompileError> {
    let file = satysfi_syntax::parse_file(src)?;
    let env = primitives::base_env();
    let scope = elaborate::Scope::new(env.names());
    let program = elaborate::elaborate_program(&file, &scope)?;
    typecheck::typecheck(&program)?;
    Ok(())
}

fn assert_well_typed(src: &str) {
    if let Err(e) = typecheck_str(src) {
        panic!("expected {src:?} to type-check, got error: {e}");
    }
}

fn assert_type_error(src: &str) {
    match typecheck_str(src) {
        Ok(()) => panic!("expected {src:?} to be rejected by the typechecker, but it passed"),
        Err(CompileError::Type(_)) => {}
        Err(other) => panic!("expected {src:?} to fail with a type error, got: {other}"),
    }
}

#[test]
fn get_leftmost_and_rightmost_script_typecheck() {
    assert_well_typed("get-leftmost-script inline-nil");
    assert_well_typed("get-rightmost-script inline-nil");
}

#[test]
fn get_leftmost_script_rejects_a_non_inline_boxes_argument() {
    assert_type_error("get-leftmost-script 3");
}

#[test]
fn inline_frame_breakable_typechecks() {
    assert_well_typed(
        "let mydeco pt l1 l2 l3 = []
         in
         inline-frame-breakable (0pt, 0pt, 0pt, 0pt) (mydeco, mydeco, mydeco, mydeco) inline-nil",
    );
}

#[test]
fn inline_frame_breakable_rejects_a_three_element_paddings_tuple() {
    assert_type_error(
        "let mydeco pt l1 l2 l3 = []
         in
         inline-frame-breakable (0pt, 0pt, 0pt) (mydeco, mydeco, mydeco, mydeco) inline-nil",
    );
}

#[test]
fn register_destination_typechecks() {
    assert_well_typed("register-destination `chapter1` (10pt, 20pt)");
}

#[test]
fn register_destination_rejects_a_non_string_name() {
    assert_type_error("register-destination 3 (10pt, 20pt)");
}

#[test]
fn register_link_to_uri_typechecks_with_and_without_a_border() {
    assert_well_typed("register-link-to-uri `https://example.com` (0pt, 0pt) 10pt 10pt 10pt None");
    assert_well_typed(
        "register-link-to-uri `https://example.com` (0pt, 0pt) 10pt 10pt 10pt \
         (Some (1pt, Gray 0.5))",
    );
}

#[test]
fn register_link_to_uri_rejects_a_malformed_border_argument() {
    assert_type_error("register-link-to-uri `u` (0pt, 0pt) 10pt 10pt 10pt 5");
}

#[test]
fn register_link_to_location_typechecks() {
    assert_well_typed(
        "register-link-to-location `chapter1` (0pt, 0pt) 10pt 10pt 10pt None",
    );
}

/// The actual shape `annot.satyh:29-40`'s `\href` body reduces to (minus the
/// `direct \href` command sugar and `read-inline`, out of scope here): guard
/// both edges with `get-leftmost-script`/`get-rightmost-script` +
/// `script-guard`, wrap the body in `inline-frame-breakable`, concatenate
/// with `++`. Proves the whole prim surface composes, not just each prim in
/// isolation.
#[test]
fn the_href_shaped_composition_typechecks() {
    assert_well_typed(
        "let guard ib =
           match get-leftmost-script ib with
           | Some s -> script-guard s inline-nil
           | None -> inline-nil
         in
         let mydeco pt l1 l2 l3 = []
         in
         let framed =
           inline-frame-breakable (0pt, 0pt, 0pt, 0pt) (mydeco, mydeco, mydeco, mydeco) inline-nil
         in
         (guard inline-nil) ++ framed ++ (guard inline-nil)",
    );
}

// ============================================================================
// Eval half — `Ast` apply chains through `eval::Interp`, mirroring
// `prims_phase4.rs`'s style; no parser involved.
// ============================================================================

fn var(name: &str) -> Ast {
    Ast::Var(name.to_string(), Span::default())
}

fn apply_all(name: &str, args: Vec<Ast>) -> Ast {
    args.into_iter()
        .fold(var(name), |f, a| Ast::Apply(Box::new(f), Box::new(a)))
}

fn str_lit(s: &str) -> Ast {
    Ast::Str(s.to_string())
}

fn len(pt: f64) -> Ast {
    Ast::Length(Length::pt(pt))
}

fn point(x_pt: f64, y_pt: f64) -> Ast {
    Ast::Tuple(vec![len(x_pt), len(y_pt)])
}

fn border_none() -> Ast {
    Ast::Ctor("None".to_string(), None)
}

fn border_some(width_pt: f64, gray: f64) -> Ast {
    Ast::Ctor(
        "Some".to_string(),
        Some(Box::new(Ast::Tuple(vec![
            len(width_pt),
            Ast::Ctor("Gray".to_string(), Some(Box::new(Ast::Float(gray)))),
        ]))),
    )
}

fn run(ast: &Ast) -> Value {
    try_run(ast).expect("evaluation should succeed")
}

fn try_run(ast: &Ast) -> Result<Value, eval::EvalError> {
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    interp.eval(&env, ast)
}

#[test]
fn get_leftmost_and_rightmost_script_return_none_stand_in() {
    for name in ["get-leftmost-script", "get-rightmost-script"] {
        match run(&apply_all(name, vec![var("inline-nil")])) {
            Value::Ctor(ctor, None) => assert_eq!(ctor, "None"),
            other => panic!("expected `{name} inline-nil` to evaluate to None, got {other:?}"),
        }
    }
}

/// The four-closure `deco-set` argument is popped and only its first
/// element (`decoS`) is kept/interned (see the prim's doc comment — the
/// atomic `PureHorzBox::Frame` never splits, so only the whole-frame closure
/// is reachable); a dummy `(fun _ _ _ _ -> [])`-shaped tuple stands in here
/// since the closures themselves are never invoked at construction time
/// (only later, by `fire_hooks`/`apply_deco`).
fn dummy_decoset() -> Ast {
    Ast::Tuple(vec![Ast::Unit, Ast::Unit, Ast::Unit, Ast::Unit])
}

#[test]
fn inline_frame_breakable_builds_an_atomic_frame_and_interns_the_decoset() {
    let ast = apply_all(
        "inline-frame-breakable",
        vec![
            Ast::Tuple(vec![len(2.0), len(3.0), len(4.0), len(5.0)]),
            dummy_decoset(),
            var("inline-nil"),
        ],
    );
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let v = interp.eval(&env, &ast).expect("evaluation should succeed");
    let Value::InlineBoxes(boxes) = v else {
        panic!("expected inline-boxes")
    };
    assert_eq!(
        boxes,
        vec![HorzBox::Pure(PureHorzBox::Frame {
            width: Length::pt(5.0),  // paddingL + inner(0) + paddingR
            height: Length::pt(4.0), // inner(0) + paddingT
            depth: Length::pt(5.0),  // inner(0) + paddingB
            deco: satysfi_backend::DecoId(0),
            contents: Vec::new(),
        })],
        "an atomic PureHorzBox::Frame, padded on all four sides, empty \
         contents for an empty inner run"
    );
    assert_eq!(
        interp.decos.len(),
        1,
        "the deco-set's first (decoS) closure must be interned into interp.decos"
    );
}

/// `register-destination`/`register-link-to-*` are gated on
/// `interp.current_page` (§0.5: they only succeed while `fire_hooks` is
/// walking a page — i.e. from a hook or a fired decoration).
fn eval_during_page_break(ast: &Ast) -> Result<(Value, eval::Interp<'static>), eval::EvalError> {
    // `Mono` is a unit struct (no data), so leaking one `'static` reference
    // is cheap and lets the returned `Interp` outlive this function without
    // fighting the borrow checker over `mono`'s local lifetime.
    let metrics: &'static Mono = Box::leak(Box::new(Mono));
    let env = primitives::base_env();
    let mut interp = eval::Interp::new(metrics);
    interp.current_page = Some(0);
    let v = interp.eval(&env, ast)?;
    Ok((v, interp))
}

#[test]
fn register_destination_outside_a_page_break_errors() {
    let ast = apply_all(
        "register-destination",
        vec![str_lit("chapter1"), point(10.0, 20.0)],
    );
    let err = try_run(&ast).expect_err("must error outside fire_hooks' current_page window");
    assert!(
        err.msg.contains("page breaking"),
        "error should name the during-page-break gate: {}",
        err.msg
    );
}

#[test]
fn register_destination_during_a_page_break_records_a_named_destination() {
    let ast = apply_all(
        "register-destination",
        vec![str_lit("chapter1"), point(10.0, 20.0)],
    );
    let (v, interp) = eval_during_page_break(&ast).expect("must succeed inside the window");
    assert!(matches!(v, Value::Unit));
    assert_eq!(interp.destinations.len(), 1);
    let d = &interp.destinations[0];
    assert_eq!(d.page, 0);
    assert_eq!(d.name, "nameddest0");
    assert_eq!(d.x, Length::pt(10.0));
    assert_eq!(d.y, Length::pt(20.0));
}

#[test]
fn register_link_to_uri_outside_a_page_break_errors() {
    let ast = apply_all(
        "register-link-to-uri",
        vec![
            str_lit("https://example.com"),
            point(0.0, 0.0),
            len(10.0),
            len(10.0),
            len(10.0),
            border_none(),
        ],
    );
    let err = try_run(&ast).expect_err("must error outside fire_hooks' current_page window");
    assert!(
        err.msg.contains("page breaking"),
        "error should name the during-page-break gate: {}",
        err.msg
    );
}

#[test]
fn register_link_to_uri_during_a_page_break_records_an_annot_with_no_border() {
    let ast = apply_all(
        "register-link-to-uri",
        vec![
            str_lit("https://example.com"),
            point(0.0, 0.0),
            len(10.0),
            len(10.0),
            len(10.0),
            border_none(),
        ],
    );
    let (v, interp) = eval_during_page_break(&ast).expect("must succeed inside the window");
    assert!(matches!(v, Value::Unit));
    assert_eq!(interp.annotations.len(), 1);
    let a = &interp.annotations[0];
    assert_eq!(a.page, 0);
    assert_eq!(a.border, None);
    assert_eq!(
        a.rect,
        (Length::pt(0.0), Length::pt(-10.0), Length::pt(10.0), Length::pt(10.0)),
        "rect = (x, y - depth, x + width, y + height)"
    );
    assert_eq!(
        a.action,
        satysfi_backend::AnnotAction::Uri("https://example.com".to_string())
    );
}

#[test]
fn register_link_to_uri_during_a_page_break_records_an_annot_with_a_border() {
    let ast = apply_all(
        "register-link-to-uri",
        vec![
            str_lit("https://example.com"),
            point(0.0, 0.0),
            len(10.0),
            len(10.0),
            len(10.0),
            border_some(1.0, 0.5),
        ],
    );
    let (_v, interp) = eval_during_page_break(&ast).expect("must succeed inside the window");
    assert_eq!(interp.annotations.len(), 1);
    assert_eq!(
        interp.annotations[0].border,
        Some((Length::pt(1.0), satysfi_backend::Color::Gray(0.5)))
    );
}

#[test]
fn register_link_to_uri_rejects_a_malformed_border_argument_without_panicking() {
    let ast = apply_all(
        "register-link-to-uri",
        vec![
            str_lit("https://example.com"),
            point(0.0, 0.0),
            len(1.0),
            len(1.0),
            len(1.0),
            Ast::Int(5), // not an option
        ],
    );
    let err = try_run(&ast).expect_err("a malformed border must be a clean EvalError, not a panic");
    assert!(
        err.msg.contains("option"),
        "error should name the expected shape: {}",
        err.msg
    );
}

#[test]
fn register_link_to_location_outside_a_page_break_errors() {
    let ast = apply_all(
        "register-link-to-location",
        vec![
            str_lit("chapter1"),
            point(0.0, 0.0),
            len(10.0),
            len(10.0),
            len(10.0),
            border_none(),
        ],
    );
    let err = try_run(&ast).expect_err("must error outside fire_hooks' current_page window");
    assert!(
        err.msg.contains("page breaking"),
        "error should name the during-page-break gate: {}",
        err.msg
    );
}

#[test]
fn register_link_to_location_during_a_page_break_resolves_through_the_shared_name_table() {
    let ast = apply_all(
        "register-link-to-location",
        vec![
            str_lit("chapter1"),
            point(0.0, 0.0),
            len(10.0),
            len(10.0),
            len(10.0),
            border_none(),
        ],
    );
    let (v, interp) = eval_during_page_break(&ast).expect("must succeed inside the window");
    assert!(matches!(v, Value::Unit));
    assert_eq!(interp.annotations.len(), 1);
    assert_eq!(
        interp.annotations[0].action,
        satysfi_backend::AnnotAction::GotoName("nameddest0".to_string()),
        "the key resolves through the SAME dest_name table register-destination uses, \
         minting a stable name even though nothing has registered that destination yet"
    );
}
