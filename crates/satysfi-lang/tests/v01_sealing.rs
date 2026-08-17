//! Sub-slice 2d-1 (`…/tmp/slice2d-sealing.md` §4.5): the end-to-end sealing
//! test suite, driven through the REAL public pipeline
//! (`satysfi_lang::compile_document_v1`, the `LoadedFile`/`LoadedCst`
//! gate-bypass shape `v01_modules.rs`/`v01_slice1.rs` already use) rather
//! than `v1::module_check::check_program` directly — that entry point is
//! deliberately `pub(crate)` (spec §4.3-D), so an external integration test
//! can only reach it through the crate's public surface.
//!
//! **The `NotADocument` trick.** `compile_document_v1` runs the FULL
//! pipeline (elaborate → `check_program` → compile → eval), and eval
//! requires the entry expression to actually produce a `Value::Document` —
//! but every ACCEPT-case fixture below is a plain expression (`M.x + 1`,
//! `!M.r`, …), not a real SATySFi document envelope (building one needs a
//! whole `page-break`-based `document` helper the sealed-`val`-only 2d-1
//! surface can't even declare, §3.2). Since type-checking happens BEFORE
//! evaluation, a type error always surfaces as `CompileError::Type`
//! regardless of what the body would have evaluated to; a program that
//! type-checks but isn't a document surfaces as `CompileError::
//! NotADocument` instead — a value we can only ever reach if `check_program`
//! already accepted the program. `assert_accepts` treats both `Ok` and
//! `NotADocument` as "type-checking accepted"; `assert_type_error` demands
//! exactly `CompileError::Type` and returns its message for content checks.

use satysfi_backend::{FontKey, FontMetrics, Length};
use satysfi_lang::CompileError;
use satysfi_loader::{LoadedCst, LoadedFile};
use satysfi_syntax::parse_file_v1;

/// A real (if crude) `FontMetrics` stub — never actually exercised (every
/// fixture below either fails type-checking or fails at the `NotADocument`
/// stage, before any text is ever measured), but `compile_document_v1`'s
/// signature still needs a concrete `&dyn FontMetrics` — same stub shape as
/// `v01_modules.rs`/`v01_slice1.rs`.
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

fn run(lib_src: &str, doc_src: &str) -> Result<(), CompileError> {
    let files = vec![
        LoadedFile {
            path: std::path::PathBuf::from("lib.satyh"),
            cst: LoadedCst::V0_1(parse_file_v1(lib_src).unwrap_or_else(|e| panic!("lib parse failed: {e}"))),
        },
        LoadedFile {
            path: std::path::PathBuf::from("doc.saty"),
            cst: LoadedCst::V0_1(parse_file_v1(doc_src).unwrap_or_else(|e| panic!("doc parse failed: {e}"))),
        },
    ];
    let mono = Mono;
    satysfi_lang::compile_document_v1(&files, &mono).map(|_| ())
}

/// Type-checking accepted `doc_src` against `lib_src` — see this file's doc
/// comment for the `NotADocument` trick.
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

// ============================================================================
// Positive (§4.5)
// ============================================================================

/// T1 accept: the spec's §4.1 worked example — width + depth both satisfied,
/// `secret` (undeclared) is silently hidden.
#[test]
fn t1_width_and_depth_accept() {
    let lib = "\
module M :> sig
  val x : int
  val f 'a : 'a -> 'a
end = struct
  val x = 1
  val f y = y
  val secret = `hush`
end
";
    assert_accepts(lib, "M.x + M.f 1");
}

/// T1's exact minimal spec fixture.
#[test]
fn t1_minimal_spec_fixture_accepts() {
    let lib = "module M :> sig val x : int end = struct val x = 1 end";
    assert_accepts(lib, "M.x");
}

/// T2 polymorphic accept: `val f 'a : 'a -> 'a` over `val f y = y` — usable
/// at multiple instances downstream (proving the COMMITTED scheme is still
/// genuinely polymorphic, not accidentally monomorphized).
#[test]
fn t2_polymorphic_accepts() {
    let lib = "module M :> sig val f 'a : 'a -> 'a end = struct val f y = y end";
    assert_accepts(lib, "(M.f 1, M.f true)");
}

/// T3 specialize accept: `val f : int -> int` over the polymorphic
/// `val f y = y` — the declared (narrower) type is what's committed.
#[test]
fn t3_specialize_accepts() {
    let lib = "module M :> sig val f : int -> int end = struct val f y = y end";
    assert_accepts(lib, "M.f 1");
}

/// T4 THE SEALING FINGERPRINT: same module as T3, but `M.f true` — REJECTED,
/// because the committed scheme is the declared `int -> int`, not the
/// inferred `'a -> 'a`. This is the one test that distinguishes real
/// sealing from parse-and-ignore.
#[test]
fn t4_sealing_fingerprint_rejects_narrowed_use() {
    let lib = "module M :> sig val f : int -> int end = struct val f y = y end";
    let msg = assert_type_error(lib, "M.f true");
    assert!(!msg.is_empty(), "expected a real diagnostic");
}

/// T6 own-type reference (transparent synonym): `val f : t -> t` declared
/// over `type t = int  val f y = y + 1` — the declared side expands `t`
/// through the SAME synonym table the impl side does.
#[test]
fn t6_transparent_own_type_synonym_accepts() {
    let lib = "\
module M :> sig
  val f : t -> t
end = struct
  type t = int
  val f y = y + 1
end
";
    assert_accepts(lib, "M.f 1");
}

/// T6's variant twin: `val mk : t` declared over `type t = | A  val mk = A`
/// — nominal `\"M.t\"` on both sides.
#[test]
fn t6_own_variant_type_accepts() {
    let lib = "\
module M :> sig
  val mk : t
end = struct
  type t = | A
  val mk = A
end
";
    assert_accepts(lib, "M.mk");
}

/// T7 nested seal: a sealed module nested inside an UNSEALED one still gets
/// checked, and its declared member is reachable qualified from outside.
#[test]
fn t7_nested_seal_accepts_declared_member() {
    let lib = "\
module M = struct
  module N :> sig
    val y : int
  end = struct
    val y = 1
    val z = 2
  end
end
";
    assert_accepts(lib, "M.N.y");
}

/// T8 mutable member: `val r : ref int` declared over `val mutable r <- 0`
/// — accepts (mono vs mono). (0.1's type-application grammar is PREFIX —
/// `ref int`, not 0.0.6's postfix `int ref` — see `ast_v1::TypeApp`'s doc
/// comment.)
#[test]
fn t8_mutable_member_accepts() {
    let lib = "module M :> sig val r : ref int end = struct val mutable r <- 0 end";
    assert_accepts(lib, "!M.r");
}

/// T8's negative half: a heterogeneous downstream use is still rejected
/// through the committed `ref int` scheme. `Overwrite`'s target is a bare
/// `VarTok` in 0.1's grammar (no qualified name), so `open` first.
#[test]
fn t8_mutable_member_heterogeneous_use_rejects() {
    let lib = "module M :> sig val r : ref int end = struct val mutable r <- 0 end";
    assert_type_error(lib, "let open M in r <- true");
}

// ============================================================================
// Negative (§4.5) — message content + span
// ============================================================================

/// T10 depth mismatch: `val x : int` over `` val x = `abc` `` — message
/// names the module, the member, and both types.
#[test]
fn t10_depth_mismatch_message() {
    let lib = "module M :> sig val x : int end = struct val x = `abc` end";
    let msg = assert_type_error(lib, "1");
    assert!(msg.contains("module `M` does not match its signature"), "{msg}");
    assert!(msg.contains('x'), "{msg}");
    assert!(msg.contains("int"), "{msg}");
    assert!(msg.contains("string"), "{msg}");
}

/// T11 width missing: `val y : int` with no `y` defined.
#[test]
fn t11_width_missing_message() {
    let lib = "module M :> sig val y : int end = struct val x = 1 end";
    let msg = assert_type_error(lib, "1");
    assert!(msg.contains("never defines `y`"), "{msg}");
}

/// T5 hiding: an undeclared member used from outside the seal — precise
/// diagnostic, not the raw "unbound variable" internal-error string.
#[test]
fn t5_hidden_member_use_message() {
    let lib = "\
module M :> sig
  val x : int
end = struct
  val x = 1
  val secret = `hush`
end
";
    let msg = assert_type_error(lib, "M.secret");
    assert!(msg.contains("exists in module `M`"), "{msg}");
    assert!(msg.contains("not exported by its signature"), "{msg}");
    assert!(msg.contains("secret"), "{msg}");
    assert!(!msg.contains("internal error"), "{msg}");
}

/// T7's negative half: a sibling of a nested seal is likewise hidden.
#[test]
fn t7_nested_seal_hides_undeclared_sibling() {
    let lib = "\
module M = struct
  module N :> sig
    val y : int
  end = struct
    val y = 1
    val z = 2
  end
end
";
    let msg = assert_type_error(lib, "M.N.z");
    assert!(msg.contains("exists in module `M.N`"), "{msg}");
    assert!(msg.contains("not exported by its signature"), "{msg}");
}

/// T12 declared-more-general: `val f 'a : 'a -> 'a` declared over the
/// monomorphic `val f y = y + 1` — the implementation is less polymorphic
/// than the signature claims.
#[test]
fn t12_declared_more_general_rejects() {
    let lib = "module M :> sig val f 'a : 'a -> 'a end = struct val f y = y + 1 end";
    assert_type_error(lib, "1");
}

/// T13 escaped skolem: `val r 'a : ref (list 'a)` declared over
/// `val mutable r <- []` — the mutable cell is monomorphic (value
/// restriction), so it cannot honestly claim the declared polymorphism.
/// (Prefix type application, as T8's note explains.)
#[test]
fn t13_escaped_skolem_message() {
    let lib = "module M :> sig val r 'a : ref (list 'a) end = struct val mutable r <- [] end";
    let msg = assert_type_error(lib, "1");
    assert!(msg.contains("less polymorphic than its signature declares"), "{msg}");
}

/// T14 unbound quant tyvar: `val f : 'a -> 'a` with NO quantifier list.
#[test]
fn t14_unbound_quant_tyvar_message() {
    let lib = "module M :> sig val f : 'a -> 'a end = struct val f y = y end";
    let msg = assert_type_error(lib, "1");
    assert!(msg.contains("not bound by this"), "{msg}");
    assert!(msg.contains("quantifier list"), "{msg}");
}

/// T15 placeholder decls: every non-`Val` `Decl` arm errors naming its
/// owning sub-slice — never panics.
#[test]
fn t15_placeholder_decls_name_their_sub_slice() {
    let cases: &[(&str, &str)] = &[
        ("type t :: o", "2d-2"),
        ("type u = int", "2d-2"),
        (r"val \c : int", "2d-2"),
        ("val +p : int", "2d-2"),
        ("module N : sig end", "2d-3"),
        ("signature S = sig end", "2d-3"),
        ("include S", "2e"),
    ];
    for (decl, expect) in cases {
        let lib = format!("module M :> sig {decl} end = struct val x = 1 end");
        let msg = assert_type_error(&lib, "1");
        assert!(msg.contains(expect), "decl {decl:?}: expected {expect:?} in {msg:?}");
    }
}

/// T16 non-struct sig forms: every shape other than a bare `sig .. end`
/// literal is a precise placeholder, naming the sub-slice that owns it.
#[test]
fn t16_non_struct_sig_forms_name_their_sub_slice() {
    let cases: &[(&str, &str)] = &[
        ("module M :> S = struct val x = 1 end", "2d-3"),
        ("module M :> A.B.S = struct val x = 1 end", "2d-3"),
        ("module M :> sig end with type t = int = struct val x = 1 end", "2e"),
        ("module M :> (X : S) -> S2 = struct val x = 1 end", "2f"),
    ];
    for (lib, expect) in cases {
        let msg = assert_type_error(lib, "1");
        assert!(msg.contains(expect), "lib {lib:?}: expected {expect:?} in {msg:?}");
    }
}

// ============================================================================
// Parity/regression (§4.5)
// ============================================================================

/// T9's e2e twin: an existing, seal-free `v01_modules.rs`-style source still
/// compiles clean end-to-end through the new `check_program`-based V0_1
/// pipeline (the fine-grained verdict/warning/error parity itself is
/// pinned inside `v1::module_check`'s own unit tests, which have direct
/// access to both `check_program` and `typecheck_verbose_with_version`).
#[test]
fn t9_seal_free_v01_modules_style_source_still_compiles() {
    let lib = "\
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
    assert_accepts(lib, "M.sum-list [1, 2, 3] + !M.c");
}
