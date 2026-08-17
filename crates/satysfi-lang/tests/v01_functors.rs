//! Sub-slice 2f-1 (`…/tmp/slice2f-functors.md` §5): end-to-end integration
//! tests for first-order single-parameter functors, driven through the REAL
//! public pipeline (`satysfi_lang::compile_document_v1`) — the same harness
//! `tests/v01_sealing.rs` uses (see that file's doc comment for the
//! `NotADocument` trick: every fixture below is a plain expression, not a
//! real SATySFi document, so a program that type-checks AND evaluates
//! surfaces as `CompileError::NotADocument` rather than `Ok`; both count as
//! "accepted" — and since evaluation genuinely runs to discover the result
//! isn't a `Value::Document`, `NotADocument` also pins that the functor
//! machinery produces a program that both type-checks AND evaluates, not
//! merely one that elaborates).

use satysfi_backend::{FontKey, FontMetrics, Length};
use satysfi_lang::CompileError;
use satysfi_loader::{LoadedCst, LoadedFile};
use satysfi_syntax::parse_file_v1;
use satysfi_syntax::SatysfiVersion;

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
            origin: Default::default(),
            version: SatysfiVersion::V0_1,
        },
        LoadedFile {
            path: std::path::PathBuf::from("doc.saty"),
            cst: LoadedCst::V0_1(parse_file_v1(doc_src).unwrap_or_else(|e| panic!("doc parse failed: {e}"))),
            origin: Default::default(),
            version: SatysfiVersion::V0_1,
        },
    ];
    let mono = Mono;
    satysfi_lang::compile_document_v1(&files, &mono).map(|_| ())
}

fn assert_accepts(lib_src: &str, doc_src: &str) {
    match run(lib_src, doc_src) {
        Ok(()) | Err(CompileError::NotADocument(_)) => {}
        Err(other) => panic!("expected compile+eval to succeed, got: {other}"),
    }
}

fn assert_type_error(lib_src: &str, doc_src: &str) -> String {
    match run(lib_src, doc_src) {
        Err(CompileError::Type(e)) => e.to_string(),
        Err(other) => panic!("expected a Type error, got: {other}"),
        Ok(()) => panic!("expected type-checking to reject, but compilation succeeded"),
    }
}

/// I-fn1 (spec §5): `map.satyg`'s `include Make Int` shape — a functor over
/// a tiny `Ord`-like parameter, applied to a concrete module, its result
/// `include`d and its members USED. Compiles and evaluates end-to-end.
#[test]
fn i_fn1_include_application_end_to_end() {
    let lib = "\
module Lib = struct
  module Int = struct
    type t = int
    val compare x y = x - y
  end
  module Make = fun (Key : sig type t :: o val compare : t -> t -> int end) -> struct
    type t = Wrap of Key.t
    val wrap x = Wrap x
    val cmp a b =
      match a with
      | Wrap x ->
        (match b with
         | Wrap y -> Key.compare x y
         end)
      end
  end
  module Test = struct
    include Make Int
  end
end
";
    assert_accepts(lib, "Lib.Test.cmp (Lib.Test.wrap 3) (Lib.Test.wrap 5)");
}

/// I-fn2 (spec §5): `code.satyh`'s `module Default = Make DefaultSettings`
/// shape — a functor over a value-only NAMED signature parameter, bound as
/// a module, then referenced QUALIFIED (`Lib.Default.…`, the std-ja
/// `\Code.Default.code` touch-point's shape, simplified to a plain `val`
/// rather than a real inline command — see this file's module doc comment;
/// the command-binding machinery itself is unrelated to 2f-1).
#[test]
fn i_fn2_module_bind_application_and_qualified_use() {
    let lib = "\
module Lib = struct
  signature Settings = sig
    val label : int
  end
  module DefaultSettings = struct
    val label = 42
  end
  module ConsoleSettings = struct
    val label = 7
  end
  module Make = fun (X : Settings) -> struct
    val get-label ctx = X.label + ctx
  end
  module Default = Make DefaultSettings
  module Console = Make ConsoleSettings
end
";
    assert_accepts(lib, "Lib.Default.get-label 0");
    // Sub-slice 2f-1 §2.4: two applications are independently instantiated
    // — `Console` reads ITS OWN argument, not `Default`'s.
    assert_accepts(lib, "Lib.Console.get-label 0");
}

/// I-fn3 (spec §5): I-fn1's shape, but the argument is missing `compare` —
/// a precise, functor-framed compile error, end-to-end.
#[test]
fn i_fn3_param_sig_mismatch_rejected_end_to_end() {
    let lib = "\
module Lib = struct
  module BadInt = struct
    type t = int
  end
  module Make = fun (Key : sig type t :: o val compare : t -> t -> int end) -> struct
    type t = Wrap of Key.t
    val wrap x = Wrap x
  end
  module Test = struct
    include Make BadInt
  end
end
";
    let msg = assert_type_error(lib, "1");
    assert!(msg.contains("does not match functor"), "{msg}");
    assert!(msg.contains("compare"), "{msg}");
}

/// A functor DEFINITION alone (never applied) compiles to a program with
/// zero residue from the functor itself — the erasure invariant (spec §6):
/// a document that never mentions the functor at all still compiles fine
/// even though the library declares one.
#[test]
fn functor_definition_alone_is_erased() {
    let lib = "\
module Lib = struct
  module Make = fun (Key : sig val v : int end) -> struct
    val get () = Key.v
  end
  val untouched = 1
end
";
    assert_accepts(lib, "Lib.untouched + 1");
}

// ============================================================================
// Sub-slice 2f-2a (`…/tmp/slice2d3b-2f2-sigmembers.md` §4/§8): a functor
// BODY that APPLIES another functor (application-in-body, a parameter
// flowing as a functor ARGUMENT) plus relative-sibling-head absolutization
// (a value/type reference whose head names a SIBLING nested module, not a
// dependency or the enclosing functor's own parameter) — `set.satyg`'s and
// `code.satyh`'s shapes.
// ============================================================================

/// F-set1 (spec §4.3's worked example, end-to-end): `Set.Make`'s body
/// applies `Map.Make` to ITS OWN parameter (`module Impl = Map.Make Elem`),
/// then references the instantiated result through RELATIVE sibling paths
/// (`Impl.t`/`Impl.wrap`/`Impl.cmp`) — both halves of 2f-2a in one fixture.
/// Two independent applications of `Set.Make` (§0.4's generativity caveat)
/// stay distinct at the abstract level is NOT exercised here (no sealing in
/// this fixture — that is 2f-2b's `F-gen`); this pins only that the shape
/// compiles, type-checks, and evaluates.
#[test]
fn f_set1_functor_body_applies_another_functor_to_its_own_parameter() {
    let lib = "\
module Lib = struct
  module IntOrd = struct
    type t = int
    val compare x y = x - y
  end
  module Map = struct
    module Make = fun (Key : sig type t :: o val compare : t -> t -> int end) -> struct
      type t = Wrap of Key.t
      val wrap x = Wrap x
      val cmp a b =
        match a with
        | Wrap x ->
          (match b with
           | Wrap y -> Key.compare x y
           end)
        end
    end
  end
  module Set = struct
    module Make = fun (Elem : sig type t :: o val compare : t -> t -> int end) -> struct
      module Impl = Map.Make Elem
      type t = Impl.t
      val wrap x = Impl.wrap x
      val cmp a b = Impl.cmp a b
    end
  end
  module S = Set.Make IntOrd
end
";
    assert_accepts(lib, "Lib.S.cmp (Lib.S.wrap 3) (Lib.S.wrap 5)");
}

/// F-set1-neg: the same shape, but the inner application's argument (`Elem`)
/// does not satisfy `Map.Make`'s declared parameter signature — a precise,
/// functor-framed compile error still surfaces through the app-in-body
/// path (the width/arity check `check_functor_applications` already runs
/// on the inner, frozen `AppResolution`, module_check.rs needing zero
/// changes for 2f-2a).
#[test]
fn f_set1_neg_inner_application_param_mismatch_rejected() {
    let lib = "\
module Lib = struct
  module BadElem = struct
    type t = int
  end
  module Map = struct
    module Make = fun (Key : sig type t :: o val compare : t -> t -> int end) -> struct
      type t = Wrap of Key.t
      val wrap x = Wrap x
    end
  end
  module Set = struct
    module Make = fun (Elem : sig type t :: o val compare : t -> t -> int end) -> struct
      module Impl = Map.Make Elem
      type t = Impl.t
      val wrap x = Impl.wrap x
    end
  end
  module S = Set.Make BadElem
end
";
    let msg = assert_type_error(lib, "1");
    assert!(msg.contains("does not match functor"), "{msg}");
    assert!(msg.contains("compare"), "{msg}");
}

/// F-abs2 (spec §8): a PLAIN (non-functor) module whose body references a
/// SIBLING nested module by a relative dotted path — compiles and
/// evaluates (retires the 2d-3 "relative sibling module references" gap
/// for plain, non-functor expressions too, not just functor bodies).
#[test]
fn f_abs2_plain_module_relative_sibling_reference() {
    let lib = "\
module Lib = struct
  module Inner = struct
    val x = 42
  end
  val y = Inner.x + 1
end
";
    assert_accepts(lib, "Lib.y");
}

/// F-abs3 (`code.satyh`'s `Console.scheme` shape): a block command's body
/// invokes a SIBLING nested module's own block command by a relative
/// dotted path (`+Inner.greet`) — the command-name leaf site, absolutized
/// the same way as the value/type sites, no guard (unlike the functor-
/// parameter case, spec §4.2 point 2).
#[test]
fn f_abs3_relative_sibling_command_invocation() {
    let lib = "\
module Lib = struct
  val inline ctx \\mathstub m = read-inline ctx {}
  module Inner = struct
    val block ctx +greet = read-block ctx '< >
  end
  val block ctx +hello = read-block ctx '< +Inner.greet; >
end
";
    let doc = "\
let ctx = get-initial-context 400pt (command \\Lib.mathstub) in
read-block ctx '< +Lib.hello; >";
    assert_accepts(lib, doc);
}
