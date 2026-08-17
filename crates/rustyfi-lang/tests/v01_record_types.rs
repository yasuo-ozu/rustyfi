//! G2 end-to-end coverage: closed record TYPES in V0_1 type position
//! (`(| l1 : ty1, … |)`, `cst_v1::ast::TypeAtom::Record`) driven all the way
//! through `compile_document_v1` — proving the v1 grammar+lowering
//! increment funnels into the EXISTING checker machinery
//! (`cst::ast::TypeAtom::Record` -> `typecheck.rs`'s `Row::Cons` chain ->
//! `MonoType::Record` -> `unify.rs`'s `unify_row`), exactly as
//! `crates/rustyfi-lang/tests/record_types.rs` already proves for the
//! frozen 0.0.6 surface.
//!
//! Two harness shapes, both reproduced locally (no shared test-support
//! library target exists in this crate — same rationale every other `v01_*`
//! integration test file already documents):
//!  - [`eval_v01`] — the "real value" bar (mirrors `v01_stdlib.rs`'s
//!    `compile_v01_via_loader_with_metrics`'s tail, minus the loader: the
//!    two sources here need no `@require:` resolution, so `LoadedFile`/
//!    `LoadedCst` are built directly in memory, exactly like
//!    `v01_slice1.rs`/`v01_sealing.rs` already do) — used by the
//!    well-typed test to prove a record TYPE'd variant payload survives
//!    elaborate -> typecheck -> eval to a real `Value`.
//!  - [`assert_accepts`]/[`assert_type_error`] — `v01_sealing.rs`'s own
//!    `NotADocument`-trick harness (see that file's doc comment for why),
//!    reused verbatim for the type-error and sig-subsumption tests, which
//!    only need to observe where type-checking lands, not a real `Value`.

use rustyfi_backend::{FontKey, FontMetrics, Length};
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck, v1::lower, CompileError};
use rustyfi_loader::{LoadedCst, LoadedFile};
use rustyfi_syntax::parse_file_v1;
use rustyfi_syntax::RustyfiVersion;

/// A real (ASCII-only) `FontMetrics` stub — mirrors `v01_slice1.rs`'s/
/// `v01_stdlib.rs`'s own `Mono`; none of these fixtures actually render
/// text, but `compile_document_v1`'s signature needs a concrete
/// `&dyn FontMetrics`.
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

fn loaded(lib_src: &str, doc_src: &str) -> Vec<LoadedFile> {
    vec![
        LoadedFile {
            path: std::path::PathBuf::from("lib.satyh"),
            cst: LoadedCst::V0_1(parse_file_v1(lib_src).unwrap_or_else(|e| panic!("lib parse failed: {e}"))),
            origin: Default::default(),
            version: RustyfiVersion::V0_1,
        },
        LoadedFile {
            path: std::path::PathBuf::from("doc.saty"),
            cst: LoadedCst::V0_1(parse_file_v1(doc_src).unwrap_or_else(|e| panic!("doc parse failed: {e}"))),
            origin: Default::default(),
            version: RustyfiVersion::V0_1,
        },
    ]
}

fn run(lib_src: &str, doc_src: &str) -> Result<(), CompileError> {
    let files = loaded(lib_src, doc_src);
    rustyfi_lang::compile_document_v1(&files, &Mono).map(|_| ())
}

/// Type-checking accepted `doc_src` against `lib_src` — see this file's doc
/// comment for the `NotADocument` trick (`v01_sealing.rs`'s own harness,
/// reused verbatim).
fn assert_accepts(lib_src: &str, doc_src: &str) {
    match run(lib_src, doc_src) {
        Ok(()) | Err(CompileError::NotADocument(_)) => {}
        Err(other) => panic!("expected type-checking to accept, got: {other}"),
    }
}

/// Type-checking rejected `doc_src` against `lib_src`; returns the message
/// for content assertions.
fn assert_type_error(lib_src: &str, doc_src: &str) -> String {
    match run(lib_src, doc_src) {
        Err(CompileError::Type(e)) => e.to_string(),
        Err(other) => panic!("expected a Type error, got: {other}"),
        Ok(()) => panic!("expected type-checking to reject, but compilation succeeded"),
    }
}

/// The "real value" bar: elaborate -> typecheck -> eval directly to a
/// `Value`, bypassing `compile_document_v1`'s document/eval-fixpoint
/// requirement (mirrors `v01_stdlib.rs`'s `compile_v01_via_loader_with_
/// metrics`'s tail, minus the loader — these two in-memory sources need no
/// `@require:` resolution).
fn eval_v01(lib_src: &str, doc_src: &str) -> Result<Value, String> {
    let lib_file = parse_file_v1(lib_src).unwrap_or_else(|e| panic!("lib parse failed: {e}"));
    let prelude = lower::lower_file_v1(&lib_file).map_err(|e| format!("lower lib: {e}"))?;

    let doc_file = parse_file_v1(doc_src).unwrap_or_else(|e| panic!("doc parse failed: {e}"));
    let body = lower::lower_document_v1(&doc_file).map_err(|e| format!("lower doc: {e}"))?;
    let eoi = match &doc_file {
        rustyfi_syntax::cst_v1::FileV1::Document { eoi, .. } => eoi.clone(),
        _ => return Err("doc_src must parse as a V0_1 document".to_string()),
    };
    let file = rustyfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: Some(rustyfi_syntax::leaf::KwIn(rustyfi_syntax::Span::default())),
        body: Some(body),
        eoi,
    };

    let env = primitives::base_env_with_version(RustyfiVersion::V0_1);
    let scope = elaborate::Scope::new(env.names());
    let elaborated = elaborate::elaborate_program(&file, &scope).map_err(|e| format!("elaborate: {e}"))?;
    typecheck::typecheck_with_version(&elaborated, RustyfiVersion::V0_1).map_err(|e| format!("typecheck: {e}"))?;
    let mut interp = eval::Interp::new(&Mono);
    interp.eval(&env, &elaborated.body).map_err(|e| format!("eval: {e}"))
}

fn as_int(v: Value) -> i64 {
    match v {
        Value::Int(n) => n,
        other => panic!("expected an int, got {other:?}"),
    }
}

// ============================================================================
// Well-typed: a closed record TYPE as a variant ctor's `of` payload,
// constructed as a record VALUE, projected via field access, evaluated to
// the expected `Value` — v1 record values + `AccessField` already work
// (this test only pins that the v1 record TYPE funnels into the same
// `MonoType::Record` unification that constrains the ctor argument).
// ============================================================================

const LIB_SRC: &str = "\
module M = struct
type t = Mk of (| x : int |)
val get-x m =
  match m with
  | Mk r -> r#x
  end
end
";

#[test]
fn record_type_ctor_payload_lowers_and_evaluates() {
    let v = eval_v01(LIB_SRC, "M.get-x (Mk (| x = 42 |))").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(as_int(v), 42);
}

// ============================================================================
// Type errors: `unify_row`'s closed×closed `MissingLabel` reachable from the
// V0_1 surface (extra label, missing label) — both directions of the row
// mismatch the paused rows track's `?'r` tail would otherwise permit.
// ============================================================================

#[test]
fn record_type_ctor_payload_with_an_extra_label_is_a_type_error() {
    assert_type_error(LIB_SRC, "M.get-x (Mk (| x = 1, y = 2 |))");
}

#[test]
fn record_type_ctor_payload_missing_a_label_is_a_type_error() {
    assert_type_error(LIB_SRC, "M.get-x (Mk (| |))");
}

// ============================================================================
// Sig subsumption: a sealed module declaring `val f : (| a : int |) -> int`
// — pins the `v1/module_check.rs` path (its reuse of `typecheck::
// lower_type_expr`, `typecheck.rs:589-594`) for a record-typed `val` sig.
// ============================================================================

const SEALED_LIB_ACCEPT: &str = "\
module M :> sig
val f : (| a : int |) -> int
end = struct
val f r = r#a
end
";

#[test]
fn record_typed_sig_val_matching_body_is_accepted() {
    assert_accepts(SEALED_LIB_ACCEPT, "M.f (| a = 1 |)");
}

const SEALED_LIB_VIOLATING: &str = "\
module M :> sig
val f : (| a : int |) -> int
end = struct
val f r = r#a == 0
end
";

#[test]
fn record_typed_sig_val_violating_body_is_a_type_error() {
    assert_type_error(SEALED_LIB_VIOLATING, "M.f (| a = 1 |)");
}
