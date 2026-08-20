//! group E3 (sliver): `text-info` — the
//! `get-initial-text-info`/`deepen-indent`/`break` pure prims. Harness copied
//! from `context_box.rs` (typecheck half via `parse_file` ->
//! `elaborate_program` -> `typecheck`; eval half via direct `Ast` apply
//! chains through `eval::Interp` + `primitives::base_env()`).
//!
//! SCOPING: `stringify-inline`/`stringify-block` and the `.satyh-text`/
//! `--text-mode` HTML backend are deliberately OUT of scope for this PDF
//! port — see `primitives.rs`'s section comment on the three prims tested
//! here.

use rustyfi_backend::{FontKey, FontMetrics, Length};
use rustyfi_lang::ast::Ast;
use rustyfi_lang::eval;
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, prim_types, primitives, typecheck, CompileError};
use rustyfi_syntax::Span;

// ============================================================================
// Typecheck half
// ============================================================================

fn typecheck_str(src: &str) -> Result<(), CompileError> {
    let file = rustyfi_syntax::parse_file(src)?;
    let env = primitives::base_env();
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = elaborate::Scope::new(&store, env.names());
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
fn break_of_deepen_indent_of_get_initial_text_info_typechecks_to_string() {
    assert_well_typed("break (deepen-indent 2 (get-initial-text-info ()))");
}

#[test]
fn deepen_indent_rejects_a_pdf_context_argument() {
    // `text-info` is a distinct base type from `context` — a real PDF
    // `context` value must NOT typecheck where a `text-info` is expected.
    assert_type_error(
        "let-inline ctx \\math m = inline-nil
         in
         deepen-indent 2 (get-initial-context 100pt (command \\math))",
    );
}

// ============================================================================
// Eval half — direct `Ast` apply chains (no parser), mirroring
// `context_box.rs`'s style.
// ============================================================================

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

fn run(ast: &Ast) -> Value {
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    interp.eval(&env, ast).expect("evaluation should succeed")
}

fn assert_str_eq(v: Value, expect: &str) {
    match v {
        Value::Str(s) => assert_eq!(s, expect),
        other => panic!("expected a string, got {other:?}"),
    }
}

fn initial_text_info() -> Ast {
    app1(var("get-initial-text-info"), Ast::Unit)
}

fn deepen(i: i64, tinfo: Ast) -> Ast {
    app2("deepen-indent", Ast::Int(i), tinfo)
}

#[test]
fn initial_indent_is_zero_and_break_is_bare_newline() {
    let ast = app1(var("break"), initial_text_info());
    assert_str_eq(run(&ast), "\n");
}

#[test]
fn deepen_indent_accumulates() {
    let ast = app1(var("break"), deepen(3, deepen(2, initial_text_info())));
    assert_str_eq(run(&ast), "\n     ");
}

#[test]
fn negative_increment_is_clamped_per_call() {
    // upstream `max i 0` clamps the INCREMENT, not the running total — so
    // `deepen-indent (-2)` after `deepen-indent 4` adds 0, leaving indent at
    // 4, not 2.
    let ast = app1(var("break"), deepen(-2, deepen(4, initial_text_info())));
    assert_str_eq(run(&ast), "\n    ");
}

// ============================================================================
// Registration coverage: every new group-E primitive resolves in base_env
// AND has a registered type (pattern: `prims_phase4.rs`'s
// `every_new_primitive_resolves_in_base_env`/`_has_a_registered_type`, but
// scoped to all 7 group-E names across E1/E2/E3).
// ============================================================================

const GROUP_E_NAMES: &[&str] = &[
    // E1
    "probe-cross-reference",
    // E2
    "get-dominant-wide-script",
    "get-dominant-narrow-script",
    "get-language",
    // E3
    "get-initial-text-info",
    "deepen-indent",
    "break",
];

#[test]
fn every_group_e_primitive_resolves_in_base_env() {
    let env = primitives::base_env();
    for name in GROUP_E_NAMES {
        assert!(
            env.lookup(name).is_some(),
            "primitive `{name}` is not bound in base_env()"
        );
    }
}

#[test]
fn every_group_e_primitive_has_a_registered_type() {
    for name in GROUP_E_NAMES {
        assert!(
            prim_types::primitive_type(name).is_some(),
            "primitive `{name}` has no registered type"
        );
    }
}
