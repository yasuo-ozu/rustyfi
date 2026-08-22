//! End-to-end coverage for record TYPES in type-expression position
//! (`cst.rs`'s `TypeAtom::Record`, `(| l : ty; … |)`), lowering to
//! `MonoType::Record` (a *closed* row), not `Kind::Record` (a label-only
//! lower bound, which the same field-list shape also parses as at
//! `constraint 'a :: (|…|)`).
//!
//! Mirrors `tests/type_synonym.rs`: this grammar has no expression/let-
//! level type-annotation syntax, so the vehicle for driving a `TypeExpr`
//! through `elaborate_program` -> `typecheck` is a variant ctor's `of ty`
//! payload (directly, or via a `type` synonym). The `val .. : ty` sig-
//! annotation shape is covered at the parse level only, since
//! `typecheck.rs` doesn't consult `sig .. end` blocks yet.

use rustyfi_lang::{elaborate, primitives, typecheck, CompileError};
use rustyfi_syntax::cst::ast::{TypeApp, TypeAtom, TypeExpr, TypeProd};
use rustyfi_syntax::cst::{SigItem, TopBinding, TypeDeclBody};

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

/// Pulls the record type's field names out of a `TypeExpr`; panics if the
/// shape isn't a bare `(| … |)` atom (guards the CST-shape assertions
/// below against a silent grammar regression).
fn record_field_names(ty: &TypeExpr) -> Vec<String> {
    let TypeExpr::Atom(TypeProd { first, rest }) = ty else {
        panic!("expected a bare TypeProd, got {ty:?}");
    };
    assert!(rest.is_empty(), "expected no `*` continuation, got {ty:?}");
    let TypeApp {
        head: TypeAtom::Record { fields, .. },
        ..
    } = first
    else {
        panic!("expected TypeAtom::Record, got {first:?}");
    };
    fields.iter().map(|f| f.name.name.clone()).collect()
}

// Grammar: `(| l : ty; … |)` parses as a TYPE (a `type` synonym body), the
// `tabularx.satyh`-shaped declaration.

#[test]
fn record_type_synonym_parses_with_expected_fields() {
    let file =
        rustyfi_syntax::parse_file("type cell-record = (| left : bool; right : bool |)\nin 0")
            .expect("parse failed");
    let TopBinding::Type(decl) = &file.prelude[0] else {
        panic!("expected a `type` declaration, got {:?}", file.prelude[0]);
    };
    assert_eq!(decl.name.name, "cell-record");
    let TypeDeclBody::Synonym(ty) = &decl.body else {
        panic!("expected TypeDeclBody::Synonym, got {:?}", decl.body);
    };
    assert_eq!(record_field_names(ty), vec!["left", "right"]);
}

/// The `val .. : ty` sig-annotation shape — parse-level only.
#[test]
fn record_type_in_sig_val_annotation_parses_with_expected_fields() {
    let file = rustyfi_syntax::parse_file(
        "module M : sig\n\
         val cell-of : (| left : bool; right : bool |)\n\
         end = struct\n\
         let cell-of = 0\n\
         end",
    )
    .expect("parse failed");
    let TopBinding::Module { sig: Some(sig), .. } = &file.prelude[0] else {
        panic!(
            "expected `module .. : sig .. end`, got {:?}",
            file.prelude[0]
        );
    };
    let SigItem::Val { name, ty, .. } = &sig.items[0] else {
        panic!("expected SigItem::Val, got {:?}", sig.items[0]);
    };
    assert_eq!(name.name, "cell-of");
    assert_eq!(record_field_names(ty), vec!["left", "right"]);
}

/// Spelled directly as a ctor's `of ty` payload (no synonym indirection)
/// — proves the grammar works at every `TypeExpr` position, not merely a
/// synonym body.
#[test]
fn record_type_directly_as_ctor_payload_parses() {
    let file =
        rustyfi_syntax::parse_file("type cell = | Cell of (| left : bool; right : bool |)\nin 0")
            .expect("parse failed");
    let TopBinding::Type(decl) = &file.prelude[0] else {
        panic!("expected a `type` declaration, got {:?}", file.prelude[0]);
    };
    let TypeDeclBody::Variant { first, .. } = &decl.body else {
        panic!("expected TypeDeclBody::Variant, got {:?}", decl.body);
    };
    let of_ty = &first.of_ty.as_ref().expect("expected `of ty`").ty;
    assert_eq!(record_field_names(of_ty), vec!["left", "right"]);
}

// Lowering + typecheck: `MonoType::Record` unified against a real record
// value, driven through a variant ctor payload (synonym and direct).

#[test]
fn record_type_synonym_lowers_and_unifies_against_a_matching_record_value() {
    assert_well_typed(
        "type cell-record = (| left : bool; right : bool |)
         type cell = | Cell of cell-record
         in Cell (| left = true; right = false |)",
    );
}

#[test]
fn record_type_directly_as_ctor_payload_lowers_and_typechecks() {
    assert_well_typed(
        "type cell = | Cell of (| left : bool; right : bool |)
         in Cell (| left = true; right = false |)",
    );
}

#[test]
fn record_type_rejects_a_value_missing_a_field() {
    assert_type_error(
        "type cell-record = (| left : bool; right : bool |)
         type cell = | Cell of cell-record
         in Cell (| left = true |)",
    );
}

#[test]
fn record_type_rejects_a_field_with_the_wrong_type() {
    assert_type_error(
        "type cell-record = (| left : bool; right : bool |)
         type cell = | Cell of cell-record
         in Cell (| left = 1; right = false |)",
    );
}

#[test]
fn record_type_field_itself_may_be_a_function_arrow() {
    // Exercises a field type going through the full recursive grammar
    // (an arrow type nested inside a record field), not just a bare name.
    assert_well_typed(
        "type cell-record = (| left : bool; get : int -> int |)
         type cell = | Cell of cell-record
         in Cell (| left = true; get = (fun x -> x) |)",
    );
}
