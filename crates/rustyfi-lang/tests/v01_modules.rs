//! Sub-slice 2a's qualified-scope integration test (spec §4.3 item 3): once
//! `v1/lower.rs::lower_file_v1` wraps a library's binds in a real
//! `cst::TopBinding::Module` instead of splicing them flat, a V0_1
//! document can *only* reach a dependency library's bindings qualified
//! (`Mod.x`) or via `let open Mod in` — never bare. This is exercised at
//! the `elaborate::elaborate_program` layer directly (not the full
//! typecheck/eval pipeline): elaboration is exactly where `Scope` name
//! resolution against the module's qualified aliases happens
//! (`elaborate.rs`'s `walk_bindings` `TopBinding::Module` arm), and none of
//! these probes need to produce a well-typed `Document` value.

use rustyfi_backend::{FontKey, FontMetrics, Length};
use rustyfi_lang::elaborate;
use rustyfi_lang::primitives;
use rustyfi_lang::v1::lower;
use rustyfi_loader::{LoadedCst, LoadedFile};
use rustyfi_syntax::cst;
use rustyfi_syntax::leaf::KwIn;
use rustyfi_syntax::{parse_file_v1, RustyfiVersion, Span};

/// Elaborate one dependency library source (a `module … = struct … end`
/// file) plus one document-body source, exactly the way
/// `compile_document_v1_with_trials` assembles its synthetic `cst::File`
/// (`lib.rs:165-195`) — reproduced locally (no `rustyfi-cli` library target
/// to import it from, same rationale `v01_slice1.rs` already documents).
fn elaborate_with_lib<'s>(
    store: &'s rustyfi_lang::symbol::SymbolStore,
    lib_src: &str,
    doc_src: &str,
) -> Result<elaborate::Program<'s>, elaborate::ElabError> {
    let lib_file = parse_file_v1(lib_src).unwrap_or_else(|e| panic!("lib parse failed: {e}"));
    let prelude = lower::lower_file_v1(&lib_file).unwrap_or_else(|e| panic!("lower_file_v1: {e}"));

    let doc_file = parse_file_v1(doc_src).unwrap_or_else(|e| panic!("doc parse failed: {e}"));
    let body =
        lower::lower_document_v1(&doc_file).unwrap_or_else(|e| panic!("lower_document_v1: {e}"));
    let eoi = match &doc_file {
        rustyfi_syntax::cst_v1::FileV1::Document { eoi, .. } => eoi.clone(),
        _ => unreachable!("doc_src must parse as a FileV1::Document"),
    };

    let file = cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: Some(KwIn(Span::default())),
        body: Some(body),
        eoi,
    };

    let env0 = primitives::base_env_with_version(RustyfiVersion::V0_1);
    let scope = elaborate::Scope::new(store, env0.names());
    elaborate::elaborate_program(&file, &scope)
}

const LIB_SRC: &str = "\
module V01Mini = struct
val document x = x
module N = struct
val y = 2
end
end
";

/// (a) `V01Mini.document …` resolves — the qualified alias `export_alias`
/// installs (`elaborate.rs:343-368`).
#[test]
fn qualified_access_resolves() {
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let result = elaborate_with_lib(&store, LIB_SRC, "V01Mini.document 1");
    assert!(
        result.is_ok(),
        "expected V01Mini.document to resolve, got {:?}",
        result.err()
    );
}

/// (b) THE key erasure-is-gone guard: bare `document` in the entry now
/// FAILS with an unbound-variable error, because Sub-slice 2a's
/// `lower_file_v1` no longer splices the library's binds flat/unqualified
/// — only the qualified `V01Mini.*` aliases are in scope after the module
/// closes (`elaborate.rs:349-350`).
#[test]
fn bare_access_fails_with_unbound_variable() {
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let err = elaborate_with_lib(&store, LIB_SRC, "document 1").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("unbound variable") && msg.contains("document"),
        "expected an 'unbound variable' error naming `document`, got: {msg}"
    );
}

/// (c) `let open V01Mini in document …` resolves — `open` re-exposes every
/// `V01Mini.*` alias back to its bare suffix (`elaborate.rs:607-619`,
/// `TopBinding::Open`'s `Expr::OpenIn` analogue).
#[test]
fn let_open_reexposes_bare_access() {
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let result = elaborate_with_lib(&store, LIB_SRC, "let open V01Mini in document 1");
    assert!(
        result.is_ok(),
        "expected `let open V01Mini in document` to resolve, got {:?}",
        result.err()
    );
}

/// (d) a nested `M.N.y` reference resolves — `qualify_key` is recursive by
/// construction (`elaborate.rs:284-288`), so a doubly-nested module's
/// binding is reachable as a two-segment qualified path.
#[test]
fn nested_module_qualified_access_resolves() {
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let result = elaborate_with_lib(&store, LIB_SRC, "V01Mini.N.y");
    assert!(
        result.is_ok(),
        "expected V01Mini.N.y to resolve, got {:?}",
        result.err()
    );
}

/// Negative control for (d): the *unqualified* inner name is not visible
/// either, even from inside the outer module's own scope after both `end`s
/// close — same flat-leak-is-gone guarantee as (b), one level deeper.
#[test]
fn nested_module_bare_inner_name_is_unbound() {
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let err = elaborate_with_lib(&store, LIB_SRC, "y").unwrap_err();
    assert!(err.to_string().contains("unbound variable"), "{err}");
}

// ---- Sub-slice 2b: full value/type `Bind` arms -----------------------------

const LIB_SRC_2B: &str = "\
module M = struct
val rec sum-list lst =
  match lst with
  | []      -> 0
  | x :: xs -> x + sum-list xs
  end
val (+++) a b = a + b
val mutable c <- 17
type t = int
end
";

/// `M.sum-list` (a `val rec` binding) resolves qualified.
#[test]
fn qualified_val_rec_resolves() {
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let result = elaborate_with_lib(&store, LIB_SRC_2B, "M.sum-list [1, 2, 3]");
    assert!(
        result.is_ok(),
        "expected M.sum-list to resolve, got {:?}",
        result.err()
    );
}

/// The bare (unqualified) name is unbound without `open` — same qualified-
/// only-export guarantee `val rec` inherits from ordinary module binds.
#[test]
fn bare_val_rec_is_unbound_without_open() {
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let err = elaborate_with_lib(&store, LIB_SRC_2B, "sum-list [1, 2, 3]").unwrap_err();
    assert!(err.to_string().contains("unbound variable"), "{err}");
}

/// `val mutable c <- 17` exports the qualified alias `M.c`, and `!M.c`
/// elaborates (the `Binding::LetMutable` + qualified-alias path,
/// `elaborate.rs:533-545,351-368`).
#[test]
fn val_mutable_exports_qualified_alias() {
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let result = elaborate_with_lib(&store, LIB_SRC_2B, "!M.c");
    assert!(
        result.is_ok(),
        "expected !M.c to resolve, got {:?}",
        result.err()
    );
}

/// A library `val (+++)` is usable infix after `let open M in` (binop *use*
/// sites need no new syntax — resolution is by operator text).
#[test]
fn val_op_named_binding_usable_infix_after_open() {
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let result = elaborate_with_lib(&store, LIB_SRC_2B, "let open M in 1 +++ 2");
    assert!(
        result.is_ok(),
        "expected `1 +++ 2` after `let open M in` to resolve, got {:?}",
        result.err()
    );
}

/// `type` binds surface into `Program::synonym_decls` with a qualified
/// (`"M.t"`-format) name (§4's pre-qualification decision).
#[test]
fn type_binds_surface_with_qualified_names() {
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let program =
        elaborate_with_lib(&store, LIB_SRC_2B, "1").unwrap_or_else(|e| panic!("elaborate: {e}"));
    assert!(
        program.type_decls.is_empty(),
        "`t` is a synonym, not a variant"
    );
    assert_eq!(program.synonym_decls.len(), 1);
    assert_eq!(program.synonym_decls[0].name, "M.t");
}

// ---- Sub-slice 2c: the placeholder-lowering pipeline-level guard -----------

/// A real (if crude) `FontMetrics` stub — never actually exercised by this
/// test (the `:>` `LowerError` fires before `compile_document_v1` ever
/// reaches elaboration/typecheck/eval), but the function signature still
/// needs a concrete `&dyn FontMetrics`, same stub shape as
/// `v01_slice1.rs`'s `Mono`.
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

/// §5.4, repurposed by Sub-slice 2d-1 (`…/tmp/slice2d-sealing.md` §4.3-I):
/// a dependency library carrying a `:>` signature ascription over a
/// `val`-only surface now COMPILES CLEAN through `compile_document_v1`'s
/// REAL load path (the same `LoadedFile`/`LoadedCst`-gate-bypass shape
/// `v01_slice1.rs` uses) — ascription is enforced (`v1::module_check::
/// check_program`), not merely parsed-and-erased. The exhaustive
/// accept/reject sealing test suite lives in `tests/v01_sealing.rs`; this
/// probe stays here purely as the pipeline-level (not just `lower_file_v1`-
/// unit-level) regression guard the original test already was.
#[test]
fn sig_annot_over_val_only_surface_compiles_through_compile_document_v1() {
    let lib_src = "module M :> sig val x : int end = struct\nval x = 1\nend";
    let doc_src = "M.x";
    let files = vec![
        LoadedFile {
            path: std::path::PathBuf::from("lib.satyh"),
            cst: LoadedCst::V0_1(
                parse_file_v1(lib_src).unwrap_or_else(|e| panic!("lib parse failed: {e}")),
            ),
            origin: Default::default(),
            version: RustyfiVersion::V0_1,
        },
        LoadedFile {
            path: std::path::PathBuf::from("doc.saty"),
            cst: LoadedCst::V0_1(
                parse_file_v1(doc_src).unwrap_or_else(|e| panic!("doc parse failed: {e}")),
            ),
            origin: Default::default(),
            version: RustyfiVersion::V0_1,
        },
    ];
    let mono = Mono;
    match rustyfi_lang::compile_document_v1(&files, &mono) {
        Ok(_) | Err(rustyfi_lang::CompileError::NotADocument(_)) => {}
        Err(other) => panic!("expected type-checking to accept a val-only seal, got: {other}"),
    }
}
