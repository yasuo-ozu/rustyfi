//! SATySFi 0.1 optional-argument rows, **increment 3a**: inline/block
//! command optional-argument PARAMETER bundles (`val inline ctx \cmd
//! ?(l = x, …) p = …`) and command **TYPE rows** (`inline [?(l:τ,…) τ_arg,
//! …]` / `block […]`) in signatures — end-to-end (parse V0_1 -> `v1::lower`
//! -> `elaborate` -> `typecheck` -> sealing/eval), plus the frozen 0.0.6
//! version gate and lowering placeholders.
//!
//! Mirrors `v01_sealing.rs`'s `run`/`assert_accepts`/`assert_type_error`
//! harness (real `satysfi_lang::compile_document_v1` pipeline, the
//! `NotADocument`-tolerant "type-checking accepted" bar — see that file's
//! own doc comment for why treating `Ok`/`NotADocument` alike is sound: type-
//! checking always runs, and fails first, before evaluation ever gets a
//! chance to produce a non-`Document` value). A handful of tests (T1/T2)
//! additionally prove the RUNTIME None-defaulting path: `compile_document_v1`
//! always calls `eval::Interp::eval` even when the result isn't a document
//! (`NotADocument` is only reached AFTER a successful eval), so
//! `assert_accepts` on a doc that actually INVOKES the command (rather than
//! a dummy `1`) is already end-to-end proof that an unbundled call defaults
//! every declared optional label to `None` without an `EvalError` — no
//! FontMetrics-based box-content introspection needed.
//!
//! No `math […]` command-type head, no `val math` parameter bundles, and no
//! command-APPLICATION `?(l=e){…}` bundles anywhere below — all three are
//! optional-arg-rows increment 3b (see the spec's §B/§13); the capstone
//! census found zero application-site bundles, so every command call below
//! is deliberately unbundled.

use satysfi_backend::{FontKey, FontMetrics, Length};
use satysfi_lang::CompileError;
use satysfi_loader::{LoadedCst, LoadedFile};
use satysfi_syntax::{parse_file, parse_file_v1};

/// Never actually exercised (every fixture below either fails type-checking
/// or fails at the `NotADocument` stage before glyph metrics matter, OR — for
/// T1/T2 — only needs `advance` to return *some* width so `read-inline`/
/// `read-block` don't choke on an unmeasurable glyph) — same stub shape as
/// `v01_sealing.rs`'s `Mono`.
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

/// Type-checking (AND, since `compile_document_v1` always evaluates before
/// checking document-hood, evaluation) accepted `doc_src` against `lib_src`.
fn assert_accepts(lib_src: &str, doc_src: &str) {
    match run(lib_src, doc_src) {
        Ok(()) | Err(CompileError::NotADocument(_)) => {}
        Err(other) => panic!("expected acceptance, got: {other}"),
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
// T1 — inline command param bundle: real invocation, unbundled -> `None`.
// ============================================================================

/// `\mathstub` supplies `get-initial-context`'s second argument (`[math-
/// text] inline-cmd`, a plain inline command needing no `math` package at
/// all — the "or a local stub command" case its own sig comment documents),
/// so every fixture below can synthesize a real `context` without pulling in
/// any bundled package.
const T1_LIB: &str = "\
module M = struct
val inline ctx \\mathstub m = read-inline ctx {}
val inline ctx \\emphwith ?(color = c) inner =
  let cv = match c with None -> 0 | Some v -> v end in
  read-inline ctx inner
end
";

#[test]
fn t1_inline_param_bundle_unbundled_call_defaults_and_evaluates() {
    let doc = "\
let ctx = get-initial-context 400pt (command \\M.mathstub) in
read-inline ctx {\\M.emphwith{hello}}";
    assert_accepts(T1_LIB, doc);
}

// ============================================================================
// T2 — block command, two labels (the std-ja `+section`/`+subsection`
// shape): real invocation, both optionals default, evaluates.
// ============================================================================

const T2_LIB: &str = "\
module M = struct
val inline ctx \\mathstub m = read-inline ctx {}
val block ctx +sec ?(label = l, outline-title = o) title inner =
  let lv = match l with None -> 0 | Some v -> v end in
  let ov = match o with None -> 0 | Some v -> v end in
  read-block ctx inner
end
";

#[test]
fn t2_block_command_two_labels_unbundled_call_defaults_and_evaluates() {
    let doc = "\
let ctx = get-initial-context 400pt (command \\M.mathstub) in
read-block ctx '< +M.sec{Title}< > >";
    assert_accepts(T2_LIB, doc);
}

// ============================================================================
// T3 — THE sealing gate: a sealed sig with a `?(l:τ,…)` command-type row
// matches its impl's `?(l=e,…)` parameter bundle, labels in NON-alphabetical
// surface order (`outline-title` before `label`) to prove sort-
// insensitivity (spec §14 risk 3).
// ============================================================================

const T3_LIB: &str = "\
module M :> sig
  val +sec : block [?(outline-title:string, label:string) inline-text, block-text]
end = struct
  val block ctx +sec ?(label = l, outline-title = o) title inner =
    read-block ctx inner
end
";

#[test]
fn t3_sealed_command_opt_labels_round_trip() {
    assert_accepts(T3_LIB, "1");
}

// ============================================================================
// T4 — seal label-set mismatch rejected (both directions: sig is a strict
// subset of the impl's bundle, and the mirror — sig has an extra label the
// impl never binds).
// ============================================================================

#[test]
fn t4_sealed_command_opt_label_set_mismatch_rejected() {
    let lib_sig_subset = "\
module M :> sig
  val +sec : block [?(label:string) inline-text, block-text]
end = struct
  val block ctx +sec ?(label = l, outline-title = o) title inner =
    read-block ctx inner
end
";
    let msg = assert_type_error(lib_sig_subset, "1");
    assert!(!msg.is_empty(), "expected a non-empty type-error message");

    let lib_sig_superset = "\
module M :> sig
  val +sec : block [?(label:string, outline-title:string) inline-text, block-text]
end = struct
  val block ctx +sec ?(label = l) title inner =
    read-block ctx inner
end
";
    let msg2 = assert_type_error(lib_sig_superset, "1");
    assert!(!msg2.is_empty(), "expected a non-empty type-error message");
}

// ============================================================================
// T5 — annot `\href` shape: compound label type (`length * color`), sealed.
// ============================================================================

const T5_LIB: &str = "\
module M :> sig
  val \\href : inline [?(border:length * color) string, inline-text]
end = struct
  val inline ctx \\href ?(border = b) uri inner =
    read-inline ctx inner
end
";

#[test]
fn t5_sealed_href_shape_compound_label_type() {
    assert_accepts(T5_LIB, "1");
}

// ============================================================================
// T6 — the frozen 0.0.6 version gate: a `?(l = x)` command-parameter bundle
// PARSES under 0.0.6 (the additive-`cst` accept surface — `Param::Bundled`
// reuses `CstOptBinders`, already 0.0.6-parseable since increment 1's
// `Expr::FunRows`), but elaboration rejects it with a version error rather
// than silently accepting it.
// ============================================================================

#[test]
fn t6_v006_command_param_bundle_version_gate() {
    let file = parse_file("let-inline ctx \\c ?(a = x) t = t in 0")
        .unwrap_or_else(|e| panic!("0.0.6 parse of the command bundle failed: {e}"));
    let env = satysfi_lang::primitives::base_env();
    let scope = satysfi_lang::elaborate::Scope::new(env.names());
    let err = satysfi_lang::elaborate::elaborate_program(&file, &scope)
        .expect_err("a 0.0.6 command binding with a `?(l=x)` bundle must be rejected");
    assert!(
        err.to_string().contains("SATySFi 0.1 syntax"),
        "expected a version-gate message, got: {err}"
    );
}

// ============================================================================
// T9 — lower placeholders.
// ============================================================================

/// An empty `?()` PARAMETER bundle on a command binding is a lower error
/// (`lower_opt_binders`'s existing empty-check, reused verbatim by
/// `lower_command_params`).
#[test]
fn t9_empty_param_bundle_on_command_is_lower_error() {
    let file = parse_file_v1(
        "module M = struct\n\
         val inline ctx \\c ?() t = t\n\
         end",
    )
    .unwrap_or_else(|e| panic!("lib parse failed: {e}"));
    let err = satysfi_lang::v1::lower::lower_file_v1(&file)
        .expect_err("an empty `?()` command-parameter bundle must be a lower error");
    assert!(
        err.to_string().contains("optional-parameter bundle") || err.to_string().contains("optional"),
        "got: {err}"
    );
}

/// An empty `?()` command-TYPE optional-label bundle is likewise a lower
/// error (`lower_type_cmd_args`'s new empty-check, §5.2) — surfaced through
/// the sig-sealing path (a sig's declared type is dropped at
/// `lower_file_v1` time and only ever lowered by `v1/module_check.rs`'s
/// `process_seal_member`, which wraps any `LowerError` it hits into a
/// `TypeError`, `module_check.rs:1280-1285` — so this reaches
/// `CompileError::Type`, not `CompileError::Lower`).
#[test]
fn t9_empty_type_row_bundle_is_lower_error() {
    let lib = "\
module M :> sig
  val \\c : inline [?() int]
end = struct
  val inline ctx \\c n = read-inline ctx {}
end
";
    let msg = assert_type_error(lib, "1");
    assert!(
        msg.contains("optional-label bundle") || msg.contains("optional"),
        "got: {msg}"
    );
}

/// `math […]` command TYPE heads — increment 3b's "still a parse error"
/// verdict is FLIPPED by math-package completion M1: `cst_v1::TypeApp::
/// MathCmdTy` is a `KwMath`-headed grammar arm reusing `TypeCmdArgItemV1`
/// exactly like `InlineCmdTy`/`BlockCmdTy` (inheriting its `?(l:τ,…)`
/// optional-label PREFIX for free), so the exact `val \derive : math
/// [?(name:math-text) list math-text, math-text]` row this test used to
/// pin as a parse error now PARSES. (Name kept for continuity with the
/// increment-3b history; the assertion is inverted.) Only the SIG grammar
/// is exercised here — pairing this labeled row with a matching bundled
/// IMPL is optional-arg-rows increment 3b's remaining M-opt slice
/// (`v1/lower.rs`'s `lower_value_math` still rejects `?(…)` bundles on
/// `val math` params; deferred, needed only by the `proof` package, zero
/// demand in `math.satyh`) — T-M1-seal below exercises the full seal path
/// with the BARE `math […]` rows `math.satyh` actually uses.
#[test]
fn t9_math_command_type_head_is_still_a_parse_error() {
    let src = "module M :> sig\n\
               val \\derive : math [?(name:math-text) list math-text, math-text]\n\
               end = struct val x = 1 end";
    assert!(
        parse_file_v1(src).is_ok(),
        "a `math [...]` command-type head (with a `?(...)` labeled row) must now parse"
    );
}

/// T-M1-seal: the sealing gate for a BARE `math […]` row (the shape
/// `math.satyh` actually uses — zero `?(` in any upstream `math […]` sig
/// row) — pins §2.2 (the `MathCmdTy` lowering arm), §2.4 (`CmdShape::
/// Inline` accepting `MonoType::MathCmd`), and `math_command_scheme_v01`'s
/// `MathCmd([])`/`MathCmd([mandatory, mandatory])` rows.
#[test]
fn t_m1_seal_bare_math_rows() {
    let lib = "\
module M :> sig
  val \\frac : math [math-text, math-text]
  val \\alpha : math []
end = struct
  val math ctx \\frac a b =
    let _ = read-math ctx a in
    let _ = read-math ctx b in
    math-char ctx MathOrd `x`
  val math ctx \\alpha = math-char ctx MathOrd `alpha`
end
";
    assert_accepts(lib, "1");
}

/// T-M1-roundtrip's typecheck-level twin: the same sig, checked against a
/// MISMATCHED arity impl (1 declared math-command argument, 2 actual params)
/// — `ArityMismatch`-shaped rejection (T-M1-mismatch(a)).
#[test]
fn t_m1_mismatch_arity() {
    let lib = "\
module M :> sig
  val \\alpha : math [math-text]
end = struct
  val math ctx \\alpha a b =
    let _ = a in
    let _ = b in
    read-math ctx a
end
";
    let msg = assert_type_error(lib, "1");
    assert!(!msg.is_empty(), "expected a non-empty type-error message");
}

/// T-M1-mismatch(b): a sig declaring `inline […]` for a binding that is
/// actually a `val math` — the seal shape guard now PASSES (both `\`-sigiled
/// shapes are accepted early), but subsumption/unify must still reject the
/// kind mismatch (`InlineCmd` vs `MathCmd`).
#[test]
fn t_m1_mismatch_inline_sig_for_math_impl() {
    let lib = "\
module M :> sig
  val \\alpha : inline [math-text]
end = struct
  val math ctx \\alpha m = read-math ctx m
end
";
    let msg = assert_type_error(lib, "1");
    assert!(!msg.is_empty(), "expected a non-empty type-error message");
}

/// T-M1-mismatch(c): the mirror — a sig declaring `math […]` for a binding
/// that is actually a `val inline`.
#[test]
fn t_m1_mismatch_math_sig_for_inline_impl() {
    let lib = "\
module M :> sig
  val \\greet : math [inline-text]
end = struct
  val inline ctx \\greet it = read-inline ctx it
end
";
    let msg = assert_type_error(lib, "1");
    assert!(!msg.is_empty(), "expected a non-empty type-error message");
}

/// T-M1-scripts: `val math ctx \lim with sub sup = …`'s synthesized
/// `with sub sup` trio must not surface as declared command-type slots —
/// sealed against a ZERO-arity `math []` row.
#[test]
fn t_m1_scripts_trio_not_surfaced_as_slots() {
    let lib = "\
module M :> sig
  val \\lim : math []
end = struct
  val math ctx \\lim with sub sup =
    let _ = sub in
    let _ = sup in
    math-char ctx MathOp `lim`
end
";
    assert_accepts(lib, "1");
}
