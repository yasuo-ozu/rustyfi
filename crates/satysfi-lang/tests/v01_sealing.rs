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
//!
//! Sub-slice 2d-2 (`…/tmp/slice2d2-opaque-types.md` §5) extends this suite
//! with its own U-numbered group below: opaque type sealing, transparent
//! type equality, constructor hiding, command-type decls, and `LONG_LOWER`
//! qualified type names. `assert_accepts_multi`/`assert_type_error_multi`
//! are this group's twin of `assert_accepts`/`assert_type_error`, taking
//! SEVERAL dependency libraries (needed by U10/U11's cross-module
//! `LONG_LOWER` fixtures) instead of exactly one.

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

/// [`run`]'s multi-dependency twin (Sub-slice 2d-2's U10/U11 need TWO
/// separate `module … = struct … end` library files, one referencing the
/// other's types via `LONG_LOWER`).
fn run_multi(lib_srcs: &[&str], doc_src: &str) -> Result<(), CompileError> {
    let mut files: Vec<LoadedFile> = lib_srcs
        .iter()
        .enumerate()
        .map(|(i, src)| LoadedFile {
            path: std::path::PathBuf::from(format!("lib{i}.satyh")),
            cst: LoadedCst::V0_1(parse_file_v1(src).unwrap_or_else(|e| panic!("lib parse failed: {e}"))),
        })
        .collect();
    files.push(LoadedFile {
        path: std::path::PathBuf::from("doc.saty"),
        cst: LoadedCst::V0_1(parse_file_v1(doc_src).unwrap_or_else(|e| panic!("doc parse failed: {e}"))),
    });
    let mono = Mono;
    satysfi_lang::compile_document_v1(&files, &mono).map(|_| ())
}

fn assert_accepts_multi(lib_srcs: &[&str], doc_src: &str) {
    match run_multi(lib_srcs, doc_src) {
        Ok(()) | Err(CompileError::NotADocument(_)) => {}
        Err(other) => panic!("expected type-checking to accept, got: {other}"),
    }
}

fn assert_type_error_multi(lib_srcs: &[&str], doc_src: &str) -> String {
    match run_multi(lib_srcs, doc_src) {
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

/// T15 placeholder decls: every remaining non-`Val`/non-`Type`/non-command
/// `Decl` arm errors naming its owning sub-slice — never panics. Sub-slice
/// 2d-2 retires the `type t :: o`/`type u = int`/`val \c : ..`/`val +p : ..`
/// rows this test used to pin as placeholders (§4-D of the opaque-types
/// spec: `TypeOpaque`/`Type`/`ValHorzCmd`/`ValVertCmd` are processed for
/// real now) — their NEW behavior is covered by `v1::module_check`'s own
/// U-numbered test group below instead.
#[test]
fn t15_placeholder_decls_name_their_sub_slice() {
    let cases: &[(&str, &str)] = &[
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

/// T16 non-struct sig forms. Sub-slice 2d-3 retires the two named-signature
/// placeholders: `:> S` / `:> A.B.S` now RESOLVE through the signature table
/// (`v1/surface.rs`), so an UNDEFINED name is a precise "unknown signature
/// name" error (not a "not enforced yet" placeholder). `with type` (2e) and
/// functor signatures (2f) stay their sub-slice placeholders.
#[test]
fn t16_non_struct_sig_forms_name_their_sub_slice() {
    let cases: &[(&str, &str)] = &[
        ("module M :> S = struct val x = 1 end", "unknown signature name"),
        ("module M :> A.B.S = struct val x = 1 end", "unknown signature name"),
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

// ============================================================================
// Sub-slice 2d-2 (`…/tmp/slice2d2-opaque-types.md` §5): opaque type sealing,
// transparent type equality, constructor hiding, command-type decls,
// `LONG_LOWER` qualified type names.
// ============================================================================

/// The spec's own worked example (§3): `type t :: o` opaque, a plain `val`
/// pair (`make`/`get`), and a sealed inline command (`\show`).
const WORKED_EXAMPLE: &str = "\
module M :> sig
  type t :: o
  val make : int -> t
  val get : t -> int
  val \\show : inline [t]
end = struct
  type t = | T of int
  val make n = T n
  val get x = match x with | T n -> n end
  val inline ctx \\show x = read-inline ctx (embed-string (arabic (get x)))
end
";

/// U1 opaque accept: the worked example, doc `M.get (M.make 3)` compiles
/// clean.
#[test]
fn u1_opaque_accept() {
    assert_accepts(WORKED_EXAMPLE, "M.get (M.make 3)");
}

/// U2 THE OPACITY FINGERPRINT: `M.get 3` (an `int`, not the opaque `M.t`)
/// is REJECTED, as is `(M.make 3) + 1` — the one test pair distinguishing
/// real abstraction from parse-and-ignore. Messages contain `M.t` and NOT
/// `#` (pins `strip_stamps`, = U17).
#[test]
fn u2_opacity_fingerprint() {
    let msg = assert_type_error(WORKED_EXAMPLE, "M.get 3");
    assert!(msg.contains("M.t"), "{msg}");
    assert!(!msg.contains('#'), "no stamp should leak: {msg}");

    let msg2 = assert_type_error(WORKED_EXAMPLE, "(M.make 3) + 1");
    assert!(!msg2.contains('#'), "no stamp should leak: {msg2}");
}

/// U3 inside-transparency (synonym impl): `type t :: o` over `type t = int;
/// val make n = n; val get x = x + 1` — accepts inside (`t ≡ int`
/// transparently); `M.get (M.make 1)` ✓, `M.get 5` ✗ (outside, `t` is
/// opaque).
#[test]
fn u3_inside_transparency_synonym_impl() {
    let lib = "\
module M :> sig
  type t :: o
  val make : int -> t
  val get : t -> int
end = struct
  type t = int
  val make n = n
  val get x = x + 1
end
";
    assert_accepts(lib, "M.get (M.make 1)");
    assert_type_error(lib, "M.get 5");
}

/// U5 transparent equality accept: sig `type sz = int  val width : sz` over
/// impl `type sz = int  val width = 10`; doc `M.width + 1` ✓ (concrete
/// outside).
#[test]
fn u5_transparent_equality_accept() {
    let lib = "\
module M :> sig
  type sz = int
  val width : sz
end = struct
  type sz = int
  val width = 10
end
";
    assert_accepts(lib, "M.width + 1");
}

/// U6 transparent mismatch: sig `type sz = string` over impl `type sz =
/// int` → a precise message naming both sides.
#[test]
fn u6_transparent_mismatch() {
    let lib = "\
module M :> sig
  type sz = string
end = struct
  type sz = int
  val x = 1
end
";
    let msg = assert_type_error(lib, "1");
    assert!(msg.contains("string"), "{msg}");
    assert!(msg.contains("int"), "{msg}");
    assert!(msg.contains("does not match its signature"), "{msg}");
}

/// U7 kind/arity: `type t :: o -> o` over `type t 'a = list 'a` ✓ (with
/// `val wrap 'a : 'a -> t 'a`); `type t :: o` over the same impl → arity
/// error; `type t :: nat` → unsupported-kind error; opaque decl with no
/// impl type → width error.
#[test]
fn u7_kind_and_arity_checks() {
    let lib_ok = "\
module M :> sig
  type t :: o -> o
  val wrap 'a : 'a -> t 'a
end = struct
  type t 'a = list 'a
  val wrap x = [x]
end
";
    assert_accepts(lib_ok, "M.wrap 1");

    let lib_arity = "\
module M :> sig
  type t :: o
  val wrap : int -> t
end = struct
  type t 'a = list 'a
  val wrap x = [x]
end
";
    let msg = assert_type_error(lib_arity, "1");
    assert!(msg.contains("arity"), "{msg}");

    let lib_kind = "\
module M :> sig
  type t :: nat
end = struct
  type t = int
  val x = 1
end
";
    let msg = assert_type_error(lib_kind, "1");
    assert!(msg.contains("unsupported kind"), "{msg}");

    let lib_width = "\
module M :> sig
  type t :: o
end = struct
  val x = 1
end
";
    let msg = assert_type_error(lib_width, "1");
    assert!(msg.contains("never defines it"), "{msg}");
}

/// U8 command decls: the worked example's `\show` seals cleanly; a depth
/// mismatch, a wrong command-type KIND (`block` declared over an `inline`
/// impl), and a plain (non-command) declared type over a command impl all
/// reject; `val +p : block [inline-text]` over a `val block` impl accepts.
#[test]
fn u8_command_decls() {
    assert_accepts(WORKED_EXAMPLE, "1");

    // depth mismatch: `val \show : inline [int]` (wrong element type —
    // the impl passes it an `M.t`-typed argument).
    let lib_depth_mismatch = "\
module M :> sig
  type t :: o
  val make : int -> t
  val \\show : inline [int]
end = struct
  type t = | T of int
  val make n = T n
  val get x = match x with | T n -> n end
  val inline ctx \\show x = read-inline ctx (embed-string (arabic (get x)))
end
";
    let msg = assert_type_error(lib_depth_mismatch, "1");
    assert!(msg.contains("\\show"), "{msg}");

    let lib_wrong_kind = "\
module M :> sig
  type t :: o
  val make : int -> t
  val \\show : block [t]
end = struct
  type t = | T of int
  val make n = T n
  val inline ctx \\show x = read-inline ctx (embed-string `x`)
end
";
    // `\show` is `\`-sigiled, hence always `ValHorzCmd` regardless of the
    // TYPE spelled after `:` — declaring `block […]` for it is a shape
    // violation against the required `inline […]`/`math […]` shape (the
    // sigil, not the spelling, decides `Decl::ValHorzCmd` vs `ValVertCmd`;
    // math-package completion M1 widened the guard's accepted set to
    // `inline […]` OR `math […]`, since math commands share the `\` sigil
    // too).
    let msg = assert_type_error(lib_wrong_kind, "1");
    assert!(msg.contains("needs an `inline [...]` or `math [...]` command type"), "{msg}");

    let lib_plain_type = "\
module M :> sig
  type t :: o
  val make : int -> t
  val \\show : int
end = struct
  type t = | T of int
  val make n = T n
  val inline ctx \\show x = read-inline ctx (embed-string `x`)
end
";
    let msg = assert_type_error(lib_plain_type, "1");
    assert!(!msg.is_empty(), "{msg}");

    let lib_block_ok = "\
module M :> sig
  val +p : block [inline-text]
end = struct
  val block ctx +p it = read-block ctx '< >
end
";
    assert_accepts(lib_block_ok, "1");
}

/// U9 ctor hiding: `T 1` (expression) → `constructor T belongs to type t,
/// which module M's signature seals abstract`; `match M.make 1 with T n ->
/// n end` (pattern) → same diagnostic; wildcard `match M.make 1 with _ ->
/// 0 end` ✓ (exhaustive.rs's unknown-domain fact, zero edits).
#[test]
fn u9_ctor_hiding() {
    let msg = assert_type_error(WORKED_EXAMPLE, "T 1");
    assert!(msg.contains("constructor `T` belongs to type `t`"), "{msg}");
    assert!(msg.contains("module `M`'s signature seals abstract"), "{msg}");

    let msg = assert_type_error(WORKED_EXAMPLE, "match M.make 1 with T n -> n end");
    assert!(msg.contains("constructor `T` belongs to type `t`"), "{msg}");

    assert_accepts(WORKED_EXAMPLE, "match M.make 1 with _ -> 0 end");
}

/// U10 hidden type via `LONG_LOWER`: impl has `type u = int` the sig omits;
/// a second lib's SEALED sig `val g : M.u -> int` → "exists in module `M`
/// but is not exported"; an UNSEALED lib's bare `type s = M.u` → same
/// (phase B's general path).
#[test]
fn u10_hidden_type_via_long_lower() {
    let lib_m = "\
module M :> sig
  val f : int -> int
end = struct
  type u = int
  val f x = x + 1
end
";
    let lib_n = "\
module N :> sig
  val g : M.u -> int
end = struct
  val g x = x
end
";
    let msg = assert_type_error_multi(&[lib_m, lib_n], "1");
    assert!(msg.contains("exists in module `M`"), "{msg}");
    assert!(msg.contains("not exported by its signature"), "{msg}");

    let lib_n2 = "\
module N = struct
  type s = M.u
  val g x = x
end
";
    let msg2 = assert_type_error_multi(&[lib_m, lib_n2], "1");
    assert!(msg2.contains("exists in module `M`"), "{msg2}");
    assert!(msg2.contains("not exported by its signature"), "{msg2}");
}

/// U11 THE LEAK FIXTURE: `module N :> sig type s = M.t val get2 : s -> int
/// end = struct type s = M.t val get2 = M.get end` — `N.get2 (M.make 3)`
/// flows (stamp equality, the leak fix did NOT block the legitimate path);
/// `N.get2 3` is REJECTED (the synonym table did not pierce `M`'s seal
/// down to `int`, `M.t`'s concrete impl).
#[test]
fn u11_leak_fixture() {
    let lib_m = "\
module M :> sig
  type t :: o
  val make : int -> t
  val get : t -> int
end = struct
  type t = int
  val make n = n
  val get x = x + 1
end
";
    let lib_n = "\
module N :> sig
  type s = M.t
  val get2 : s -> int
end = struct
  type s = M.t
  val get2 = M.get
end
";
    assert_accepts_multi(&[lib_m, lib_n], "N.get2 (M.make 3)");
    let msg = assert_type_error_multi(&[lib_m, lib_n], "N.get2 3");
    assert!(!msg.is_empty(), "{msg}");
}

/// U12 sealed nested module + sibling: `module M = struct module N :> sig
/// type t :: o val mk : int -> t end = struct … end val result = M.N.mk 1
/// end` ✓ (a sibling reaches a nested module through the SAME full
/// qualified path elaboration always exports, `M.N.mk` — not a bare `N.mk`,
/// even from directly inside M's own struct); a sibling treating `M.N.mk 1`
/// as `int` ✗ (sibling is OUTSIDE the child seal).
#[test]
fn u12_sealed_nested_module_and_sibling() {
    let lib_ok = "\
module M = struct
  module N :> sig
    type t :: o
    val mk : int -> t
  end = struct
    type t = int
    val mk n = n
  end
  val result = M.N.mk 1
end
";
    assert_accepts(lib_ok, "1");

    let lib_bad = "\
module M = struct
  module N :> sig
    type t :: o
    val mk : int -> t
  end = struct
    type t = int
    val mk n = n
  end
  val result = M.N.mk 1 + 1
end
";
    let msg = assert_type_error(lib_bad, "1");
    assert!(!msg.is_empty(), "{msg}");
}

/// U13 self-containment: sig `val f : u -> u` where the sig declares `type
/// t :: o` (opting into type control) but NOT `u` (impl-defined) →
/// "mentions its type `u` without declaring it". A module whose sig
/// declares ZERO types at all is exempt (T6's pinned pre-2d-2 accept
/// behavior — a bare own-type reference with no sig-level type control at
/// all stays implicitly transparent).
#[test]
fn u13_self_containment() {
    let lib = "\
module M :> sig
  type t :: o
  val mk : int -> t
  val f : u -> u
end = struct
  type t = | T of int
  type u = int
  val mk n = T n
  val f x = x
end
";
    let msg = assert_type_error(lib, "1");
    assert!(msg.contains("mentions its type `u` without declaring it"), "{msg}");
}

/// U15 zero-value-member module: `module M :> sig type t :: o end = struct
/// type t = | T end` + doc `T` → hidden-ctor error (the immediate-hide
/// path — no value member exists for the trigger to ride on).
#[test]
fn u15_zero_value_member_immediate_hide() {
    let lib = "\
module M :> sig
  type t :: o
end = struct
  type t = | T
end
";
    let msg = assert_type_error(lib, "T");
    assert!(msg.contains("constructor `T` belongs to type `t`"), "{msg}");
    assert!(msg.contains("module `M`'s signature seals abstract"), "{msg}");
}

/// U19 placeholder decls: `module N : sig end` / `signature S = sig end` /
/// `include S` decls in a sig still produce their 2d-3/2e errors; a sig
/// `type t = | A` (variant body) produces the NEW 2d-3 ctor-re-export
/// placeholder.
#[test]
fn u19_placeholder_decls_still_error() {
    let cases: &[(&str, &str)] = &[
        ("module N : sig end", "2d-3"),
        ("signature S = sig end", "2d-3"),
        ("include S", "2e"),
        ("type t = | A", "2d-3"),
    ];
    for (decl, expect) in cases {
        let lib = format!("module M :> sig {decl} end = struct type t = | A  val x = 1 end");
        let msg = assert_type_error(&lib, "1");
        assert!(msg.contains(expect), "decl {decl:?}: expected {expect:?} in {msg:?}");
    }
}

/// U20 hidden command member: sig omits `\hidden`; doc `{ \M.hidden; }` →
/// "value `M.\hidden` exists in module `M` but is not exported by its
/// signature" (pins the two new command-format matchers, dual-side, like
/// 2d-1's `:1429` coupling).
#[test]
fn u20_hidden_command_member() {
    let lib = "\
module M :> sig
  val x : int
end = struct
  val x = 1
  val inline ctx \\hidden = read-inline ctx (embed-string `secret`)
end
";
    let msg = assert_type_error(lib, "{ \\M.hidden; }");
    assert!(msg.contains("exists in module `M`"), "{msg}");
    assert!(msg.contains("not exported by its signature"), "{msg}");
}

// ============================================================================
// optional-arg-rows increment 2 — the unsoundness gate's PROOF: a sealed
// signature whose `val` declares a labeled-optional-argument type
// (`?(bias : int) int -> int`, `TypeExpr::OptRowFun`) matches an increment-1
// `?(bias = …)`-taking implementation END-TO-END through the SAME `unify`-
// based subsumption path every other sealed `val` flows through — because
// `MonoType::Func` now carries the optional-arg `Row` and
// `typecheck::lower_type_expr`'s `OptRowFun` arm lowers the sig to the SAME
// closed-row `Func` that `Ast::LambdaOpt` infers for the impl. And two
// mismatches (a declared optional label ABSENT from the impl; a WRONG
// codomain type) are rejected — proving the row and the domains are really
// carried through subsumption, not silently dropped (spec §13 risk 1).
// ============================================================================

/// V1 (the sealed-sig PROOF): `val f : ?(bias : int) int -> int` over an
/// increment-1 `val f ?(bias = b) x = …` impl seals, and a plain `M.f 1`
/// (no bundle — `bias` defaults) type-checks against the committed scheme.
#[test]
fn v1_opt_arg_typed_sig_matches_opt_taking_impl() {
    let lib = "\
module M :> sig
  val f : ?(bias : int) int -> int
end = struct
  val f ?(bias = b) x = x + (match b with None -> 0 | Some v -> v end)
end
";
    assert_accepts(lib, "M.f 1");
}

/// V2 reject: the sig declares an optional `bias`, but the impl is a plain
/// `val f x = …` with no optional argument at all — the declared row
/// `Cons(bias, int, Empty)` cannot unify against the impl's empty row, so
/// sealing is rejected (the "declared optional label absent from impl" case).
#[test]
fn v2_opt_arg_sig_over_plain_impl_rejects() {
    let lib = "\
module M :> sig
  val f : ?(bias : int) int -> int
end = struct
  val f x = x + 1
end
";
    assert_type_error(lib, "M.f 1");
}

/// V3 reject: the impl DOES take the optional `bias`, but its codomain is
/// `bool`, not the declared `int` — the domain/codomain still flow through
/// subsumption alongside the row, so the `int` vs `bool` codomain clash is
/// caught (the "wrong type" case).
#[test]
fn v3_opt_arg_sig_wrong_codomain_rejects() {
    let lib = "\
module M :> sig
  val f : ?(bias : int) int -> int
end = struct
  val f ?(bias = b) x = x == (match b with None -> 0 | Some v -> v end)
end
";
    assert_type_error(lib, "M.f 1");
}

// ============================================================================
// Sub-slice 2d-3 (`…/tmp/slice2d3-module-sig-decls.md` §5): named signatures
// at ascription sites + module-alias re-export. (`Decl::Module`/
// `Decl::Signature` MEMBERS of a signature, revocation, and sealed-alias
// NARROWING are deferred — see `v1/module_check.rs`'s module doc; those
// still emit their precise placeholder/"unknown" errors, never panic.)
// ============================================================================

/// N5 named sig at an ascription: `signature S = sig … end` in a library,
/// then `module A :> S = struct … end`. Declared `x` is committed; the
/// undeclared `y` is hidden.
#[test]
fn n5_named_signature_accept_and_hide() {
    let lib = "\
module Lib = struct
  signature S = sig val x : int end
  module A :> S = struct val x = 1 val y = 2 end
end
";
    assert_accepts(lib, "Lib.A.x");
}

/// N5b: a hidden member reached through the named-sig seal errors precisely
/// (the seal really narrowed the surface — not parse-and-ignore).
#[test]
fn n5b_named_signature_hides_undeclared_member() {
    let lib = "\
module Lib = struct
  signature S = sig val x : int end
  module A :> S = struct val x = 1 val y = 2 end
end
";
    let msg = assert_type_error(lib, "Lib.A.y");
    assert!(
        msg.contains("not exported") || msg.contains("unbound"),
        "expected a hidden-member error, got: {msg}"
    );
}

/// N5c: an ascription against an UNDEFINED signature name is a precise
/// "unknown signature name" error.
#[test]
fn n5c_unknown_signature_name_rejects() {
    let lib = "\
module Lib = struct
  module A :> Nope = struct val x = 1 end
end
";
    let msg = assert_type_error(lib, "1");
    assert!(msg.contains("unknown signature name"), "got: {msg}");
}

/// N7 GENERATIVITY FINGERPRINT (spec §5): the same named signature `Store`
/// used at TWO ascription sites mints DISTINCT opaque `t` stamps, so a value
/// made by `A.mk` cannot be consumed by `B`'s view of the abstract type.
/// This is the one test distinguishing per-site stamping from
/// "elaborate-once" (which would wrongly share one stamp).
#[test]
fn n7_named_signature_generativity_fingerprint() {
    let lib = "\
module Lib = struct
  signature Store = sig
    type t :: o
    val mk : int -> t
    val get : t -> int
  end
  module A :> Store = struct  type t = int  val mk n = n  val get x = x  end
  module B :> Store = struct  type t = int  val mk n = n  val get x = x  end
end
";
    // Same-module round-trip accepts (A's own abstract t).
    assert_accepts(lib, "Lib.A.get (Lib.A.mk 1)");
    // Cross-module use rejects: A.t#i and B.t#j are distinct abstract types.
    let msg = assert_type_error(lib, "Lib.A.get (Lib.B.mk 1)");
    assert!(!msg.is_empty(), "expected a generativity mismatch");
}

/// N8 self-containment through a NAMED signature: U13's exact scenario, but
/// the sig is `signature S = …` used at an ascription. The resolved decls
/// feed the identical prescan pipeline, so 2d-2's own-type rule applies
/// unchanged — a `val` mentioning an impl-defined but sig-undeclared type
/// `u` (the sig having opted into type control by declaring `t`) is the
/// same "mentions its type … without declaring it" error.
#[test]
fn n8_named_signature_self_containment() {
    let lib = "\
module Lib = struct
  signature S = sig
    type t :: o
    val mk : int -> t
    val f : u -> u
  end
  module A :> S = struct
    type t = | T of int
    type u = int
    val mk n = T n
    val f x = x
  end
end
";
    let msg = assert_type_error(lib, "1");
    assert!(msg.contains("without declaring it"), "got: {msg}");
}

/// L1 alias re-export: `module Alias = Base` re-exports Base's public
/// surface under `Alias.*` — values usable at the target's own types, and
/// an alias member interchangeable with the target's (an alias is NOT
/// generative, upstream `UTModVar` returns the same signature).
#[test]
fn l1_module_alias_reexports_members() {
    let lib = "\
module Lib = struct
  module Base = struct
    val mk n = n + 1
    val get x = x
  end
  module Alias = Base
end
";
    assert_accepts(lib, "Lib.Alias.get (Lib.Alias.mk 3)");
    // Mixed alias/target use accepts too (same types, non-generative).
    assert_accepts(lib, "Lib.Alias.get (Lib.Base.mk 3)");
}

/// L1b path alias: `module C = A.B` re-exports a nested target's members.
#[test]
fn l1b_path_alias_reexports_nested_members() {
    let lib = "\
module Lib = struct
  module A = struct
    module B = struct val x = 7 end
  end
  module C = A.B
end
";
    assert_accepts(lib, "Lib.C.x + 1");
}

// ============================================================================
// Sub-slice 2e-1: struct-include (`include M`) — spec §5's I-numbered group.
// ============================================================================

/// I4 + I7 + I14: the spec's worked example 1 — `include Base` inside a
/// SEALED includer `P`. Combines three fingerprints in one fixture:
/// - I7 re-abstraction: `P.get (P.mk 1)` accepts (P's own fresh stamp,
///   round-trip), `P.get (Base.mk 1)` REJECTS (P's `t` is a NEW abstract
///   stamp minted at P's own ascription — the include copy re-abstracts,
///   it does not inherit Base's concrete `t`).
/// - I14 ctor visibility: `Base`'s constructor `T` is NEVER hidden by `P`'s
///   seal — it still belongs to (and is exported by) `Base` — so `T 1`
///   keeps constructing OUTSIDE `P` even though `P.t` is sealed abstract.
#[test]
fn i4_i7_i14_sealed_includer_re_abstracts_but_never_hides_the_targets_ctor() {
    let lib = "\
module Lib = struct
  module Base = struct
    type t = | T of int
    val mk n = T n
    val get x = match x with | T n -> n end
  end
  module P :> sig
    type t :: o
    val mk : int -> t
    val get : t -> int
    val extra : int
  end = struct
    include Base
    val extra = get (mk 3)
  end
end
";
    // I7 round-trip through P's own fresh stamp.
    assert_accepts(lib, "Lib.P.get (Lib.P.mk 1)");
    assert_accepts(lib, "Lib.P.extra + 1");
    // I7 re-abstraction: Base's concrete `t` does not subsume P's stamp.
    let msg = assert_type_error(lib, "Lib.P.get (Lib.Base.mk 1)");
    assert!(!msg.is_empty(), "expected a generativity mismatch, got empty message");
    // I14: Base's own ctor T is untouched by P's seal.
    assert_accepts(lib, "Lib.Base.get (Lib.Base.mk 1)");
    let doc = "match Lib.Base.mk 1 with | T n -> n end";
    assert_accepts(lib, doc);
}

/// I5: a sig omitting an INCLUDED member hides it (the standard
/// not-exported rewrite, owner = the includer `P`, not `Base`); a sig
/// declaring a member neither defined nor included is the standard width
/// error.
#[test]
fn i5_sealed_includer_hides_undeclared_included_members() {
    let lib_hide = "\
module Lib = struct
  module Base = struct
    val mk n = n
    val get x = x
  end
  module P :> sig
    val mk : int -> int
  end = struct
    include Base
  end
end
";
    // `get` was included but never declared by P's own sig: hidden, owner P.
    let msg = assert_type_error(lib_hide, "Lib.P.get 1");
    assert!(msg.contains("exists in module `Lib.P`"), "got: {msg}");
    assert!(msg.contains("not exported"), "got: {msg}");

    let lib_width = "\
module Lib = struct
  module Base = struct
    val mk n = n
  end
  module P :> sig
    val mk : int -> int
    val never-defined : int
  end = struct
    include Base
  end
end
";
    let msg = assert_type_error(lib_width, "1");
    assert!(msg.contains("never-defined"), "got: {msg}");
    assert!(msg.contains("never defines"), "got: {msg}");
}

/// I5's tripwire (spec §8 risk 2): the sealed includer's LAST value member
/// arrives VIA the include (not a direct bind) — the ctor-hide/revocation
/// trigger key ("the last value member in source order") must count the
/// SPLICED member, not the last direct bind before it. If
/// `struct_member_names_spliced` ever regressed to ignoring `Bind::Include`
/// (like the retired `struct_member_names`), the trigger would key on
/// `pre` instead of `mk` — this fixture at least proves the splice-position
/// code path runs to completion and gives the right verdict, structurally
/// exercising the exact "include is the final bind" shape.
#[test]
fn i5_tripwire_last_value_member_arrives_via_include() {
    let lib = "\
module Lib = struct
  module Base = struct
    val mk n = n
  end
  module P :> sig
    val pre : int
    val mk : int -> int
  end = struct
    val pre = 1
    include Base
  end
end
";
    assert_accepts(lib, "Lib.P.mk (Lib.P.pre) + 1");
}

/// I2 include-then-shadow (a documented deviation from upstream, which
/// REJECTS this as `ConflictInSignature`): a LATER direct bind of the same
/// name shadows the included copy — the qualified alias `P.mk` ends up
/// meaning the LAST binding in source order, exactly like the port's
/// pre-existing behavior for two direct `val mk` binds in one struct.
/// Distinguished at the TYPE level (`Base.mk` returns `bool`, the shadowing
/// bind returns `int`) so the test can tell which one actually won.
#[test]
fn i2_include_then_shadow_the_later_direct_bind_wins() {
    let lib = "\
module Lib = struct
  module Base = struct
    val mk n = true
  end
  module P = struct
    include Base
    val mk n = n + 100
  end
end
";
    // The shadowing `val mk` (int -> int) is what `P.mk` means now.
    assert_accepts(lib, "Lib.P.mk 1 + 1");
    // `Base.mk` itself is untouched (still bool-valued).
    let msg = assert_type_error(lib, "Lib.Base.mk 1 + 1");
    assert!(!msg.is_empty(), "expected a type mismatch (bool + int), got empty message");
}

/// I6 type re-export: a downstream synonym over the includer's re-exported
/// type unifies with the TARGET's own values (the copy is a synonym chain
/// `P.t = Base.t`, transparently expanding).
#[test]
fn i6_included_type_re_export_unifies_with_the_targets_values() {
    let lib = "\
module Lib = struct
  module Base = struct
    type t = int
    val mk n = n
  end
  module P = struct
    include Base
  end
end
";
    assert_accepts(lib, "Lib.P.mk 1 + Lib.Base.mk 2");
}

/// I8 nested module through include: `Base.Inner.x` is reachable as
/// `P.Inner.x` after `include Base` (recursive member-copy through nested
/// modules).
#[test]
fn i8_nested_module_reexports_through_include() {
    let lib = "\
module Lib = struct
  module Base = struct
    module Inner = struct
      val x = 42
    end
  end
  module P = struct
    include Base
  end
end
";
    assert_accepts(lib, "Lib.P.Inner.x + 1");
}

/// I9 sig-member re-export: `include Basic` (which itself defines a named
/// signature `Ord`) makes `P.Ord` resolvable at a LATER ascription site —
/// the spec's example 3 shape.
#[test]
fn i9_include_reexports_the_targets_named_signature() {
    let lib = "\
module Lib = struct
  module Basic = struct
    type ordering = | Less | Equal | Greater
    signature Ord = sig
      type t :: o
      val compare : t -> t -> ordering
    end
  end
  module Std = struct
    include Basic
    module Int = struct
      type t = int
      val compare m n =
        if m < n then Less else if m == n then Equal else Greater
    end
  end
  module M :> Std.Ord = Std.Int
end
";
    assert_accepts(lib, "Lib.M.compare 1 2");
}

/// I9b: the SAME re-export through an ALIAS instead of an include — the
/// found 2d-3 gap fix (`Alias.S` never resolved before 2e-1's
/// `register_sig_reexports`).
#[test]
fn i9b_alias_reexports_the_targets_named_signature() {
    let lib = "\
module Lib = struct
  module Basic = struct
    signature Ord = sig
      type t :: o
      val compare : t -> t -> int
    end
  end
  module A2 = Basic
  module Int = struct
    type t = int
    val compare m n = m - n
  end
  module M :> A2.Ord = Int
end
";
    assert_accepts(lib, "Lib.M.compare 1 2");
}

/// I10 mutable sharing: `include Base` shares Base's mutable CELL (not a
/// copy of its current value) — a write through `P`'s alias is observed
/// through `Base`'s own name.
#[test]
fn i10_include_shares_the_targets_mutable_cell() {
    // Two dependency files (cross-file include, §2.1 step 1's "the
    // cross-file case rides the existing outward fallback") so the
    // document can `let open P in` — `let open` only accepts a BARE
    // `CtorTok` (`cst_v1.rs`'s `OpenIn`), so a NESTED `P` (one wrapped
    // inside an outer `Lib`) could not be opened directly from the
    // document at all; making `P` its own top-level library sidesteps
    // that unrelated grammar limit while still proving the point: a
    // write through `P`'s alias is observed through `Base`'s own name —
    // ONE shared cell, not a copy of a value.
    let lib_base = "module Base = struct val mutable r <- 0 end";
    let lib_p = "module P = struct include Base end";
    // V0_1 dropped 0.0.6's `before` sequencing keyword; use `let _ = e1 in e2`
    // (G10 confirmed wildcard expr-`let` params) to sequence the write-then-read.
    assert_accepts_multi(&[lib_base, lib_p], "let open P in (let _ = (r <- 5) in !Base.r)");
}

/// I13 stdlib shape (spec example 3, the demand pin): `include Basic`
/// splices a variant + a named signature; the ctors stay globally usable
/// (flat namespace, nothing hidden — Std is unsealed here).
#[test]
fn i13_stdlib_shape_include_basic_then_a_nested_impl_module() {
    let lib = "\
module Lib = struct
  module Basic = struct
    type ordering = | Less | Equal | Greater
    signature Ord = sig
      type t :: o
      val compare : t -> t -> ordering
    end
  end
  module Std = struct
    include Basic
    module Int = struct
      type t = int
      val compare m n =
        if m < n then Less else if m == n then Equal else Greater
    end
  end
end
";
    assert_accepts(lib, "match Lib.Std.Int.compare 1 2 with | Less -> 0 | Equal -> 1 | Greater -> 2 end");
}
