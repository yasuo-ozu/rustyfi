//! End-to-end coverage for transparent type-synonym support
//! (`type name = ty`, `typecheck.rs`'s `SynonymDecl`/`expand_synonyms`):
//! real SATySFi source text run through `parse_file` ->
//! `elaborate::elaborate_program` -> `typecheck::typecheck`, mirroring
//! `tests/typecheck.rs`'s own harness.
//!
//! **Why ctor payloads, not a `let ... : ty = ..` annotation.** This port's
//! surface grammar has no expression- or let-level type-annotation syntax at
//! all (`ColonTok` only ever appears inside a `sig .. end` module signature,
//! which the typechecker doesn't consult — `cst.rs`'s `SigItem`/`SigAnnot`).
//! The one place a CST `TypeExpr` actually reaches the typechecker today is
//! a variant constructor's `of ty` payload (`typecheck::build_variant_decl`,
//! the same seam `UserTypeDecl` already uses) — so that is the vehicle these
//! tests use to prove a synonym is transparent to unification: a ctor
//! declared `of <synonym name>` must accept/reject exactly what the
//! synonym's *expansion* would.
//!
//! `color`/`inline-boxes` from the upstream `paren` example are avoided in
//! the semantic (well-typed/ill-typed) tests below, to stay independent of
//! the graphics primitives landing concurrently elsewhere in this port;
//! [`upstream_pervasives_examples_parse_and_register`] still exercises the
//! exact upstream declarations verbatim (parse + register only, no value of
//! either type is ever constructed).

use rustyfi_lang::{elaborate, primitives, typecheck, CompileError};

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

fn assert_type_error(src: &str) -> CompileError {
    match typecheck_str(src) {
        Ok(()) => panic!("expected {src:?} to be rejected by the typechecker, but it passed"),
        Err(e @ CompileError::Type(_)) => e,
        Err(other) => panic!("expected {src:?} to fail with a type error, got: {other}"),
    }
}

// ============================================================================
// Zero-param synonym (`type point = length * length`), referenced through a
// variant ctor payload — transparency into unification.
// ============================================================================

#[test]
fn zero_param_synonym_expands_in_ctor_payload() {
    assert_well_typed(
        "type point = length * length
         type mark = | Mark of point
         in
         Mark (1pt, 2pt)",
    );
}

#[test]
fn zero_param_synonym_payload_mismatch_is_rejected() {
    assert_type_error(
        "type point = length * length
         type mark = | Mark of point
         in
         Mark (1, 2)",
    );
}

// ============================================================================
// A `paren`-shaped synonym: several arrows then a trailing product with a
// parenthesized arrow inside it — same shape as upstream's own `paren`
// (`length -> length -> length -> length -> color -> inline-boxes * (length
// -> length)`), with `color`/`inline-boxes` swapped for `int`/`bool` so the
// test doesn't depend on the graphics primitives landing separately.
// ============================================================================

#[test]
fn multi_arrow_product_synonym_typechecks() {
    assert_well_typed(
        "type paren = length -> length -> length -> length -> int -> bool * (length -> length)
         type holder = | Hold of paren
         in
         Hold (fun a b c d e -> (true, fun x -> x))",
    );
}

#[test]
fn multi_arrow_product_synonym_mismatch_is_rejected() {
    assert_type_error(
        "type paren = length -> length -> length -> length -> int -> bool * (length -> length)
         type holder = | Hold of paren
         in
         Hold (fun a b c d e -> (1, fun x -> x))",
    );
}

// ============================================================================
// The exact upstream `pervasives.satyh` declarations (verbatim), to prove
// this literal syntax parses and registers — declaration only, no value of
// either type is constructed (so this is independent of whether `color`
// resolves to a real base type or a nominal placeholder).
// ============================================================================

#[test]
fn upstream_pervasives_examples_parse_and_register() {
    assert_well_typed(
        "type point = length * length
         type paren = length -> length -> length -> length -> color -> inline-boxes * (length -> length)
         in
         0",
    );
}

// ============================================================================
// Cyclic synonyms must be rejected with a clear error, not loop.
// ============================================================================

#[test]
fn mutually_cyclic_synonym_is_rejected() {
    assert_type_error(
        "type a = b
         type b = a
         in
         0",
    );
}

#[test]
fn self_referential_synonym_is_rejected() {
    assert_type_error(
        "type a = a
         in
         0",
    );
}

// ============================================================================
// Parameterised synonyms: params parse/register (declaration-side, kept
// symmetric with `UserTypeDecl`), but no surface syntax exists to
// *instantiate* one at a reference site (`cst::ast::TypeAtom` has no applied
// type-constructor form), so a reference is always a zero-arg one and a
// nonzero-param synonym reports a clean arity error rather than silently
// misbehaving.
// ============================================================================

#[test]
fn parameterized_synonym_declares_without_error_when_unused() {
    assert_well_typed(
        "type 'a box = 'a * 'a
         in
         1 + 2",
    );
}

#[test]
fn parameterized_synonym_reference_reports_arity_mismatch() {
    assert_type_error(
        "type 'a box = 'a * 'a
         type wrap = | Wrap of box
         in
         Wrap (1, 2)",
    );
}
