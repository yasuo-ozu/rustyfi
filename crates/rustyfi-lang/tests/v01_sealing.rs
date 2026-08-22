//! The end-to-end sealing test suite. Driven through the
//! public `rustyfi_lang::compile_document_v1` (the `LoadedFile`/`LoadedCst`
//! gate-bypass shape `v01_modules.rs`/`v01_slice1.rs` already use) rather
//! than `v1::module_check::check_program`, which is deliberately
//! `pub(crate)` and so unreachable from an integration test.
//!
//! **The `NotADocument` trick.** `compile_document_v1` runs the FULL
//! pipeline (elaborate → `check_program` → compile → eval), and eval
//! requires the entry expression to produce a `Value::Document` — but every
//! ACCEPT-case fixture below is a plain expression (`M.x + 1`, `!M.r`, …),
//! not a document envelope (building one needs a whole `page-break`-based
//! `document` helper the sealed-`val`-only surface cannot declare).
//! Type-checking happens BEFORE evaluation, so a type error always surfaces
//! as `CompileError::Type` regardless of what the body would have evaluated
//! to; a program that type-checks but isn't a document surfaces as
//! `CompileError::NotADocument`, which is only reachable if `check_program`
//! already accepted the program. `assert_accepts` treats both `Ok` and
//! `NotADocument` as "type-checking accepted"; `assert_type_error` demands
//! exactly `CompileError::Type` and returns its message for content checks.
//!
//! The opaque-type-sealing group below covers opaque type sealing,
//! transparent type equality, constructor hiding, command-type decls, and
//! `LONG_LOWER` qualified type names; `assert_accepts_multi`/
//! `assert_type_error_multi` are its twin of the two helpers above, taking
//! SEVERAL dependency libraries (some of its cross-module fixtures) instead
//! of exactly one.

use rustyfi_backend::{FontKey, FontMetrics, Length};
use rustyfi_lang::CompileError;
use rustyfi_loader::{LoadedCst, LoadedFile};
use rustyfi_syntax::parse_file_v1;
use rustyfi_syntax::RustyfiVersion;

/// A `FontMetrics` stub — never actually exercised (every fixture below
/// either fails type-checking or stops at `NotADocument`, before any text is
/// measured), but `compile_document_v1`'s signature still needs a concrete
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

fn run(lib_src: &str, doc_src: &str) -> Result<(), CompileError> {
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
    rustyfi_lang::compile_document_v1(&files, &mono).map(|_| ())
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

/// [`run`]'s multi-dependency twin (some fixtures below need TWO
/// separate `module … = struct … end` library files, one referencing the
/// other's types via `LONG_LOWER`).
fn run_multi(lib_srcs: &[&str], doc_src: &str) -> Result<(), CompileError> {
    let mut files: Vec<LoadedFile> = lib_srcs
        .iter()
        .enumerate()
        .map(|(i, src)| LoadedFile {
            path: std::path::PathBuf::from(format!("lib{i}.satyh")),
            cst: LoadedCst::V0_1(
                parse_file_v1(src).unwrap_or_else(|e| panic!("lib parse failed: {e}")),
            ),
            origin: Default::default(),
            version: RustyfiVersion::V0_1,
        })
        .collect();
    files.push(LoadedFile {
        path: std::path::PathBuf::from("doc.saty"),
        cst: LoadedCst::V0_1(
            parse_file_v1(doc_src).unwrap_or_else(|e| panic!("doc parse failed: {e}")),
        ),
        origin: Default::default(),
        version: RustyfiVersion::V0_1,
    });
    let mono = Mono;
    rustyfi_lang::compile_document_v1(&files, &mono).map(|_| ())
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
// Positive
// ============================================================================

/// Accept: the worked example — width + depth both satisfied, `secret`
/// (undeclared) is silently hidden.
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

#[test]
fn t1_minimal_spec_fixture_accepts() {
    let lib = "module M :> sig val x : int end = struct val x = 1 end";
    assert_accepts(lib, "M.x");
}

/// Polymorphic accept: `val f 'a : 'a -> 'a` over `val f y = y` — usable
/// at multiple instances downstream (proving the COMMITTED scheme is still
/// genuinely polymorphic, not accidentally monomorphized).
#[test]
fn t2_polymorphic_accepts() {
    let lib = "module M :> sig val f 'a : 'a -> 'a end = struct val f y = y end";
    assert_accepts(lib, "(M.f 1, M.f true)");
}

/// Specialize accept: `val f : int -> int` over the polymorphic
/// `val f y = y` — the declared (narrower) type is what's committed.
#[test]
fn t3_specialize_accepts() {
    let lib = "module M :> sig val f : int -> int end = struct val f y = y end";
    assert_accepts(lib, "M.f 1");
}

/// THE SEALING FINGERPRINT: same module as the specialize-accept case above, but `M.f true` — REJECTED,
/// because the committed scheme is the declared `int -> int`, not the
/// inferred `'a -> 'a`. This is the one test that distinguishes real
/// sealing from parse-and-ignore.
#[test]
fn t4_sealing_fingerprint_rejects_narrowed_use() {
    let lib = "module M :> sig val f : int -> int end = struct val f y = y end";
    let msg = assert_type_error(lib, "M.f true");
    assert!(!msg.is_empty(), "expected a real diagnostic");
}

/// Own-type reference (transparent synonym): `val f : t -> t` declared
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

/// The own-type-reference test's variant twin: `val mk : t` declared over `type t = | A  val mk = A`
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

/// Nested seal: a sealed module nested inside an UNSEALED one still gets
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

/// Mutable member: `val r : ref int` declared over `val mutable r <- 0`
/// — accepts (mono vs mono). (0.1's type-application grammar is PREFIX —
/// `ref int`, not 0.0.6's postfix `int ref` — see `ast_v1::TypeApp`'s doc
/// comment.)
#[test]
fn t8_mutable_member_accepts() {
    let lib = "module M :> sig val r : ref int end = struct val mutable r <- 0 end";
    assert_accepts(lib, "!M.r");
}

/// The mutable-member test's negative half: a heterogeneous downstream use is still rejected
/// through the committed `ref int` scheme. `Overwrite`'s target is a bare
/// `VarTok` in 0.1's grammar (no qualified name), so `open` first.
#[test]
fn t8_mutable_member_heterogeneous_use_rejects() {
    let lib = "module M :> sig val r : ref int end = struct val mutable r <- 0 end";
    assert_type_error(lib, "let open M in r <- true");
}

// ============================================================================
// Negative — message content + span
// ============================================================================

/// Depth mismatch: `val x : int` over `` val x = `abc` `` — message
/// names the module, the member, and both types.
#[test]
fn t10_depth_mismatch_message() {
    let lib = "module M :> sig val x : int end = struct val x = `abc` end";
    let msg = assert_type_error(lib, "1");
    assert!(
        msg.contains("module `M` does not match its signature"),
        "{msg}"
    );
    assert!(msg.contains('x'), "{msg}");
    assert!(msg.contains("int"), "{msg}");
    assert!(msg.contains("string"), "{msg}");
}

/// Width missing: `val y : int` with no `y` defined.
#[test]
fn t11_width_missing_message() {
    let lib = "module M :> sig val y : int end = struct val x = 1 end";
    let msg = assert_type_error(lib, "1");
    assert!(msg.contains("never defines `y`"), "{msg}");
}

/// Hiding: an undeclared member used from outside the seal — precise
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

/// The nested-seal test's negative half: a sibling of a nested seal is likewise hidden.
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

/// Declared-more-general: `val f 'a : 'a -> 'a` declared over the
/// monomorphic `val f y = y + 1` — the implementation is less polymorphic
/// than the signature claims.
#[test]
fn t12_declared_more_general_rejects() {
    let lib = "module M :> sig val f 'a : 'a -> 'a end = struct val f y = y + 1 end";
    assert_type_error(lib, "1");
}

/// Escaped skolem: `val r 'a : ref (list 'a)` declared over
/// `val mutable r <- []` — the mutable cell is monomorphic (value
/// restriction), so it cannot honestly claim the declared polymorphism.
/// (Prefix type application, as the mutable-member test's note explains.)
#[test]
fn t13_escaped_skolem_message() {
    let lib = "module M :> sig val r 'a : ref (list 'a) end = struct val mutable r <- [] end";
    let msg = assert_type_error(lib, "1");
    assert!(
        msg.contains("less polymorphic than its signature declares"),
        "{msg}"
    );
}

/// Unbound quant tyvar: `val f : 'a -> 'a` with NO quantifier list.
#[test]
fn t14_unbound_quant_tyvar_message() {
    let lib = "module M :> sig val f : 'a -> 'a end = struct val f y = y end";
    let msg = assert_type_error(lib, "1");
    assert!(msg.contains("not bound by this"), "{msg}");
    assert!(msg.contains("quantifier list"), "{msg}");
}

/// Every remaining non-`Val`/non-`Type`/non-command `Decl` arm errors
/// precisely — never panics. `Decl::Module`/`Decl::Signature` members are
/// recursively matched, so an `N`/`S` the struct body
/// never defines is a REAL width error naming the missing member; `include
/// S` decls FLATTEN through `resolve_sig`, so an undefined
/// `S` is the precise "unknown signature name" error (the same shape the
/// sig-include unknown-name test below pins).
#[test]
fn t15_placeholder_decls_name_their_sub_slice() {
    let cases: &[(&str, &str)] = &[
        ("module N : sig end", "never defines `N`"),
        (
            "signature S = sig end",
            "never defines a signature named `S`",
        ),
        ("include S", "unknown signature name"),
    ];
    for (decl, expect) in cases {
        let lib = format!("module M :> sig {decl} end = struct val x = 1 end");
        let msg = assert_type_error(&lib, "1");
        assert!(
            msg.contains(expect),
            "decl {decl:?}: expected {expect:?} in {msg:?}"
        );
    }
}

/// Non-struct sig forms. `:> S` / `:> A.B.S` resolve through the
/// signature table (`v1/surface.rs`), so an UNDEFINED name
/// is a precise "unknown signature name" error; `sig end with type t = int`
/// resolves for real and, the base sig never declaring
/// `t`, hits the precise "refines a type the signature never declares"
/// error (the same shape the with-type undefined-name test below pins).
#[test]
fn t16_non_struct_sig_forms_name_their_sub_slice() {
    let cases: &[(&str, &str)] = &[
        (
            "module M :> S = struct val x = 1 end",
            "unknown signature name",
        ),
        (
            "module M :> A.B.S = struct val x = 1 end",
            "unknown signature name",
        ),
        (
            "module M :> sig end with type t = int = struct val x = 1 end",
            "refines a type the signature never declares",
        ),
        // A functor-signature ASCRIPTION directly on a module bind stays
        // unsupported (no demand); a functor sig is only enforced as a
        // `Decl::Module` sig MEMBER, which is enforced elsewhere.
        (
            "module M :> (X : S) -> S2 = struct val x = 1 end",
            "not supported",
        ),
    ];
    for (lib, expect) in cases {
        let msg = assert_type_error(lib, "1");
        assert!(
            msg.contains(expect),
            "lib {lib:?}: expected {expect:?} in {msg:?}"
        );
    }
}

// ============================================================================
// Parity/regression
// ============================================================================

/// The end-to-end twin of the whole-program/per-binding parity check: an existing, seal-free `v01_modules.rs`-style source still
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
// Opaque type sealing, transparent type equality,
// constructor hiding, command-type decls, `LONG_LOWER` qualified type names.
// ============================================================================

/// The worked example: `type t :: o` opaque, a plain `val` pair
/// (`make`/`get`), and a sealed inline command (`\show`).
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

/// Opaque accept: the worked example, doc `M.get (M.make 3)` compiles
/// clean.
#[test]
fn u1_opaque_accept() {
    assert_accepts(WORKED_EXAMPLE, "M.get (M.make 3)");
}

/// THE OPACITY FINGERPRINT: `M.get 3` (an `int`, not the opaque `M.t`)
/// is REJECTED, as is `(M.make 3) + 1` — the one test pair distinguishing
/// real abstraction from parse-and-ignore. Messages contain `M.t` and NOT
/// `#` (pins `strip_stamps`).
#[test]
fn u2_opacity_fingerprint() {
    let msg = assert_type_error(WORKED_EXAMPLE, "M.get 3");
    assert!(msg.contains("M.t"), "{msg}");
    assert!(!msg.contains('#'), "no stamp should leak: {msg}");

    let msg2 = assert_type_error(WORKED_EXAMPLE, "(M.make 3) + 1");
    assert!(!msg2.contains('#'), "no stamp should leak: {msg2}");
}

/// Inside-transparency (synonym impl): `type t :: o` over `type t = int;
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

/// Transparent equality accept: sig `type sz = int  val width : sz` over
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

/// Transparent mismatch: sig `type sz = string` over impl `type sz =
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

/// Kind/arity: `type t :: o -> o` over `type t 'a = list 'a` ✓ (with
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

/// Command decls: the worked example's `\show` seals cleanly; a depth
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
    // the guard's accepted set is
    // `inline […]` OR `math […]`, since math commands share the `\` sigil
    // too).
    let msg = assert_type_error(lib_wrong_kind, "1");
    assert!(
        msg.contains("needs an `inline [...]` or `math [...]` command type"),
        "{msg}"
    );

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

/// Ctor hiding: `T 1` (expression) → `constructor T belongs to type t,
/// which module M's signature seals abstract`; `match M.make 1 with T n ->
/// n end` (pattern) → same diagnostic; wildcard `match M.make 1 with _ ->
/// 0 end` ✓ (exhaustive.rs's unknown-domain fact, zero edits).
#[test]
fn u9_ctor_hiding() {
    let msg = assert_type_error(WORKED_EXAMPLE, "T 1");
    assert!(msg.contains("constructor `T` belongs to type `t`"), "{msg}");
    assert!(
        msg.contains("module `M`'s signature seals abstract"),
        "{msg}"
    );

    let msg = assert_type_error(WORKED_EXAMPLE, "match M.make 1 with T n -> n end");
    assert!(msg.contains("constructor `T` belongs to type `t`"), "{msg}");

    assert_accepts(WORKED_EXAMPLE, "match M.make 1 with _ -> 0 end");
}

/// Hidden type via `LONG_LOWER`: impl has `type u = int` the sig omits;
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

/// THE LEAK FIXTURE: `module N :> sig type s = M.t val get2 : s -> int
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

/// Sealed nested module + sibling: `module M = struct module N :> sig
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

/// Self-containment: sig `val f : u -> u` where the sig declares `type
/// t :: o` (opting into type control) but NOT `u` (impl-defined) →
/// "mentions its type `u` without declaring it". A module whose sig
/// declares ZERO types at all is exempt (the own-type-reference test's
/// pinned accept behavior — a bare own-type reference with no sig-level type control at
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
    assert!(
        msg.contains("mentions its type `u` without declaring it"),
        "{msg}"
    );
}

/// Zero-value-member module: `module M :> sig type t :: o end = struct
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
    assert!(
        msg.contains("module `M`'s signature seals abstract"),
        "{msg}"
    );
}

/// The four non-`Val` sig decls all error precisely, never panic.
/// `module N : sig end` / `signature S = sig end` are real width errors
/// (recursively matched); `include S` FLATTENS, so an undefined
/// `S` is "unknown signature name"; a sig-side variant redeclaration
/// (`type t = | A`) is PERMANENTLY unsupported — a transparently declared
/// type whose implementation is a variant is refused.
#[test]
fn u19_placeholder_decls_still_error() {
    let cases: &[(&str, &str)] = &[
        ("module N : sig end", "never defines `N`"),
        (
            "signature S = sig end",
            "never defines a signature named `S`",
        ),
        ("include S", "unknown signature name"),
        ("type t = | A", "not supported"),
    ];
    for (decl, expect) in cases {
        let lib = format!("module M :> sig {decl} end = struct type t = | A  val x = 1 end");
        let msg = assert_type_error(&lib, "1");
        assert!(
            msg.contains(expect),
            "decl {decl:?}: expected {expect:?} in {msg:?}"
        );
    }
}

/// Hidden command member: sig omits `\hidden`; doc `{ \M.hidden; }` →
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
// The unsoundness gate's PROOF: a sealed
// signature whose `val` declares a labeled-optional-argument type
// (`?(bias : int) int -> int`, `TypeExpr::OptRowFun`) matches a plain
// `?(bias = …)`-taking implementation END-TO-END through the SAME `unify`-
// based subsumption path every other sealed `val` flows through — because
// `MonoType::Func` now carries the optional-arg `Row` and
// `typecheck::lower_type_expr`'s `OptRowFun` arm lowers the sig to the SAME
// closed-row `Func` that `Ast::LambdaOpt` infers for the impl. And two
// mismatches (a declared optional label ABSENT from the impl; a WRONG
// codomain type) are rejected — proving the row and the domains are really
// carried through subsumption, not silently dropped.
// ============================================================================

/// (the sealed-sig PROOF): `val f : ?(bias : int) int -> int` over a plain
/// `val f ?(bias = b) x = …` impl seals, and a plain `M.f 1`
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

/// Reject: the sig declares an optional `bias`, but the impl is a plain
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

/// Reject: the impl DOES take the optional `bias`, but its codomain is
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
// Named signatures at ascription sites + module-alias
// re-export.
// ============================================================================

/// Named sig at an ascription: `signature S = sig … end` in a library,
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

/// A hidden member reached through the named-sig seal errors precisely
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

/// An ascription against an UNDEFINED signature name is a precise
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

/// GENERATIVITY FINGERPRINT: the same named signature `Store` used at
/// TWO ascription sites mints DISTINCT opaque `t` stamps, so a value
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

/// Self-containment through a NAMED signature: the direct self-containment
/// test's exact scenario, but
/// the sig is `signature S = …` used at an ascription. The resolved decls
/// feed the identical prescan pipeline, so the same own-type rule applies
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

/// Alias re-export: `module Alias = Base` re-exports Base's public
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

/// Path alias: `module C = A.B` re-exports a nested target's members.
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
// Struct-include (`include M`).
// ============================================================================

/// Worked example 1 — `include Base` inside a SEALED
/// includer `P`. Combines three fingerprints in one fixture:
/// - The included members are usable in a further definition inside the
///   same sealed module (`val extra = get (mk 3)`, `Lib.P.extra + 1` accepts).
/// - Re-abstraction: `P.get (P.mk 1)` accepts (P's own fresh stamp,
///   round-trip), `P.get (Base.mk 1)` REJECTS (P's `t` is a NEW abstract
///   stamp minted at P's own ascription — the include copy re-abstracts,
///   it does not inherit Base's concrete `t`).
/// - Ctor visibility: `Base`'s constructor `T` is NEVER hidden by `P`'s
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
    // Round-trip through P's own fresh stamp.
    assert_accepts(lib, "Lib.P.get (Lib.P.mk 1)");
    assert_accepts(lib, "Lib.P.extra + 1");
    // Re-abstraction: Base's concrete `t` does not subsume P's stamp.
    let msg = assert_type_error(lib, "Lib.P.get (Lib.Base.mk 1)");
    assert!(
        !msg.is_empty(),
        "expected a generativity mismatch, got empty message"
    );
    // Base's own ctor T is untouched by P's seal.
    assert_accepts(lib, "Lib.Base.get (Lib.Base.mk 1)");
    let doc = "match Lib.Base.mk 1 with | T n -> n end";
    assert_accepts(lib, doc);
}

/// A sig omitting an INCLUDED member hides it (the standard
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

/// The sig-omission test's tripwire: the sealed includer's LAST value member arrives VIA the
/// include (not a direct bind) — the ctor-hide/revocation trigger key ("the
/// last value member in source order") must count the SPLICED member, not
/// the last direct bind before it. If `struct_member_names_spliced` ever
/// regressed to ignoring `Bind::Include` the trigger would key on `pre`
/// instead of `mk`; this fixture exercises the exact "include is the final
/// bind" shape and pins the right verdict.
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

/// Include-then-shadow (a documented deviation from upstream, which
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
    assert!(
        !msg.is_empty(),
        "expected a type mismatch (bool + int), got empty message"
    );
}

/// Type re-export: a downstream synonym over the includer's re-exported
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

/// Nested module through include: `Base.Inner.x` is reachable as
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

/// Sig-member re-export: `include Basic` (which itself defines a named
/// signature `Ord`) makes `P.Ord` resolvable at a LATER ascription site.
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

/// The SAME re-export through an ALIAS instead of an include — a
/// gap fix (`Alias.S` never resolved before
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

/// Mutable sharing: `include Base` shares Base's mutable CELL (not a
/// copy of its current value) — a write through `P`'s alias is observed
/// through `Base`'s own name.
#[test]
fn i10_include_shares_the_targets_mutable_cell() {
    // Two dependency files (the cross-file include case rides the existing
    // outward fallback) so the document can `let open P in` — `let open`
    // only accepts a BARE `CtorTok` (`cst_v1.rs`'s `OpenIn`), so a NESTED
    // `P` (one wrapped inside an outer `Lib`) could not be opened directly
    // from the document at all; making `P` its own top-level library
    // sidesteps that unrelated grammar limit while still proving the point:
    // a write through `P`'s alias is observed through `Base`'s own name —
    // ONE shared cell, not a copy of a value.
    let lib_base = "module Base = struct val mutable r <- 0 end";
    let lib_p = "module P = struct include Base end";
    // V0_1 dropped 0.0.6's `before` sequencing keyword; use `let _ = e1 in e2`
    // (wildcard expr-`let` params are supported) to sequence the write-then-read.
    assert_accepts_multi(
        &[lib_base, lib_p],
        "let open P in (let _ = (r <- 5) in !Base.r)",
    );
}

/// Stdlib shape (the demand pin): `include Basic`
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
    assert_accepts(
        lib,
        "match Lib.Std.Int.compare 1 2 with | Less -> 0 | Equal -> 1 | Greater -> 2 end",
    );
}

// ============================================================================
// Nested-module sig MEMBERS, `Decl::Signature` identity,
// `member_revoke`, and alias-body seal narrowing.
// ============================================================================

/// The shared fixture for the tests below (the worked example): `M`'s own seal
/// imposes `module N : sig val y : int end` over a sub-module `N` that is
/// ITSELF sealed more widely (`val y`+`val z`) — the `PendingLink` shape.
fn d123_lib() -> &'static str {
    "\
module M :> sig
  module N : sig val y : int end
  val w : int
end = struct
  module N :> sig val y : int  val z : int end = struct
    val y = 1  val z = 2  val secret = 3
  end
  val w = M.N.z
end
"
}

/// The layer check accepts (`N`'s own sealed `y : int` ⊑ `M`'s declared
/// `y : int`) and a document use of the still-visible members type-checks —
/// `M.N.y` (through the link) and `M.w` (M's own declared member, whose OWN
/// body `M.N.z` type-checked fine while `z` was still committed, BEFORE the
/// parent-imposed revoke trigger fires).
#[test]
fn d1_nested_module_sig_member_accepts_through_the_link() {
    assert_accepts(d123_lib(), "M.N.y + M.w");
}

/// The SAME fixture — `M.N.z` is exported by `N`'s own seal but omitted
/// by `M`'s imposed `S_N`, so it is REVOKED once `M`'s subtree-last value
/// alias (`M.w`) commits; a document (which runs strictly AFTER the whole
/// library) using `M.N.z` is rejected with the precise "not exported"
/// wording, never the raw unbound-variable internal-error format.
#[test]
fn d2_member_revoke_hides_the_omitted_member_after_the_parent_trigger_commits() {
    let msg = assert_type_error(d123_lib(), "M.N.z");
    assert!(
        msg.contains("exists in module `M`") && msg.contains("not exported by its signature"),
        "{msg}"
    );
}

/// (the correctness-core tripwire): `M`'s sig declares
/// `N.id` MORE POLYMORPHIC (`'a -> 'a`) than `N`'s OWN seal committed
/// (`int -> int`, narrower than the RAW impl's genuinely-polymorphic `val id
/// x = x`) — rejected because the layer check compares INNER (the
/// COMMITTED, already-narrowed scheme) against OUTER, never the raw impl
/// directly; checking the raw impl directly would have WRONGLY ACCEPTED
/// this (`'a -> 'a` does subsume `'a -> 'a`), so this pins that the link
/// mechanism never takes that unsound shortcut.
#[test]
fn d3_link_layer_checks_the_childs_own_committed_scheme_not_the_raw_impl() {
    let lib = "\
module M :> sig
  module N : sig val id 'a : 'a -> 'a end
  val w : int
end = struct
  module N :> sig val id : int -> int end = struct
    val id x = x
  end
  val w = 1
end
";
    let msg = assert_type_error(lib, "1");
    assert!(
        msg.contains("does not match its signature") && msg.contains("sub-module `M.N`"),
        "{msg}"
    );
}

/// The link layer compares the declared STAGE as well as the declared
/// type — the parent-vs-child half of the stage conformance
/// `staging_v1.rs` pins for the sig-vs-implementation half.
///
/// Upstream checks the two in the same fold: `subtype_concrete_with_concrete`'s
/// `ConcStructure/ConcStructure` case matches `(stage1, stage2)` — inner
/// against outer — before ever calling `subtype_poly_type`, and reports
/// `NotASubtypeAboutValueStage` on the fallthrough
/// (`dev-0-1-0 signatureSubtyping.ml:279-298`). The accepting rows are exactly
/// `stage_conforms`: a persistent inner member satisfies any outer
/// declaration, the other two must match.
///
/// Reaching this needs a NESTED sealed module (`d123_lib`'s shape): only there
/// do two signatures meet each other, rather than a signature meeting a
/// binding. `int` and `int` unify in every case below, so a link layer that
/// compares only types accepts all four.
#[test]
fn d3s_link_layer_checks_the_declared_stage_not_only_the_type() {
    // The parent promises the document stage may not name `N.y` without a
    // splice (`val ~y`); `N`'s own seal exports it at the document stage. The
    // two disagree, and the disagreement is invisible to the type check.
    let lib = "\
module M :> sig
  module N : sig val ~y : int end
  val w : int
end = struct
  module N :> sig val y : int end = struct
    val y = 1
  end
  val w = 1
end
";
    let msg = assert_type_error(lib, "1");
    assert!(
        msg.contains("stage 1") && msg.contains("stage 0") && msg.contains("sub-module `M.N`"),
        "the error must name both stages and the sub-module, got {msg}"
    );
}

#[test]
fn d3s_a_matching_stage_through_the_link_is_accepted() {
    // Without this the test above is satisfied by refusing every staged member
    // behind a link. Same fixture, now honest on both layers.
    let lib = "\
module M :> sig
  module N : sig val ~y : int end
  val w : int
end = struct
  module N :> sig val ~y : int end = struct
    val ~y = 1
  end
  val w = 1
end
";
    assert_accepts(lib, "1");
}

#[test]
fn d3s_a_persistent_child_member_satisfies_any_parent_declaration() {
    // Upstream's three `(Persistent0, _)` rows, through the link: a persistent
    // member is nameable from every stage, so it delivers whatever the parent's
    // signature narrows it to. This is the row a real 0.1 library actually
    // needs — `persistent` is what a nested utility module exports.
    for parent in ["val y : int", "val ~y : int", "val persistent ~y : int"] {
        let lib = format!(
            "\
module M :> sig
  module N : sig {parent} end
  val w : int
end = struct
  module N :> sig val persistent ~y : int end = struct
    val persistent ~y = 1
  end
  val w = 1
end
"
        );
        assert_accepts(&lib, "1");
    }
}

#[test]
fn d3s_the_stage_subsumption_does_not_run_backwards_through_the_link() {
    // The asymmetry: `persistent` in the PARENT's signature promises the
    // document stage may name `M.N.y` directly, which a child sealed at stage 0
    // cannot honour. Pinning it separately is what stops the rule above from
    // being implemented as "any stage pair involving persistent is fine".
    let lib = "\
module M :> sig
  module N : sig val persistent ~y : int end
  val w : int
end = struct
  module N :> sig val ~y : int end = struct
    val ~y = 1
  end
  val w = 1
end
";
    let msg = assert_type_error(lib, "1");
    assert!(
        msg.contains("stage 0") && msg.contains("persistent stage"),
        "the error must name both stages, got {msg}"
    );
}

/// Width: the sig declares `module P : sig end` but the struct never
/// defines `P` at all, and (second case) defines `P` as a plain VALUE
/// instead of a module — both precise width errors, not a panic.
#[test]
fn d3b_nested_module_width_errors_both_wordings() {
    // Module names are `CtorTok` (uppercase-headed) and value names are
    // `VarTok`/`BindName` (lowercase-headed) — lexically DISJOINT in this
    // grammar, so a value can never collide with a `Decl::Module`'s
    // uppercase member name; only the "never defines at all" width wording
    // is reachable from real source (the "defines it as a value, not a
    // module" wording stays a defensive, sound fallback — never actually
    // hit, since no source can construct that shape).
    let missing = assert_type_error(
        "module M :> sig module P : sig end end = struct val x = 1 end",
        "1",
    );
    assert!(missing.contains("never defines `P`"), "{missing}");
}

/// An UNSEALED child recursed via a SYNTHETIC (parent-imposed) seal —
/// the declared member escapes, and an omitted member is revoked at the
/// PARENT's own trigger rather than hidden immediately: a SIBLING use
/// (`val w = M.N.z`, inside the SAME lib, BEFORE the trigger fires) still
/// accepts — the parent-imposed-deferral pin.
#[test]
fn d4_unsealed_child_defers_hiding_to_the_parent_trigger() {
    let lib = "\
module M :> sig
  module N : sig val y : int end
  val w : int
end = struct
  module N = struct val y = 1  val z = 2 end
  val w = M.N.z
end
";
    assert_accepts(lib, "M.N.y + M.w");
    let msg = assert_type_error(lib, "M.N.z");
    assert!(
        msg.contains("exists in module `M`") && msg.contains("not exported by its signature"),
        "{msg}"
    );
}

/// Alias-body seal narrowing is now enforced — `module M :> sig val mk
/// : int -> int end = Base` REJECTS when `Base`'s `mk` does not match, and
/// hides an un-declared member of `Base` (the alias copies through the
/// same seal machinery a struct literal gets); the double spelling `module
/// M :> S1 = Base :> S2` chains innermost-first (`S2` is the real seal,
/// `S1` an outer link).
#[test]
fn d5_alias_body_seal_narrowing_is_enforced() {
    // `module M :> S = N` can only occur NESTED (a `Bind::Module`) — the
    // top-level `FileV1::Library` shape is always `module Name = struct …
    // end` — so every case below wraps the aliasing module `M` in an outer
    // `Wrap`, cross-file-resolving `Base` via `v1/surface.rs`'s outward
    // search (the same mechanism `i9`/`i9b` already exercise).
    let base = "module Base = struct val mk x = x + 1  val extra = 9 end";

    // Mismatch: `mk` is declared `int -> int -> int` (wrong arity/shape).
    let mismatch_lib = "module Wrap = struct\n\
                         module M :> sig val mk : int -> int -> int end = Base\n\
                         end";
    let msg = assert_type_error_multi(&[base, mismatch_lib], "1");
    assert!(msg.contains("does not match its signature"), "{msg}");

    // Accept + hide: `mk` matches, `extra` is un-declared and hidden.
    let accept_lib = "module Wrap = struct\n\
                       module M :> sig val mk : int -> int end = Base\n\
                       end";
    assert_accepts_multi(&[base, accept_lib], "Wrap.M.mk 1");
    let hidden_msg = assert_type_error_multi(&[base, accept_lib], "Wrap.M.extra");
    assert!(
        hidden_msg.contains("exists in module `Wrap.M`") && hidden_msg.contains("not exported"),
        "{hidden_msg}"
    );

    // The double-spelling chain: `module M :> S1 = Base :> S2` — `S2` (on
    // the `Coerce`) is the real seal, `S1` (the outer `sig_annot`) an
    // additional link; `extra` is declared by `S2` but omitted by `S1`, so
    // it is hidden through the LINK's own revoke, not the inner seal's.
    let chain_accept_lib = "module Wrap = struct\n\
                             module M :> sig val mk : int -> int  val extra : int end \
                             = Base :> sig val mk : int -> int  val extra : int end\n\
                             end";
    assert_accepts_multi(&[base, chain_accept_lib], "Wrap.M.mk 1 + Wrap.M.extra");

    let chain_hide_lib = "module Wrap = struct\n\
                           module M :> sig val mk : int -> int end \
                           = Base :> sig val mk : int -> int  val extra : int end\n\
                           end";
    let chain_hidden = assert_type_error_multi(&[base, chain_hide_lib], "Wrap.M.extra");
    assert!(chain_hidden.contains("not exported"), "{chain_hidden}");
}

/// `Decl::Signature` members — verbatim re-declaration (code.satyh's
/// shape) accepts; a differing body is the identity-comparator error; an
/// omitted struct signature is rejected too (narrower width).
#[test]
fn d7_decl_signature_verbatim_accepts_differing_rejects() {
    let verbatim = "\
module M :> sig
  signature Settings = sig val font-size : int end
  module Make : Settings
end = struct
  signature Settings = sig val font-size : int end
  module Make = struct val font-size = 10 end
end
";
    assert_accepts(verbatim, "M.Make.font-size");

    let differing = "\
module M :> sig
  signature Settings = sig val font-size : int end
end = struct
  signature Settings = sig val font-weight : int end
end
";
    let msg = assert_type_error(differing, "1");
    assert!(
        msg.contains("does not match its signature") && msg.contains("Settings"),
        "{msg}"
    );
}

// ============================================================================
// Functor sig-MEMBERS inside a `:> sig … end` umbrella.
// ============================================================================

/// The map.satyg-shaped umbrella fixture: `Map :> sig module Make : (Key :
/// Ord) -> sig … end end`, with an internal helper the codomain never
/// declares.
fn umbrella_lib() -> &'static str {
    "\
module Lib = struct
  signature Ord = sig
    type t :: o
    val compare : t -> t -> int
  end
  module IntOrd = struct
    type t = int
    val compare x y = x - y
  end
  module FlagOrd = struct
    type t = | Yes | No
    val compare x y = 0
  end
  module Map :> sig
    module Make : (Key : Ord) -> sig
      type t :: o
      val empty : t
      val add : Key.t -> t -> t
    end
  end = struct
    module Make = fun (Key : Ord) -> struct
      % A synonym body (never a fresh variant) — every instantiation of
      % this SAME functor shares no constructor names, sidestepping this
      % port's flat/global ctor namespace (a documented, pre-existing
      % 0.0.6-inherited limitation unrelated to sealing — see
      % `build_impl_type_table`'s own doc comment) so the fixture's
      % abstract-fingerprint pin isolates 2f-2b's OWN mechanism.
      type t = list Key.t
      val empty = []
      val add k m = k :: m
      val helper x = x
    end
  end
  module M1 = Map.Make IntOrd
  module M2 = Map.Make IntOrd
  module M3 = Map.Make FlagOrd
end
"
}

/// The umbrella accepts and a consumer application works through the
/// declared (sealed) interface.
#[test]
fn u1_functor_umbrella_accepts_and_applications_work() {
    assert_accepts(umbrella_lib(), "Lib.M1.add 1 Lib.M1.empty");
}

/// F-gen: the abstract fingerprint — SAME-instantiation use accepts,
/// CROSS-instantiation use (mixing `M1`'s and `M2`'s results, two SEPARATE
/// applications of the identical functor to the identical argument) is
/// REJECTED at the abstract level (fresh stamps per application —
/// generative); internal helpers (`helper`) are hidden
/// with the sealing wording, not a raw unbound-variable message.
#[test]
fn f_gen_abstract_fingerprint_rejects_cross_instantiation_use() {
    let msg = assert_type_error(umbrella_lib(), "Lib.M1.add 1 Lib.M2.empty");
    assert!(!msg.is_empty(), "{msg}");
    let hidden_msg = assert_type_error(umbrella_lib(), "Lib.M1.helper 1");
    assert!(
        hidden_msg.contains("exists in module `Lib.M1`") && hidden_msg.contains("not exported"),
        "{hidden_msg}"
    );
}

/// A hidden-functor application is rejected — the umbrella's `Map`
/// exports ONLY `Make`; a hypothetical direct access to a functor the
/// umbrella never declares (simulated here by sealing `Map` OVER a wider
/// struct that ALSO defines a second, undeclared functor `Other`) is
/// rejected with the "not exported" wording.
#[test]
fn u2_hidden_functor_application_is_rejected() {
    let lib = "\
module Lib = struct
  signature Ord = sig
    type t :: o
    val compare : t -> t -> int
  end
  module IntOrd = struct
    type t = int
    val compare x y = x - y
  end
  module Map :> sig
    module Make : (Key : Ord) -> sig type t :: o  val empty : t end
  end = struct
    module Make = fun (Key : Ord) -> struct type t = | E  val empty = E end
    module Other = fun (Key : Ord) -> struct type t = | E  val empty = E end
  end
  module Bad = Map.Other IntOrd
end
";
    let msg = assert_type_error(lib, "1");
    assert!(msg.contains("not exported by its signature"), "{msg}");
}

/// `NestedFunctorSubstitution` reachable from real source — a curried
/// functor sig MEMBER (`module F : (X:S) -> (Y:S2) -> S3`) is the typed
/// rejection, not a panic; `t16`'s functor-ascription row (a DIRECT
/// ascription, not a member) is the separate, still-unsupported shape.
#[test]
fn u4_curried_functor_sig_member_is_the_nested_functor_substitution_rejection() {
    let lib = "\
module M :> sig
  signature S = sig end
  module F : (X : S) -> (Y : S) -> S
end = struct
  signature S = sig end
  module F = fun (X : S) -> struct end
end
";
    let msg = assert_type_error(lib, "1");
    assert!(
        msg.contains("higher-order") && msg.contains("nested-functor substitution"),
        "{msg}"
    );
}

// ============================================================================
// Sig-`include` (`Decl::Include`) + `with type`
// (`SigExpr::WithType`).
// ============================================================================

/// Sig-include accepts + hides an undeclared member: `Big` splices
/// `Eq`'s `type t :: o`/`val equal` in, adds `val hash`; `A :> Big` defines
/// all three PLUS an undeclared `secret` — the module itself type-checks,
/// and `secret` is hidden with the standard "not exported" wording.
#[test]
fn s1_sig_include_accepts_and_hides_undeclared_member() {
    let lib = "\
module Lib = struct
  signature Eq = sig
    type t :: o
    val equal : t -> t -> bool
  end
  signature Big = sig
    include Eq
    val hash : t -> int
  end
  module A :> Big = struct
    type t = int
    val equal x y = x == y
    val hash x = x
    val secret = 99
  end
end
";
    assert_accepts(lib, "1");
    let msg = assert_type_error(lib, "Lib.A.secret");
    assert!(msg.contains("not exported"), "{msg}");
}

/// Include of a literal + a dep-file-qualified path: `include sig val x
/// : int end` inline, and `include Other.S` naming a signature defined in
/// an earlier module.
#[test]
fn s2_sig_include_literal_and_path() {
    let lib = "\
module Lib = struct
  module Other = struct
    signature S = sig val z : int end
  end
  module A :> sig include sig val x : int end end = struct val x = 1 end
  module B :> sig include Other.S end = struct val z = 1 end
end
";
    assert_accepts(lib, "Lib.A.x + Lib.B.z");
}

/// Spliced-opaque generativity: two modules independently sealed `:>
/// Big` (an included `type t :: o`, unrefined) each mint their OWN fresh
/// stamp for `t` — mixing values across the two instantiations rejects,
/// same-instantiation use accepts (the per-site-stamps-through-a-splice
/// fingerprint).
#[test]
fn s3_spliced_opaque_generativity_per_site_stamps() {
    let lib = "\
module Lib = struct
  signature Eq = sig
    type t :: o
    val mk : int -> t
  end
  signature Big = sig
    include Eq
    val hash : t -> int
  end
  module A :> Big = struct
    type t = int
    val mk n = n
    val hash x = x
  end
  module B :> Big = struct
    type t = int
    val mk n = n
    val hash x = x
  end
end
";
    let msg = assert_type_error(lib, "Lib.A.hash (Lib.B.mk 1)");
    assert!(!msg.is_empty(), "{msg}");
    assert_accepts(lib, "Lib.A.hash (Lib.A.mk 1)");
}

/// Conflict: an `include`-spliced `val equal` colliding with a directly
/// declared `val equal` in the same sig is a hard `ConflictInSignature`-
/// shaped error; the tightening this ALSO applies to a literal sig with two
/// DIRECT `val x` decls (pre-2e-2: silently last-wins) is pinned too.
#[test]
fn s4_sig_include_conflict_rejects() {
    let lib = "\
module Lib = struct
  signature Eq = sig
    type t :: o
    val equal : t -> t -> bool
  end
  module A :> sig
    include Eq
    val equal : int -> int -> bool
  end = struct
    type t = int
    val equal x y = x == y
  end
end
";
    let msg = assert_type_error(lib, "1");
    assert!(
        msg.contains("conflicting declarations for `equal`"),
        "{msg}"
    );

    let lib_direct = "module M :> sig val x : int  val x : bool end = struct val x = 1 end";
    let msg2 = assert_type_error(lib_direct, "1");
    assert!(msg2.contains("conflicting declarations for `x`"), "{msg2}");
}

/// Cycle: `signature S = sig include S end`, used at an ascription,
/// "includes itself" — never a stack overflow/panic.
#[test]
fn s5_sig_include_self_cycle_rejects() {
    let lib = "\
module Lib = struct
  signature S = sig include S end
  module A :> S = struct end
end
";
    let msg = assert_type_error(lib, "1");
    assert!(msg.contains("includes itself"), "{msg}");
}

/// Unknown: `sig include Nope end` → "unknown signature name `Nope`"
/// (the shape `t15`/`u19`'s re-pinned rows exercise end-to-end).
#[test]
fn s6_sig_include_unknown_name_rejects() {
    let lib = "module M :> sig include Nope end = struct val x = 1 end";
    let msg = assert_type_error(lib, "1");
    assert!(msg.contains("unknown signature name `Nope`"), "{msg}");
}

/// Refinement accepts + the transparency fingerprint: `sig type t :: o
/// val mk : int -> t val get : t -> int end with type t = int` — BOTH
/// directions flow concrete ints through the doc side (`t` is really
/// transparent, not just parsed-and-ignored).
#[test]
fn w1_with_type_refinement_accepts_and_is_transparent() {
    let lib = "\
module A :> sig
  type t :: o
  val mk : int -> t
  val get : t -> int
end with type t = int = struct
  type t = int
  val mk n = n
  val get x = x
end
";
    assert_accepts(lib, "A.get 5 + A.mk 1");
}

/// Impl mismatch: `with type t = int` over an implementation whose OWN
/// `type t = bool` — the transparent-equality error fires, reached via
/// refinement instead of a literal `type t = τ` sig decl.
#[test]
fn w2_with_type_impl_mismatch_rejects() {
    let lib = "\
module A :> sig
  type t :: o
  val mk : int -> t
end with type t = int = struct
  type t = bool
  val mk n = n > 0
end
";
    let msg = assert_type_error(lib, "1");
    assert!(msg.contains("does not match its signature"), "{msg}");
}

/// Undefined: `with type u = int` over a sig that never declares `u` —
/// "refines a type the signature never declares".
#[test]
fn w3_with_type_refines_undeclared_type_rejects() {
    let lib = "module M :> sig val x : int end with type u = int = struct val x = 1 end";
    let msg = assert_type_error(lib, "1");
    assert!(
        msg.contains("refines a type the signature never declares"),
        "{msg}"
    );
}

/// Transparent restrict: the base sig already declares `type t = bool`
/// TRANSPARENTLY — `with type t = int` cannot restrict it (upstream
/// `CannotRestrictTransparentType`).
#[test]
fn w4_with_type_over_already_transparent_type_rejects() {
    let lib = "\
module M :> sig
  type t = bool
  val x : t
end with type t = int = struct
  type t = bool
  val x = true
end
";
    let msg = assert_type_error(lib, "1");
    assert!(msg.contains("already declares it transparently"), "{msg}");
}

/// Arity both ways: `type t :: o -> o` refined by an arity-0 `with type
/// t = int` rejects (the refine-arity error); `type t :: o -> o` refined by
/// `with type t 'a = list 'a` over a matching `type t 'a = list 'a` impl
/// accepts.
#[test]
fn w5_with_type_arity_checks_both_directions() {
    let lib_bad = "\
module M :> sig
  type t :: o -> o
end with type t = int = struct
  type t 'a = list 'a
end
";
    let msg = assert_type_error(lib_bad, "1");
    assert!(msg.contains("arity"), "{msg}");

    let lib_ok = "\
module M :> sig
  type t :: o -> o
  val wrap 'a : 'a -> t 'a
end with type t 'a = list 'a = struct
  type t 'a = list 'a
  val wrap x = [x]
end
";
    assert_accepts(lib_ok, "M.wrap 1");
}

/// Named-sig storage + chained refinement: `signature S2 = S with type
/// t = int` composes across the `signature` bind boundary (`B :> S2`
/// accepts, `B.mk 1 + 1` typechecks — a plain-int flow); refining `S2`
/// AGAIN at a use site (`S2 with type t = bool`) hits the ordered-first-
/// match `CannotRestrictTransparentType` rule.
#[test]
fn w6_named_sig_with_type_storage_and_chained_refine_rejects() {
    let lib = "\
module Lib = struct
  signature S = sig
    type t :: o
    val mk : int -> t
  end
  signature S2 = S with type t = int
  module B :> S2 = struct
    type t = int
    val mk n = n
  end
end
";
    assert_accepts(lib, "Lib.B.mk 1 + 1");

    let lib_chain = "\
module Lib = struct
  signature S = sig
    type t :: o
    val mk : int -> t
  end
  signature S2 = S with type t = int
  module C :> S2 with type t = bool = struct
    type t = int
    val mk n = n
  end
end
";
    let msg = assert_type_error(lib_chain, "1");
    assert!(msg.contains("already declares it transparently"), "{msg}");
}

/// Mixed generativity: `A :> S` (unrefined, fresh stamp), `C`/`D :> S
/// with type t = int` (both refined to concrete `int`) — `C`/`D` mix with
/// each other and with plain ints; `A` mixes with neither (upstream's
/// `quant.remove(tyid)` semantics — refining removes the member from the
/// quantifier, reproduced here by simply never minting a stamp).
#[test]
fn w7_with_type_mixed_generativity() {
    let lib = "\
module Lib = struct
  signature S = sig
    type t :: o
    val mk : int -> t
  end
  module A :> S = struct
    type t = int
    val mk n = n
  end
  module C :> S with type t = int = struct
    type t = int
    val mk n = n
  end
  module D :> S with type t = int = struct
    type t = int
    val mk n = n
  end
end
";
    assert_accepts(lib, "Lib.C.mk 1 + Lib.D.mk 2");
    assert_accepts(lib, "Lib.C.mk 1 + 1");
    let msg = assert_type_error(lib, "Lib.A.mk 1 + Lib.C.mk 2");
    assert!(!msg.is_empty(), "{msg}");
}

/// Out-of-scope rows: a variant-bodied refinement (`with type t = | A`)
/// cannot introduce constructors.
#[test]
fn w8_with_type_out_of_scope_rows() {
    let lib_variant = "module M :> sig type t :: o end with type t = | A = struct type t = | A end";
    let msg = assert_type_error(lib_variant, "1");
    assert!(msg.contains("cannot introduce constructors"), "{msg}");
}

/// `S with M type t = τ` — the SUB-MODULE refinement form, routed into
/// the `Decl::Module` members the child layer created: the
/// chain rides on [`surface::Refine::path`], `prescan_seal_types` hands it to
/// the `Decl::Module` member of that name with one segment consumed, and the
/// child layer applies it to its own `type t :: o` exactly as an
/// unqualified `with type` would — no stamp minted, `M.Sub.t` transparent.
///
/// The proof is TRANSPARENCY, not mere acceptance: an unrefined sibling
/// (`Opaque`) must keep its stamp and refuse to mix with `int`.
#[test]
fn w8b_with_submodule_type_refines_the_nested_member() {
    let lib = "\
module M :> sig
  module Sub : sig
    type t :: o
    val mk : int -> t
  end
end with Sub type t = int = struct
  module Sub = struct
    type t = int
    val mk n = n
  end
end
";
    assert_accepts(lib, "M.Sub.mk 1 + 1");

    let lib_opaque = "\
module M :> sig
  module Sub : sig
    type t :: o
    val mk : int -> t
  end
end = struct
  module Sub = struct
    type t = int
    val mk n = n
  end
end
";
    let msg = assert_type_error(lib_opaque, "M.Sub.mk 1 + 1");
    assert!(!msg.is_empty(), "{msg}");
}

/// The same form, spelled through a NAMED signature
/// (`signature S2 = S with Sub type t = int`) — the refinement is stored on
/// the `SigDef` and inherited at every use site, the same composition the
/// unqualified-form test above pins.
#[test]
fn w8c_with_submodule_type_composes_through_a_named_signature() {
    let lib = "\
module Lib = struct
  signature S = sig
    module Sub : sig
      type t :: o
      val mk : int -> t
    end
  end
  signature S2 = S with Sub type t = int
  module M :> S2 = struct
    module Sub = struct
      type t = int
      val mk n = n
    end
  end
end
";
    assert_accepts(lib, "Lib.M.Sub.mk 1 + 1");
}

/// The two shapes `S with M type t = τ` still refuses, each for a reason
/// derived from what a refinement IS — a narrowing of a type member that
/// exists at a path:
///
/// - a chain segment naming no `module` member of the signature at all;
/// - a chain segment naming a FUNCTOR member: a functor has no types at a
///   path of its own, only at an application's.
#[test]
fn w8d_with_submodule_type_refuses_a_path_that_names_no_module() {
    let lib_missing = "\
module M :> sig
  module Sub : sig type t :: o end
end with Nope type t = int = struct
  module Sub = struct type t = int end
end
";
    let msg = assert_type_error(lib_missing, "1");
    assert!(
        msg.contains("never declares it as a module") || msg.contains("never declares as a module"),
        "{msg}"
    );

    let lib_functor = "\
module M :> sig
  module Make : (X : sig val n : int end) -> sig type t :: o end
end with Make type t = int = struct
  module Make = fun (X : sig val n : int end) -> struct type t = int end
end
";
    let msg2 = assert_type_error(lib_functor, "1");
    assert!(msg2.contains("FUNCTOR"), "{msg2}");
}

/// Refinement through a splice: example 2 verbatim — `Big` includes `Eq`
/// (contributing the abstract `t`), `A :> Big with type t = int` refines
/// the INCLUDE-DEFINED `t`; both `hash` (declared OUTSIDE the include) and
/// `equal` (declared INSIDE it) see the same transparent `t` — the splice
/// makes that composition just work.
#[test]
fn w9_with_type_refinement_through_a_splice() {
    let lib = "\
module Lib = struct
  signature Eq = sig
    type t :: o
    val equal : t -> t -> bool
  end
  signature Big = sig
    include Eq
    val hash : t -> int
  end
  module A :> Big with type t = int = struct
    type t = int
    val equal x y = x == y
    val hash x = x
  end
end
";
    assert_accepts(lib, "Lib.A.hash 5 + 1");
    assert_accepts(lib, "Lib.A.equal 1 2");
}

// ============================================================================
// A functor PARAMETER signature may use `include`
// (flattened for real, at the functor-application width/arity check);
// `with type` there is an EXPLICIT reject (this check's own "name/arity only"
// posture), never silence.
// ============================================================================

/// (i) A functor parameter signature using `include` flattens for real:
/// the width check against the argument surface sees `Base`'s spliced `x`,
/// so a conforming argument accepts and a non-conforming one rejects with
/// the ordinary functor-argument-mismatch wording.
#[test]
fn functor_param_sig_include_flattens_for_width_check() {
    let lib_ok = "\
module Lib = struct
  signature Base = sig
    val x : int
  end
  signature Param = sig
    include Base
  end
  module F = fun (X : Param) -> struct val y = X.x end
  module Arg = struct val x = 1 end
  module Applied = F Arg
end
";
    assert_accepts(lib_ok, "Lib.Applied.y");

    // The functor BODY deliberately never references the missing member
    // (`X.y`, from the sig's OWN direct decl) — only the INCLUDE-supplied
    // `X.x` — so LOWERING alone can't catch `Bad`'s missing `y`
    // (elaboration only complains about names it actually substitutes and
    // uses); this isolates `check_functor_applications`'s OWN width check
    // seeing THROUGH the splice (mirrors `t_chk2`'s own shape,
    // `v1::module_check`'s unit tests).
    let lib_bad = "\
module Lib = struct
  signature Base = sig
    val x : int
  end
  signature Param = sig
    include Base
    val y : int
  end
  module F = fun (X : Param) -> struct val out = X.x end
  module Bad = struct val x = 1 end
  module Applied = F Bad
end
";
    let msg = assert_type_error(lib_bad, "1");
    assert!(msg.contains("does not match functor"), "{msg}");
}

/// (ii) `fun (X : S with type t = int) -> …`: a `with type` on a functor
/// PARAMETER signature is name-invisible (refining never changes the name
/// set), so 2f-1's name/arity-only check can't enforce it — an explicit
/// reject fires instead of silently ignoring the refinement.
#[test]
fn functor_param_sig_with_type_is_an_explicit_reject_not_silence() {
    let lib = "\
module Lib = struct
  signature S = sig
    type t :: o
    val mk : int -> t
  end
  module F = fun (X : S with type t = int) -> struct val y = X.mk 1 end
  module Arg = struct type t = int  val mk n = n end
  module Applied = F Arg
end
";
    let msg = assert_type_error(lib, "1");
    assert!(
        msg.contains("with type") && msg.contains("not enforced"),
        "{msg}"
    );
}
